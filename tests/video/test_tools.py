"""Agent tools and the desktop share the engine project and its undo history."""

import json
from collections.abc import AsyncIterator
from itertools import pairwise
from pathlib import Path

import pytest

import columbina.tools
import columbina.video
from columbina.kernel import Context
from columbina.tools import TOOLS
from columbina.video import VIDEOS, Config, Videos, edits
from columbina.video.transcribe import Whisper
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio_tools import desk_of, result


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        yield desk, fake


async def test_edit_batch_and_human_undo_share_one_project(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    batch = [edits.add_timeline("Cut", edits.Rate(num=25, den=1), id="$film")]
    changed = result(await desk.call("video_edit", edits=batch))
    assert changed["names"]["$film"] == "t1"
    assert fake.asked.count("transaction.apply") == 1
    assert len(fake.steps) == 1
    assert not (desk.work / ".columbina/studio/timeline.json").exists()
    fake.change_by("desktop")
    result(await desk.call("video_undo"))
    assert len(fake.project["timelines"]) == 1
    result(await desk.call("video_undo"))
    assert fake.project["timelines"] == []
    result(await desk.call("video_undo", redo=True))
    assert fake.project["timelines"][0]["name"] == "Cut"


async def test_first_default_studio_call_migrates_once_and_returns_its_report(
    studio: tuple[Desk, Fake],
) -> None:
    from columbina.video.legacy import Manifest, Timeline

    desk, fake = studio
    old = desk.work / "Old Film"
    old.mkdir()
    (old / "project.json").write_text(Manifest().model_dump_json(), encoding="utf-8")
    (old / "timeline.json").write_text(Timeline().model_dump_json(), encoding="utf-8")
    info = result(await desk.call("video_info"))
    assert len(info["migrations"]) == 1
    assert info["timelines"][0]["name"] == "Old Film"
    assert len(fake.steps) == 1
    again = result(await desk.call("video_info"))
    assert "migrations" not in again
    assert len(fake.steps) == 1
    result(await desk.call("video_undo"))
    assert fake.project["timelines"] == []
    assert result(await desk.call("video_info"))["timelines"] == []


async def test_an_old_project_that_cannot_be_migrated_stands_in_the_way_of_no_call(
    studio: tuple[Desk, Fake], tmp_path: Path
) -> None:
    from columbina.video.legacy import Asset, Manifest, Timeline
    from columbina.video.project import Identity

    desk, fake = studio
    broken = desk.work / "Broken"
    broken.mkdir()
    (broken / "project.json").write_text(Manifest().model_dump_json(), encoding="utf-8")
    (broken / "timeline.json").write_text("{not json", encoding="utf-8")
    # A source outside the workspace is not read without asking, and the session may not.
    outside = desk.work / "Outside"
    outside.mkdir()
    elsewhere = tmp_path / "elsewhere.wav"
    elsewhere.write_bytes(b"sound")
    asset = Asset(
        id="talk",
        path=str(elsewhere),
        kind="audio",
        format="wav",
        duration_us=1_000_000,
        identity=Identity(size=5, modified_ns=0, digest="0" * 64),
    )
    (outside / "project.json").write_text(
        Manifest(assets=(asset,)).model_dump_json(), encoding="utf-8"
    )
    (outside / "timeline.json").write_text(Timeline().model_dump_json(), encoding="utf-8")
    good = desk.work / "Good"
    good.mkdir()
    (good / "project.json").write_text(Manifest().model_dump_json(), encoding="utf-8")
    (good / "timeline.json").write_text(Timeline().model_dump_json(), encoding="utf-8")

    first = result(await desk.call("video_info"))
    second = result(await desk.call("video_info"))
    edited = result(
        await desk.call(
            "video_edit", edits=[edits.add_timeline("Cut", edits.Rate(num=25, den=1), id="$f")]
        )
    )

    told = {Path(one["source"]).name: one for one in first["migrations"]}
    assert set(told) == {"Broken", "Good", "Outside"}
    assert "Broken" in told["Broken"]["error"]
    assert "OUTSIDE_WORKING_DIRECTORY" in told["Outside"]["error"]
    assert "error" not in told["Good"]
    assert [one["name"] for one in first["timelines"]] == ["Good"]
    assert "migrations" not in second
    assert edited["names"]["$f"]
    assert not (broken / "migrated.json").exists()
    assert (good / "migrated.json").exists()
    assert len(fake.steps) == 2


async def test_a_migration_that_takes_too_long_is_left_for_the_command(
    studio: tuple[Desk, Fake], monkeypatch: pytest.MonkeyPatch
) -> None:
    import asyncio

    import columbina.video.migrate
    from columbina.video.legacy import Manifest, Timeline

    desk, _fake = studio
    old = desk.work / "Old Film"
    old.mkdir()
    (old / "project.json").write_text(Manifest().model_dump_json(), encoding="utf-8")
    (old / "timeline.json").write_text(Timeline().model_dump_json(), encoding="utf-8")

    async def slow(*args: object, **kwargs: object) -> list[dict[str, object]]:
        await asyncio.sleep(30)
        return []

    monkeypatch.setattr(columbina.video.migrate, "migrate", slow)
    monkeypatch.setattr(columbina.video, "_MIGRATION_SECONDS", 0.2)

    told = result(await desk.call("video_info"))

    assert "columbina video migrate" in told["migrations"][0]["error"]
    assert told["timelines"] == []


async def test_import_bin_is_one_step_and_invalid_bin_has_no_partial_import(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    added = result(await desk.call("video_edit", edits=[edits.add_bin("Rushes", id="$rushes")]))
    bin = added["names"]["$rushes"]
    (desk.work / "take.wav").write_bytes(b"fake media")
    imported = result(await desk.call("video_import", paths=["take.wav"], bin=bin))
    assert imported["added"] == [fake.project["media"][0]["id"]]
    assert fake.project["media"][0]["bin"] == bin
    assert len(fake.steps) == 2
    result(await desk.call("video_undo"))
    assert fake.project["media"] == []
    refused = await desk.call("video_import", paths=["take.wav"], bin="missing")
    assert refused.error is not None
    assert refused.error.code == "VIDEO_MISSING"
    assert fake.project["media"] == []


async def test_info_needs_no_ffmpeg_and_names_all_timelines(
    studio: tuple[Desk, Fake],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    desk, _fake = studio
    monkeypatch.setenv("PATH", "")
    told = result(await desk.call("video_info"))
    assert told["media"] == []
    assert told["timelines"] == []
    assert "video_overlay" not in desk.tools.registry
    assert {"video_story", "video_edit"} <= set(desk.tools.registry)


@pytest.mark.parametrize("reason", ["locked", "ripple_blocked", "missing", "too_large"])
async def test_engine_refusals_preserve_the_reason(studio: tuple[Desk, Fake], reason: str) -> None:
    desk, fake = studio
    fake.refusals["transaction.apply"] = {
        "code": -32001,
        "message": "refused",
        "data": {"reason": reason, "index": 0},
    }
    failed = await desk.call(
        "video_edit", edits=[edits.add_timeline("Cut", edits.Rate(num=25, den=1))]
    )
    assert failed.error is not None
    assert failed.error.code == f"VIDEO_{reason.upper()}"
    assert fake.steps == []


async def test_paths_outside_workspace_are_checked_before_engine_mutation(
    studio: tuple[Desk, Fake],
    tmp_path: Path,
) -> None:
    desk, fake = studio
    path = tmp_path / "elsewhere.wav"
    path.write_bytes(b"sound")
    failed = await desk.call("video_import", paths=[str(path)])
    assert failed.error is not None
    assert failed.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert "media.import" not in fake.asked
    failed = await desk.call("video_render", path=str(tmp_path / "elsewhere.mp4"))
    assert failed.error is not None
    assert failed.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert "render.start" not in fake.asked


async def test_the_tools_are_given_only_where_the_config_says_so(ctx: Context) -> None:
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.video, {"enabled": False})
    assert not [name for name in ctx.use(TOOLS).registry if name.startswith("video_")]
    assert ctx.get(VIDEOS) is None


def test_a_model_of_faster_whisper_is_what_transcribes() -> None:
    told = Config.model_validate(
        {"enabled": "1", "whisper_model": "", "whisper_python": "", "engine": ""}
    )
    given = Videos(Config(whisper_model=Path("/models/small"), device="cpu"), None)
    assert (told.enabled, told.whisper_model, told.whisper_python, told.engine) == (
        True,
        None,
        None,
        None,
    )
    assert isinstance(given.transcriber, Whisper)
    assert (given.transcriber.model, given.transcriber.device) == (Path("/models/small"), "cpu")


async def seed_story(desk: Desk, fake: Fake) -> str:
    from columbina.video.transcribe import transcript_of
    from columbina.video.transcript import Word
    from columbina.video.words import Words, keep_words
    from support.studio import clip_of, timeline_of, track_of

    (desk.work / "talk.wav").write_bytes(b"fake audio")
    imported = result(await desk.call("video_import", paths=["talk.wav"]))
    media_id = imported["added"][0]
    fake.project["timelines"] = [
        timeline_of(
            track_of("A1", [clip_of("c20", 0, 50, media=media_id)]),
            edits.track("subtitle", id="S1"),
        )
    ]
    transcript = transcript_of(
        media_id,
        "0" * 64,
        [
            Word(start_us=0, end_us=400_000, text="Hello"),
            Word(start_us=400_000, end_us=800_000, text="filler"),
            Word(start_us=800_000, end_us=2_000_000, text="world."),
        ],
        [],
        2_000_000,
    )
    keep_words(fake.directory, Words.of(transcript))
    return media_id


async def test_story_file_edit_dry_apply_and_undo(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    await seed_story(desk, fake)
    told = result(await desk.call("video_story"))
    assert Path(told["path"]) == fake.directory / "story/t1.md"
    await desk.have_read(told["path"])
    changed = await desk.call("edit", path=told["path"], old="filler", new="~~filler~~")
    assert changed.error is None, changed.text
    steps = len(fake.steps)
    dry = result(await desk.call("video_apply", dry=True))
    assert dry["removed_frames"] == 10
    assert len(fake.steps) == steps
    applied = result(await desk.call("video_apply"))
    assert applied["removed_frames"] == 10
    assert len(fake.steps) == steps + 1
    assert fake.edits[-1] == edits.ripple_delete_range("t1", 10, 20)
    assert "filler" not in Path(told["path"]).read_text(encoding="utf-8")
    result(await desk.call("video_undo"))
    assert fake.project["timelines"][0]["tracks"][0]["clips"][0]["length"] == 50


async def test_cached_transcript_subtitles_and_correction(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    media_id = await seed_story(desk, fake)
    told = result(await desk.call("video_transcribe", media=media_id, subtitles="S1"))
    assert told["reused"] is True
    assert fake.edits[-1] == {
        "edit": "cues_from",
        "timeline": "t1",
        "track": "S1",
        "items": [{"start": 0, "length": 50, "text": "Hello filler world."}],
    }
    steps = len(fake.steps)
    changed = result(
        await desk.call("video_correct", media=media_id, sentence=0, wrong="world", right="WORLD")
    )
    assert changed["shown"] == {"2": "WORLD"}
    assert len(fake.steps) == steps
    await desk.call("video_story")
    text = (fake.directory / "story/t1.md").read_text(encoding="utf-8")
    assert "Hello filler WORLD" in text


async def test_subtitles_tell_every_transcribed_media_of_the_dialogue_and_never_overlap(
    studio: tuple[Desk, Fake],
) -> None:
    from columbina.video.transcribe import transcript_of
    from columbina.video.transcript import Word
    from columbina.video.words import Words, keep_words
    from support.studio import clip_of, timeline_of, track_of

    desk, fake = studio
    for name in ("one.wav", "two.wav"):
        (desk.work / name).write_bytes(name.encode())
    first, second = result(await desk.call("video_import", paths=["one.wav", "two.wav"]))["added"]
    fake.project["timelines"] = [
        timeline_of(
            track_of(
                "A1",
                [
                    clip_of("c1", 0, 50, media=first),
                    # The second media plays backward, from 1 s of it.
                    clip_of("c2", 50, 25, media=second, ticks=12800, reverse=True),
                ],
            ),
            edits.track("subtitle", id="S1"),
        )
    ]
    # The fake engine gives every file one digest; the analysis of each is kept by its own.
    fake.project["media"][1]["identity"]["digest"] = "1" * 64
    digests = {item["id"]: item["identity"]["digest"] for item in fake.project["media"]}
    # A transcriber times words that overlap: each long word here begins before the last ends.
    overlapping = [
        Word(start_us=at * 300_000, end_us=at * 300_000 + 360_000, text="abcdefghijkl")
        for at in range(6)
    ]
    keep_words(
        fake.directory,
        Words.of(transcript_of(first, digests[first], overlapping, [], 2_000_000)),
    )
    keep_words(
        fake.directory,
        Words.of(
            transcript_of(
                second,
                digests[second],
                [Word(start_us=1_200_000, end_us=1_800_000, text="Other.")],
                [],
                3_000_000,
            )
        ),
    )

    told = result(await desk.call("video_transcribe", media=first, subtitles="S1"))

    items = fake.edits[-1]["items"]
    assert told["cues"] == len(items)
    assert [one["text"] for one in items] == [
        "abcdefghijkl abcdefghijkl abcdefghijkl",
        "abcdefghijkl abcdefghijkl abcdefghijkl",
        "Other.",
    ]
    assert all(one["length"] > 0 for one in items)
    assert all(a["start"] + a["length"] <= b["start"] for a, b in pairwise(items))
    # The first cue ends where the second begins, though its last word ends 60 ms later.
    assert (items[0]["start"], items[0]["start"] + items[0]["length"]) == (0, 23)
    # "Other." is said 1.2 s to 1.8 s into the second media, of which c2 plays 1 s to 2 s
    # backward, from frame 50: frames 55 to 70.
    assert (items[2]["start"], items[2]["length"]) == (55, 15)


async def test_default_render_path_progress_and_no_history_step(studio: tuple[Desk, Fake]) -> None:
    from columbina.video import VIDEO_PROGRESS, RenderProgress

    desk, fake = studio
    await seed_story(desk, fake)
    progress: list[RenderProgress] = []
    desk.ctx.on(VIDEO_PROGRESS, progress.append)
    steps = len(fake.steps)
    rendered = result(await desk.call("video_render", codec="prores", quality="small"))
    assert Path(rendered["path"]) == desk.work / "out/Cut.mov"
    assert fake.rendered[-1]["quality"] == "small"
    assert fake.rendered[-1]["subtitles"] is True
    assert progress[-1].progress.state == "done"
    assert progress[0].progress.state == "running"
    assert len(fake.steps) == steps


async def test_ambiguous_timeline_requires_a_choice_and_batch_reports_clamping(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.add_timeline("A", edits.Rate(num=25, den=1)),
                edits.add_timeline("B", edits.Rate(num=25, den=1)),
            ],
        )
    )
    failed = await desk.call("video_story")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_AMBIGUOUS"
    fake.clamped = [0]
    told = result(await desk.call("video_edit", timeline="t1", edits=[{"edit": "blade", "at": 5}]))
    assert told["clamped"] == [0]
    assert fake.edits[-1] == {"edit": "blade", "at": 5, "timeline": "t1"}


async def test_explicit_project_and_missing_engine(ctx: Context, tmp_path: Path) -> None:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / "Another") as fake:
        told = result(await desk.call("video_info", project="Another"))
        assert Path(told["project"]) == fake.directory
    desk.ctx.use(VIDEOS)._program = str(tmp_path / "missing-engine.exe")  # pyright: ignore[reportPrivateUsage]
    failed = await desk.call("video_info", project="missing")
    assert failed.error is not None
    assert failed.error.code == "NO_ENGINE"


async def test_real_engine_tool_workflow(ctx: Context, tmp_path: Path) -> None:
    from columbina.video.media import located, probed
    from columbina.video.transcribe import Scripted
    from support.media import made, real, real_engine

    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    source = made(
        desk.work,
        "talk.mp4",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=160x120:r=25:d=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=4",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
    )
    transcript_path = desk.work / "words.json"
    transcript_path.write_text(
        json.dumps(
            [
                {"start_us": 400_000, "end_us": 800_000, "text": "Hello"},
                {"start_us": 800_000, "end_us": 1_600_000, "text": "world."},
            ]
        ),
        encoding="utf-8",
    )
    desk.ctx.use(VIDEOS).transcriber = Scripted(transcript_path)
    media_id = result(await desk.call("video_import", paths=[str(source)]))["added"][0]
    info = result(await desk.call("video_info"))
    scale = info["media"][0]["facts"]["duration"]["scale"]
    source_range = edits.of_media(
        media_id, start=edits.Time(ticks=scale // 5, scale=scale), length=40
    )
    batch = [
        edits.add_timeline(
            "Cut",
            edits.Rate(num=25, den=1),
            id="$film",
            width=160,
            height=120,
            tracks=[
                edits.track("video", id="$v"),
                edits.track("audio", id="$a"),
                edits.track("subtitle", id="$s"),
            ],
        ),
        edits.insert("$film", source_range, 0),
        edits.insert("$film", source_range, 40),
    ]
    names = result(await desk.call("video_edit", edits=batch))["names"]
    timeline = names["$film"]
    heard = result(
        await desk.call(
            "video_transcribe", media=media_id, subtitles=names["$s"], timeline=timeline
        )
    )
    assert heard["words"] == 2
    assert heard["cues"] == 2
    info = result(await desk.call("video_info", timeline=timeline))
    assert len(info["timelines"][0]["tracks"][2]["cues"]) == 2
    result(await desk.call("video_edit", timeline=timeline, edits=[{"edit": "blade", "at": 20}]))
    result(await desk.call("video_edit", edits=[edits.ripple_delete_range(timeline, 20, 30)]))
    info = result(await desk.call("video_info", timeline=timeline))
    clips = info["timelines"][0]["tracks"][0]["clips"]
    result(
        await desk.call(
            "video_edit",
            edits=[
                edits.add_transition(timeline, clips[-2]["id"], clips[-1]["id"], 4, kind="dissolve")
            ],
        )
    )
    rendered = result(await desk.call("video_render", timeline=timeline, codec="prores"))
    assert Path(rendered["path"]).exists()
    assert (desk.work / "out/Cut.srt").exists()
    tools = await located(str(Path(real()).parent), [], cwd=tmp_path)
    facts = await probed(tools, Path(rendered["path"]), place=tmp_path)
    assert abs(facts.duration_us - 2_800_000) <= 45_000
    assert facts.video is not None
    assert (facts.video.width, facts.video.height) == (160, 120)
    assert facts.audio is not None
    assert not (desk.work / ".columbina/studio/timeline.json").exists()


async def test_concurrent_desktop_change_refuses_precomputed_text_cut(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    await seed_story(desk, fake)
    told = result(await desk.call("video_story"))
    path = Path(told["path"])
    path.write_text(
        path.read_text(encoding="utf-8").replace("filler", "~~filler~~"), encoding="utf-8"
    )
    steps = len(fake.steps)
    fake.then["project.get"] = lambda: fake.change_by("desktop")
    failed = await desk.call("video_apply", timeline="t1")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_STALE_REVISION"
    assert len(fake.steps) == steps + 1  # Only the person's change.
    assert "~~filler~~" in path.read_text(encoding="utf-8")
    assert fake.project["timelines"][0]["tracks"][0]["clips"][0]["length"] == 50


async def test_info_does_not_create_a_project_in_read_only_mode(
    ctx: Context, tmp_path: Path
) -> None:
    desk = await desk_of(ctx, tmp_path)
    desk.sandbox.set_mode(desk.session, "read-only")
    failed = await desk.call("video_info")
    assert failed.error is not None
    assert failed.error.code == "READ_ONLY"
    assert not (desk.work / ".columbina/studio").exists()


async def test_committed_cut_is_reported_even_if_story_refresh_loses_its_track(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    await seed_story(desk, fake)
    told = result(await desk.call("video_story"))
    path = Path(told["path"])
    path.write_text(
        path.read_text(encoding="utf-8").replace("filler", "~~filler~~"), encoding="utf-8"
    )
    fake.then["transaction.apply"] = lambda: fake.project["timelines"].clear()
    changed = result(await desk.call("video_apply"))
    assert changed["removed_frames"] == 10
    assert "story_refresh_error" in changed
    assert changed["revision"] == 2


async def test_render_failure_and_cancellation_do_not_change_timeline(
    studio: tuple[Desk, Fake],
) -> None:
    import asyncio

    from columbina.tools import ToolRun

    desk, fake = studio
    await seed_story(desk, fake)
    fake.render_end = "failed"
    failed = await desk.call("video_render")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_FAILED"
    fake.render_hold = True
    ready = asyncio.Event()
    fake.then["render.start"] = ready.set
    editor = await desk.ctx.use(VIDEOS).editor(
        ToolRun("cancel", desk.work, desk.session), "video_render"
    )
    pending = asyncio.create_task(
        editor.rendered(
            "t1",
            desk.work / "out/stop.mp4",
            codec="h264",
            quality="small",
            subtitles=False,
            replace=False,
        )
    )
    await asyncio.wait_for(ready.wait(), 5)
    pending.cancel()
    with pytest.raises(asyncio.CancelledError):
        await pending
    assert "render.cancel" in fake.asked
    assert "cancelled" in fake.jobs.values()
    assert len(fake.steps) == 1


async def test_a_long_talk_is_told_and_cut_within_the_time_of_a_call(
    studio: tuple[Desk, Fake],
) -> None:
    import time

    from columbina.video.transcribe import transcript_of
    from columbina.video.transcript import Word
    from columbina.video.words import Words, keep_words
    from support.studio import clip_of, timeline_of, track_of

    desk, fake = studio
    (desk.work / "talk.wav").write_bytes(b"fake audio")
    media_id = result(await desk.call("video_import", paths=["talk.wav"]))["added"][0]
    each = 8000 * 300_000 * 25 // 1_000_000 // 200
    fake.project["timelines"] = [
        timeline_of(
            track_of(
                "A1",
                [
                    clip_of(f"c{at}", at * each, each - 1, ticks=at * each * 512, media=media_id)
                    for at in range(200)
                ],
            )
        )
    ]
    said = [
        Word(
            start_us=at * 300_000 + 10_000,
            end_us=at * 300_000 + 250_000,
            text=f"w{at}{'.' if at % 12 == 11 else ''}",
        )
        for at in range(8000)
    ]
    keep_words(
        fake.directory,
        Words.of(transcript_of(media_id, "0" * 64, said, [], 8000 * 300_000 + 1_000_000)),
    )

    began = time.perf_counter()
    path = Path(result(await desk.call("video_story"))["path"])
    text = path.read_text(encoding="utf-8")
    for at in range(5, 8000, 400):
        text = text.replace(f" w{at} ", f" ~~w{at}~~ ", 1)
    path.write_text(text, encoding="utf-8")
    applied = result(await desk.call("video_apply"))
    took = time.perf_counter() - began

    assert len(applied["ranges"]) == 20
    assert "story_refresh_error" not in applied
    # A second or so; a call has 30 seconds.
    assert took < 15, took


async def test_applying_before_a_story_is_written_says_to_write_it_first(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    await seed_story(desk, fake)
    failed = await desk.call("video_apply")
    assert failed.error is not None
    assert failed.error.code == "VIDEO_STORY"
    assert "video_story" in failed.text
    assert "Errno" not in failed.text


async def test_the_engine_takes_subtitles_of_words_timed_to_overlap(
    ctx: Context, tmp_path: Path
) -> None:
    from columbina.video.transcribe import Scripted
    from support.media import made, real_engine

    desk = await desk_of(ctx, tmp_path, engine=real_engine())
    source = made(
        desk.work,
        "talk.wav",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=3",
        "-c:a",
        "pcm_s16le",
    )
    words = desk.work / "words.json"
    words.write_text(
        json.dumps(
            [
                {"start_us": at * 300_000, "end_us": at * 300_000 + 360_000, "text": "abcdefghijkl"}
                for at in range(6)
            ]
        ),
        encoding="utf-8",
    )
    desk.ctx.use(VIDEOS).transcriber = Scripted(words)
    media_id = result(await desk.call("video_import", paths=[str(source)]))["added"][0]
    names = result(
        await desk.call(
            "video_edit",
            edits=[
                edits.add_timeline(
                    "Cut",
                    edits.Rate(num=25, den=1),
                    id="$film",
                    tracks=[edits.track("audio", id="$a"), edits.track("subtitle", id="$s")],
                ),
                edits.insert("$film", edits.of_media(media_id, length=60), 0),
            ],
        )
    )["names"]

    heard = result(await desk.call("video_transcribe", media=media_id, subtitles=names["$s"]))

    assert heard["cues"] == 2
    info = result(await desk.call("video_info", timeline=names["$film"]))
    cues = info["timelines"][0]["tracks"][1]["cues"]
    assert [(one["start"], one["length"]) for one in cues] == [(0, 23), (23, 24)]
    assert not list((desk.work / ".columbina/studio/analysis").glob("audio-*.wav"))
