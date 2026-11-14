"""Legacy migration preserves old files and commits media plus the timeline once."""

import asyncio
import hashlib
import json
import shutil
import subprocess
from pathlib import Path
from typing import Any

import pytest

from columbina.video import edits
from columbina.video.engine import Engine, EngineError
from columbina.video.media import Tools, located
from columbina.video.migrate import discover, migrate
from columbina.video.project import ProjectError, written
from support.legacy import installed
from support.media import made, real, real_engine


@pytest.fixture
async def old_project(tmp_path: Path) -> tuple[Path, Tools]:
    workspace = tmp_path / "workspace"
    raw = workspace / "raw"
    words = [
        (200_000, 800_000, "one"),
        (900_000, 1_400_000, "two."),
        (3_200_000, 3_800_000, "three"),
        (3_900_000, 4_400_000, "four."),
    ]
    voiced = "+".join(f"between(t,{start / 1e6},{end / 1e6})" for start, end, _ in words)
    made(
        raw,
        "talk.mp4",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=160x120:r=25:d=5",
        "-f",
        "lavfi",
        "-i",
        f"aevalsrc='0.4*sin(440*2*PI*t)*({voiced})':d=5:s=48000",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
    )
    made(raw, "yellow.png", "-f", "lavfi", "-i", "color=c=yellow:s=64x48:d=1", "-frames:v", "1")
    made(raw, "song.wav", "-f", "lavfi", "-i", "sine=f=220:d=10")
    (raw / "words.json").write_text(
        json.dumps(
            [{"start_us": start, "end_us": end, "text": text} for start, end, text in words]
        ),
        encoding="utf-8",
    )
    tools = await located(str(Path(real()).parent), [], cwd=tmp_path)
    installed(workspace, "Old Film")
    return workspace, tools


def preserved(directory: Path) -> dict[str, str]:
    # Hashes here verify the explicit migration requirement: every old file stays byte-identical.
    return {
        str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in directory.rglob("*")
        if path.is_file() and path.name != "migrated.json"
    }


def test_discover_obeys_depth_and_excludes_internal_directories(tmp_path: Path) -> None:
    def old(path: Path) -> None:
        path.mkdir(parents=True)
        (path / "project.json").write_text(
            '{"version":1,"canvas":null,"assets":[]}', encoding="utf-8"
        )
        (path / "timeline.json").write_text('{"version":1}', encoding="utf-8")

    for path in [
        tmp_path / "one",
        tmp_path / "a/b/three",
        tmp_path / "a/b/c/four",
        tmp_path / ".git/film",
        tmp_path / "node_modules/film",
        tmp_path / ".columbina/film",
        tmp_path / "already",
    ]:
        old(path)
    (tmp_path / "already/migrated.json").write_text("{}", encoding="utf-8")
    assert discover(tmp_path) == [tmp_path / "a/b/three", tmp_path / "one"]


async def test_migration_preserves_sources_maps_layers_and_undoes_once(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    old = workspace / "Old Film"
    before = preserved(old)
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        reports = await migrate(workspace, engine, tools=tools)
        assert len(reports) == 1
        report = reports[0]
        marker: dict[str, Any] = json.loads((old / "migrated.json").read_text(encoding="utf-8"))
        assert marker["timeline"] == report["timeline"]
        assert set(marker["media"]) == {"talk", "yellow", "song"}
        assert any("duck" in item for item in marker["dropped"])
        _, project = await engine.project()
        assert len(project["media"]) == 3
        timeline = project["timelines"][0]
        assert timeline["name"] == "Old Film"
        video = next(t for t in timeline["tracks"] if t["kind"] == "video")
        audio = next(t for t in timeline["tracks"] if t["kind"] == "audio")
        assert len(video["clips"]) == len(audio["clips"]) == 2
        assert video["clips"][0]["link"] == audio["clips"][0]["link"]
        assert video["clips"][0]["link"] is not None
        assert video["transitions"][0]["length"] == 5
        assert audio["transitions"][0]["kind"] == "crossfade"
        picture = next(
            c
            for t in timeline["tracks"]
            for c in t.get("clips", [])
            if c["source"].get("media") == marker["media"]["yellow"]
        )
        assert (picture["start"], picture["length"]) == (10, 30)
        assert picture["transform"]["scale"] == pytest.approx(0.25)
        assert picture["transform"]["opacity"] == pytest.approx(0.6)
        assert picture["transform"]["x"] > 0
        assert picture["transform"]["y"] < 0
        assert (picture["fade_in"], picture["fade_out"]) == (3, 3)
        music = next(
            c
            for t in timeline["tracks"]
            for c in t.get("clips", [])
            if c["source"].get("media") == marker["media"]["song"]
        )
        assert music["gain_db"] == pytest.approx(-6.020599913)
        assert music["speed"] == {"num": 5, "den": 4}
        assert any(
            c["source"].get("text") == "Welcome"
            for t in timeline["tracks"]
            for c in t.get("clips", [])
        )
        assert len(next(t for t in timeline["tracks"] if t["kind"] == "subtitle")["cues"]) == 2
        assert (engine.directory / "story" / f"{report['timeline']}.md").exists()
        assert list((engine.directory / "analysis").glob("words-*.json"))
        assert list((engine.directory / "analysis").glob("silences-*-40-200.json"))
        revision = engine.revision
        assert await migrate(workspace, engine, tools=tools) == []
        assert engine.revision == revision
        await engine.undo()
        _, undone = await engine.project()
        assert undone["media"] == []
        assert undone["timelines"] == []
        assert await engine.undo() is None
    assert preserved(old) == before


async def test_migration_reuses_existing_media_and_undo_keeps_it(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        _, ids = await engine.imported([workspace / "raw/talk.mp4"])
        report = (await migrate(workspace, engine, tools=tools))[0]
        assert report["media"]["talk"] == ids[0]
        await engine.undo()
        _, project = await engine.project()
        assert [m["id"] for m in project["media"]] == ids
        assert project["timelines"] == []


async def test_migration_refreshes_pre_studio_still_metadata_without_duplicate_media(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        old_image = (await engine.inspected([workspace / "raw/yellow.png"]))[0]
        old_image["facts"]["video"].pop("still")
        imported = await engine.apply([{"op": "add_media", "media": old_image}])
        image_id = imported.done[0]["media"]["id"]
        report = (await migrate(workspace, engine, tools=tools))[0]
        assert report["media"]["yellow"] == image_id
        _, project = await engine.project()
        image = next(m for m in project["media"] if m["id"] == image_id)
        assert image["facts"]["video"]["still"] is True
        picture = next(
            c
            for t in project["timelines"][0]["tracks"]
            for c in t.get("clips", [])
            if c["source"].get("media") == image_id
        )
        assert picture["length"] == 30
        with pytest.raises(EngineError) as refused:
            await engine.apply([{"op": "set_media", "id": image_id, "still": False}])
        assert refused.value.data is not None
        assert refused.value.data["reason"] == "out_of_source"
        await engine.undo()
        _, project = await engine.project()
        assert len(project["media"]) == 1
        assert project["media"][0]["facts"]["video"].get("still", False) is False
        assert project["timelines"] == []


async def test_a_one_frame_movie_is_not_mistaken_for_a_still_image(tmp_path: Path) -> None:
    path = made(
        tmp_path,
        "one.mp4",
        "-f",
        "lavfi",
        "-i",
        "color=c=yellow:s=64x48:r=25:d=1",
        "-frames:v",
        "1",
        "-c:v",
        "libx264",
    )
    async with await Engine.attach(tmp_path / "studio", program=real_engine()) as engine:
        items = await engine.inspected([path])
        assert items[0]["facts"]["video"].get("still", False) is False
        _, ids = await engine.imported([path])
        with pytest.raises(EngineError) as refused:
            await engine.apply(
                [
                    edits.add_timeline(
                        "Movie",
                        edits.Rate(num=25, den=1),
                        id="$film",
                        width=64,
                        height=48,
                        tracks=[edits.track("video")],
                    ),
                    edits.insert("$film", edits.of_media(ids[0], length=50), 0),
                ]
            )
        assert refused.value.data is not None
        assert refused.value.data["reason"] == "no_handles"
        assert (await engine.project())[1]["timelines"] == []


async def test_a_changed_source_refuses_migration_without_a_partial_project(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    with (workspace / "raw/talk.mp4").open("ab") as source:
        source.write(b"changed source")
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        (report,) = await migrate(workspace, engine, tools=tools)
        assert "changed since import" in report["error"]
        assert report["source"] == str(workspace / "Old Film")
        _, project = await engine.project()
        assert project["media"] == project["timelines"] == []
        assert await engine.undo() is None
        assert not list((engine.directory / "analysis/migrations").glob("pending-*.json"))
    assert not (workspace / "Old Film/migrated.json").exists()


async def test_a_project_that_cannot_be_migrated_is_reported_and_the_others_are_migrated(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    broken = workspace / "Broken"
    broken.mkdir()
    (broken / "project.json").write_text(
        '{"version":1,"canvas":null,"assets":[]}', encoding="utf-8"
    )
    (broken / "timeline.json").write_text("{not json", encoding="utf-8")
    missing = workspace / "Missing"
    shutil.copytree(workspace / "Old Film", missing)
    (missing / "transcripts/talk.json").unlink()
    told: list[dict[str, Any]] = []
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        reports = await migrate(workspace, engine, tools=tools, told=told.append)
        again = await migrate(workspace, engine, tools=tools)
        _, project = await engine.project()

    assert told == reports
    by = {Path(one["source"]).name: one for one in reports}
    assert set(by) == {"Broken", "Missing", "Old Film"}
    assert "Broken" in by["Broken"]["error"]
    assert "no transcript for talk" in by["Missing"]["error"]
    assert by["Old Film"]["timeline"] == project["timelines"][0]["id"]
    assert len(project["timelines"]) == 1
    # Those that could not be migrated are tried again; the one that was is not.
    assert sorted(Path(one["source"]).name for one in again) == ["Broken", "Missing"]


async def test_a_migration_given_up_after_the_engine_took_it_is_not_made_twice(
    old_project: tuple[Path, Tools],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace, tools = old_project
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        applying = engine.apply

        async def taken_then_given_up(*args: Any, **kwargs: Any) -> Any:
            await applying(*args, **kwargs)
            raise asyncio.CancelledError

        with monkeypatch.context() as patch:
            patch.setattr(engine, "apply", taken_then_given_up)
            with pytest.raises(asyncio.CancelledError):
                await migrate(workspace, engine, tools=tools)
        assert not (workspace / "Old Film/migrated.json").exists()
        revision, project = await engine.project()
        (report,) = await migrate(workspace, engine, tools=tools)
        marker = json.loads((workspace / "Old Film/migrated.json").read_text(encoding="utf-8"))

        assert (await engine.project())[0] == revision
        assert report["timeline"] == marker["timeline"] == project["timelines"][0]["id"]
        assert set(marker["media"]) == {"talk", "yellow", "song"}
        assert set(marker["media"].values()) == {item["id"] for item in project["media"]}
        assert any("given up" in warning for warning in report["warnings"])
        assert len(project["timelines"]) == 1
        assert not list((engine.directory / "analysis/migrations").glob("*.json"))
        assert await migrate(workspace, engine, tools=tools) == []


async def test_a_migration_that_was_undone_is_made_again_and_says_so(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        (first,) = await migrate(workspace, engine, tools=tools)
        await engine.undo()
        (second,) = await migrate(workspace, engine, tools=tools)
        _, project = await engine.project()
        marker = json.loads((workspace / "Old Film/migrated.json").read_text(encoding="utf-8"))

        assert [timeline["id"] for timeline in project["timelines"]] == [second["timeline"]]
        assert marker["timeline"] == second["timeline"]
        assert any(
            f"timeline {first['timeline']}" in warning and "no longer" in warning
            for warning in second["warnings"]
        )
        assert await migrate(workspace, engine, tools=tools) == []


async def test_old_relative_sources_are_found_beside_the_old_project_wherever_this_runs(
    old_project: tuple[Path, Tools],
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    workspace, tools = old_project
    manifest = workspace / "Old Film/project.json"
    value = json.loads(manifest.read_text(encoding="utf-8"))
    for asset in value["assets"]:
        asset["path"] = "../raw/" + Path(asset["path"]).name
    manifest.write_text(json.dumps(value), encoding="utf-8")
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    monkeypatch.chdir(elsewhere)
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        (report,) = await migrate(workspace, engine, tools=tools)
        _, project = await engine.project()

    assert "error" not in report
    assert {Path(item["path"]) for item in project["media"]} == {
        (workspace / "raw" / name).resolve() for name in ("talk.mp4", "yellow.png", "song.wav")
    }


async def test_silences_that_cannot_be_found_in_time_leave_the_cuts_at_the_words(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    for cached in (workspace / "Old Film/cache").glob("talk-*-silences-*.json"):
        cached.unlink()
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        (report,) = await migrate(workspace, engine, tools=tools, analysis_seconds=0.001)

    assert report["timeline"]
    assert any(item.startswith("talk: silences not found") for item in report["dropped"]), report[
        "dropped"
    ]


def test_a_directory_that_cannot_be_read_is_passed_over(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("closed/film", "open/film"):
        (tmp_path / name).mkdir(parents=True)
        (tmp_path / name / "project.json").write_text(
            '{"version":1,"canvas":null,"assets":[]}', encoding="utf-8"
        )
        (tmp_path / name / "timeline.json").write_text('{"version":1}', encoding="utf-8")
    listed = Path.iterdir

    def iterdir(self: Path) -> Any:
        if self.name == "closed":
            raise PermissionError(13, "Access is denied", str(self))
        return listed(self)

    monkeypatch.setattr(Path, "iterdir", iterdir)
    assert discover(tmp_path) == [tmp_path / "open/film"]


async def test_short_clips_shorten_legacy_transitions_to_the_longest_the_engine_allows(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    path = workspace / "Old Film/timeline.json"
    old = json.loads(path.read_text(encoding="utf-8"))
    old["main"].append(old["main"][0])
    old["crossfades"] = {"0": 1000, "1": 1000}
    old["captions"] = False
    path.write_text(json.dumps(old), encoding="utf-8")
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        (report,) = await migrate(workspace, engine, tools=tools)
        _, project = await engine.project()

    shortened = [
        int(item.rsplit(" shortened to ", 1)[1].split()[0])
        for item in report["dropped"]
        if "available handles" in item
    ]
    lengths = sorted(
        transition["length"]
        for track in project["timelines"][0]["tracks"]
        for transition in track.get("transitions", [])
    )
    assert shortened
    assert all(one > 0 for one in shortened)
    # A transition that can be shorter is kept as long as the engine allows, not taken away.
    assert all(one in lengths for one in shortened)


async def test_concurrent_migration_clients_only_create_one_timeline(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    async with (
        await Engine.attach(workspace / ".columbina/studio", program=real_engine()) as first,
        await Engine.attach(workspace / ".columbina/studio", program=real_engine()) as second,
    ):
        reports = await asyncio.gather(
            migrate(workspace, first, tools=tools), migrate(workspace, second, tools=tools)
        )
        assert sum(len(items) for items in reports) == 1
        assert len((await first.project())[1]["timelines"]) == 1
        await first.undo()
        assert await first.undo() is None


async def test_a_failed_marker_is_reported_and_repaired_without_duplicate_edits(
    old_project: tuple[Path, Tools],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from columbina.video import migrate as migration

    workspace, tools = old_project
    original = written

    def fail_marker(path: Path, text: str) -> None:
        if path.name == "migrated.json":
            raise ProjectError("legacy directory is read only")
        original(path, text)

    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        with monkeypatch.context() as patch:
            patch.setattr(migration, "written", fail_marker)
            result = (await migrate(workspace, engine, tools=tools))[0]
        assert any("marker" in warning for warning in result["warnings"])
        revision, _ = await engine.project()
        repaired = (await migrate(workspace, engine, tools=tools))[0]
        assert repaired["timeline"] == result["timeline"]
        assert (await engine.project())[0] == revision
        assert await migrate(workspace, engine, tools=tools) == []
        assert not list((engine.directory / "analysis/migrations").glob("*.json"))


async def test_cli_migration_reports_json_and_then_has_nothing_to_do(
    old_project: tuple[Path, Tools],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    from columbina.apps.launcher import main

    workspace, _ = old_project
    monkeypatch.chdir(workspace)
    monkeypatch.setenv("COLUMBINA_HOME", str(workspace / "home"))
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(workspace / "agents"))
    monkeypatch.setenv("COLUMBINA_ENGINE", real_engine())
    args = ["video", "migrate", "--workspace", str(workspace)]
    assert await asyncio.to_thread(main, [*args, "--json"]) == 0
    said = json.loads(capsys.readouterr().out)
    assert len(said["migrations"]) == 1
    assert said["migrations"][0]["source"] == str(workspace / "Old Film")
    assert await asyncio.to_thread(main, args) == 0
    assert "duck:" in capsys.readouterr().out
    assert (
        await asyncio.to_thread(main, [*args, "--workspace", str(workspace / "missing"), "--json"])
        == 2
    )
    assert "workspace does not exist" in json.loads(capsys.readouterr().out)["error"]


async def test_audio_only_legacy_frame_positions_keep_their_millisecond_timebase(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    made(
        workspace / "raw",
        "voice.wav",
        "-i",
        str(workspace / "raw/talk.mp4"),
        "-vn",
        "-c:a",
        "pcm_s16le",
    )
    installed(workspace, "Audio")
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        reports = await migrate(workspace, engine, tools=tools)
        id = next(
            report["timeline"] for report in reports if Path(report["source"]).name == "Audio"
        )
        _, project = await engine.project()
        timeline = next(t for t in project["timelines"] if t["id"] == id)
        voice, music = [t for t in timeline["tracks"] if t["kind"] == "audio"]
        assert voice["transitions"][0]["length"] == 5
        assert (music["clips"][0]["start"], music["clips"][0]["length"]) == (12, 25)


async def test_short_clips_shorten_legacy_transitions_and_report_the_change(
    old_project: tuple[Path, Tools],
) -> None:
    workspace, tools = old_project
    path = workspace / "Old Film/timeline.json"
    old = json.loads(path.read_text(encoding="utf-8"))
    old["main"].append(old["main"][0])
    old["crossfades"] = {"0": 1000, "1": 1000}
    old["captions"] = False
    path.write_text(json.dumps(old), encoding="utf-8")
    before = preserved(path.parent)
    async with await Engine.attach(
        workspace / ".columbina/studio", program=real_engine()
    ) as engine:
        reports = await migrate(workspace, engine, tools=tools)
        assert any("available handles" in item for item in reports[0]["dropped"])
        _, project = await engine.project()
        assert len(project["timelines"]) == 1
        await engine.undo()
        assert await engine.undo() is None
    assert preserved(path.parent) == before


@pytest.mark.parametrize("extension", ["png", "jpg", "webp"])
async def test_still_images_hold_their_picture_throughout_an_export(
    tmp_path: Path,
    extension: str,
) -> None:
    path = made(
        tmp_path,
        f"yellow.{extension}",
        "-f",
        "lavfi",
        "-i",
        "color=c=yellow:s=64x48:d=1",
        "-frames:v",
        "1",
    )
    async with await Engine.attach(tmp_path / "studio", program=real_engine()) as engine:
        items = await engine.inspected([path])
        assert items[0]["facts"]["video"]["still"] is True
        assert (await engine.project())[1]["media"] == []
        _, ids = await engine.imported([path])
        done = await engine.apply(
            [
                edits.add_timeline(
                    "Still",
                    edits.Rate(num=25, den=1),
                    id="$film",
                    width=64,
                    height=48,
                    tracks=[edits.track("video")],
                ),
                edits.insert("$film", edits.of_media(ids[0], length=50), 0),
            ]
        )
        film = tmp_path / "still.mov"
        job = await engine.render(done.names["$film"], film, codec="prores")
        try:
            assert (await job.finished()).state == "done"
        finally:
            await job.aclose()
    for at in (0.0, 1.9):
        read = await asyncio.to_thread(
            subprocess.run,
            [
                real(),
                "-v",
                "error",
                "-ss",
                str(at),
                "-i",
                str(film),
                "-frames:v",
                "1",
                "-vf",
                "scale=1:1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-",
            ],
            capture_output=True,
            check=True,
            timeout=30,
        )
        assert len(read.stdout) == 3
        r, g, b = read.stdout
        assert r > 180, (extension, at, read.stdout)
        assert g > 180, (extension, at, read.stdout)
        assert b < 60, (extension, at, read.stdout)
