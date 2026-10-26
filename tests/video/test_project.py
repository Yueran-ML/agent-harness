"""A project of editing: its sources, what is worked out of them, and its timeline."""

import json
import os
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.process import Completed
from columbina.video import media, project
from columbina.video.media import (
    MediaUnavailableError,
    Silence,
    Tools,
    UnreadableMediaError,
    located,
)
from columbina.video.project import (
    Canvas,
    ChangedSourceError,
    Identity,
    Manifest,
    Project,
    ProjectError,
    StaleError,
    digest_of,
    identity_of,
    named,
    written,
)
from support.media import made, real


@pytest.fixture
async def tools(tmp_path: Path) -> Tools:
    """The ffmpeg of this machine. A test that asks for it is passed over where there is none."""
    found = real()
    return await located(str(Path(found).parent), [], cwd=tmp_path)


def landscape(where: Path, name: str = "land.mp4", colour: str = "red") -> Path:
    return made(
        where, name,
        "-f", "lavfi", "-i", f"color=c={colour}:s=320x180:r=30:d=2",
        "-f", "lavfi", "-i", "sine=f=440:d=2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    )  # fmt: skip


def upright(where: Path) -> Path:
    plain = landscape(where, "plain.mp4", "green")
    return made(where, "up.mp4", "-display_rotation:v:0", "90", "-i", str(plain), "-c", "copy")


def still(where: Path) -> Path:
    return made(
        where, "still.png", "-f", "lavfi", "-i", "color=c=blue:s=64x48:d=1", "-frames:v", "1"
    )


def talk(where: Path) -> Path:
    """A second of tone, a pause, a second of tone, a pause, and half a second of tone."""
    tone = "if(between(t,1,1.8)+between(t,2.8,3.4),0,0.5*sin(440*2*PI*t))"
    return made(where, "talk.wav", "-f", "lavfi", "-i", f"aevalsrc='{tone}':d=3.9:s=16000")


# Ids and files


@pytest.mark.parametrize(
    ("name", "taken", "key"),
    [
        ("Interview Take 2.MP4", [], "interview-take-2"),
        ("采访 第一段.mov", [], "采访-第一段"),
        ("talk.wav", ["talk"], "talk-2"),
        ("talk.wav", ["talk", "talk-2"], "talk-3"),
        ("!!!.mp4", [], "source"),
        (f"{'a' * 50}.mp4", [], "a" * 40),
    ],
)
def test_a_source_is_named_for_its_file(name: str, taken: list[str], key: str) -> None:
    assert named(name, taken) == key


def test_a_file_is_written_whole_or_not_at_all(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "deep" / "file.json"
    written(target, "first")
    assert target.read_text(encoding="utf-8") == "first"

    def refused(self: Path, other: Path) -> Path:
        raise OSError("it is open elsewhere")

    monkeypatch.setattr(Path, "replace", refused)
    with pytest.raises(ProjectError, match="could not be written: it is open elsewhere"):
        written(target, "second")

    assert target.read_text(encoding="utf-8") == "first"
    assert [one.name for one in target.parent.iterdir()] == ["file.json"]


def test_the_identity_of_a_file_is_its_size_and_both_its_ends(tmp_path: Path) -> None:
    small, large = tmp_path / "small", tmp_path / "large"
    small.write_bytes(b"x" * 10)
    large.write_bytes(b"a" * 200_000)
    before = identity_of(large)

    large.write_bytes(b"a" * 100_000 + b"b" + b"a" * 99_999)
    middle = identity_of(large)
    large.write_bytes(b"a" * 199_999 + b"b")

    assert identity_of(small).size == 10
    # A change in the middle is found by the time, and not by the digest.
    assert middle.digest == before.digest
    assert identity_of(large).digest != before.digest


def test_a_manifest_of_another_version_is_not_read(tmp_path: Path) -> None:
    (tmp_path / "project.json").write_text(json.dumps({"version": 2}), encoding="utf-8")

    with pytest.raises(ProjectError, match="is not what this program writes"):
        Project(tmp_path).manifest()
    assert Project(tmp_path / "new").manifest() == Manifest()


# Taking sources in


async def test_sources_are_taken_in_as_ffprobe_tells_of_them(tmp_path: Path, tools: Tools) -> None:
    sources = tmp_path / "raw"
    files = [upright(sources), landscape(sources), still(sources), talk(sources)]
    work = Project(tmp_path / "film")

    done = await work.imported(files, tools)

    assert [one.id for one in done.added] == ["up", "land", "still", "talk"]
    for asset, path in zip(done.added, files, strict=True):
        told = await media.probed(tools, path, place=tmp_path / "probe")
        assert asset.model_dump(
            include={"kind", "format", "duration_us", "video", "audio"}
        ) == told.model_dump(include={"kind", "format", "duration_us", "video", "audio"})
        assert asset.path == str(path)
    up = done.added[0]
    assert up.video is not None
    assert (up.video.width, up.video.height, up.video.rotation) == (180, 320, 90)
    # The frame of the film is that of the first source with a picture, as it is shown.
    assert work.manifest().canvas == Canvas(width=180, height=320, rate="30/1")
    assert not list((tmp_path / "film" / "cache").glob(".run-*"))


async def test_taking_a_source_in_again_changes_nothing_unless_it_is_another_file(
    tmp_path: Path, tools: Tools
) -> None:
    source = landscape(tmp_path)
    work = Project(tmp_path / "film")
    (first,) = (await work.imported([source], tools)).added

    again = await work.imported([source], tools)
    landscape(tmp_path, colour="yellow")
    changed = await work.imported([source], tools)

    assert again.same == (first,)
    assert [one.id for one in changed.changed] == ["land"]
    assert changed.changed[0].identity.digest != first.identity.digest
    assert [one.id for one in work.manifest().assets] == ["land"]


async def test_the_frame_of_the_film_stays_when_its_first_source_changes(
    tmp_path: Path, tools: Tools
) -> None:
    source = landscape(tmp_path)
    work = Project(tmp_path / "film")
    await work.imported([source], tools)

    source.write_bytes(upright(tmp_path / "other").read_bytes())
    changed = await work.imported([source], tools)

    assert [one.id for one in changed.changed] == ["land"]
    assert work.manifest().canvas == Canvas(width=320, height=180, rate="30/1")


async def test_what_is_no_source_is_not_taken_in(tmp_path: Path, tools: Tools) -> None:
    text = tmp_path / "notes.txt"
    text.write_text("no film", encoding="utf-8")
    work = Project(tmp_path / "film")

    with pytest.raises(ProjectError, match="is no source that can be edited"):
        await work.imported([text], tools)
    with pytest.raises(ProjectError, match="cannot be read"):
        await work.imported([tmp_path / "gone.mp4"], tools)
    assert work.manifest() == Manifest()


# Using sources


async def test_a_source_that_was_replaced_is_not_used(tmp_path: Path, tools: Tools) -> None:
    source = landscape(tmp_path)
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([source], tools)).added

    os.utime(source, ns=(10**18, 10**18))
    touched = await work.checked(asset)
    assert touched.identity.digest == asset.identity.digest
    assert work.asset("land").identity.modified_ns == source.stat().st_mtime_ns
    assert await work.checked(touched) == touched

    landscape(tmp_path, colour="yellow")
    with pytest.raises(ChangedSourceError, match="another file than the one that was taken in"):
        await work.checked(touched)
    source.unlink()
    with pytest.raises(ChangedSourceError, match="is not there"):
        await work.checked(touched)
    with pytest.raises(ProjectError, match="has no source nothing"):
        work.asset("nothing")


async def test_the_sound_is_taken_out_once_as_a_transcriber_reads_it(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch
) -> None:
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([talk(tmp_path)], tools)).added

    sound = await work.cached_audio(asset, tools)
    told = await media.probed(tools, sound, place=tmp_path / "probe")

    assert told.audio is not None
    assert (told.audio.sample_rate, told.audio.channels) == (16000, 1)
    assert abs(told.duration_us - asset.duration_us) < 1000

    async def again(*args: object, **more: object) -> None:
        raise AssertionError("the sound was taken out again")

    monkeypatch.setattr(media, "audio", again)
    assert await work.cached_audio(asset, tools) == sound


async def test_the_pauses_of_a_talk_are_found_once_for_each_threshold(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch
) -> None:
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([talk(tmp_path)], tools)).added

    found = await work.cached_silences(asset, tools)

    assert len(found) == 2
    for silence, (start, end) in zip(
        found, [(1_000_000, 1_800_000), (2_800_000, 3_400_000)], strict=True
    ):
        assert abs(silence.start_us - start) <= 20_000
        assert abs(silence.end_us - end) <= 20_000

    async def again(*args: object, **more: object) -> tuple[Silence, ...]:
        raise AssertionError("the pauses were looked for again")

    monkeypatch.setattr(media, "silences", again)
    assert await work.cached_silences(asset, tools) == found
    with pytest.raises(AssertionError, match="looked for again"):
        await work.cached_silences(asset, tools, shortest_us=500_000)


async def test_a_talk_that_ends_in_silence_has_its_last_pause_end_with_it(
    tmp_path: Path, tools: Tools
) -> None:
    ending = made(
        tmp_path,
        "ending.wav",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc='if(lt(t,1),0.5*sin(440*2*PI*t),0)':d=2:s=16000",
    )
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([ending], tools)).added

    (silence,) = await work.cached_silences(asset, tools)

    assert silence.end_us == asset.duration_us


async def test_a_source_without_sound_has_none_to_take_out(tmp_path: Path, tools: Tools) -> None:
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([still(tmp_path)], tools)).added

    with pytest.raises(ProjectError, match="has no sound"):
        await work.cached_audio(asset, tools)
    with pytest.raises(ProjectError, match="has no sound"):
        await work.cached_silences(asset, tools)


# The timeline


def test_commits_are_undone_and_done_again(tmp_path: Path) -> None:
    work = Project(tmp_path)
    empty, base = work.timeline()
    assert empty == ""

    one = work.commit("one", base, "first")
    two = work.commit("two", one, "second")
    three = work.commit("three", two, "third")
    back = work.undo(three, "undo")
    assert work.undo(back, "undo") == one
    assert work.redo(one, "redo") == two

    assert work.timeline() == ("two", digest_of("two"))
    assert (tmp_path / "timeline.json").read_bytes() == b"two"
    # A commit after an undo leaves nothing to redo.
    work.commit("four", two, "fourth")
    with pytest.raises(ProjectError, match="nothing to redo"):
        work.redo(digest_of("four"), "redo")


def test_a_change_on_a_timeline_that_was_changed_since_is_refused(tmp_path: Path) -> None:
    work = Project(tmp_path)
    _, base = work.timeline()
    work.commit("one", base, "first")

    for change in (
        lambda: work.commit("two", base, "late"),
        lambda: work.undo(base, "late"),
        lambda: work.redo(base, "late"),
    ):
        with pytest.raises(StaleError, match="was changed since"):
            change()
    assert work.timeline()[0] == "one"


def test_nothing_is_undone_where_nothing_was_done(tmp_path: Path) -> None:
    work = Project(tmp_path)

    with pytest.raises(ProjectError, match="nothing to undo"):
        work.undo(work.timeline()[1], "undo")


def test_the_history_keeps_the_last_timelines(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(project, "HISTORY_MOST", 3)
    work = Project(tmp_path)
    digest = work.timeline()[1]
    for number in range(6):
        digest = work.commit(str(number), digest, "commit")

    kept = sorted((tmp_path / "history" / "undo").glob("*.json"))
    assert [json.loads(one.read_text(encoding="utf-8"))["timeline"] for one in kept] == [
        "2",
        "3",
        "4",
    ]
    assert json.loads(kept[-1].read_text(encoding="utf-8"))["note"] == "commit"


def test_a_timeline_that_cannot_be_read_is_said(tmp_path: Path) -> None:
    (tmp_path / "timeline.json").write_bytes(b"\xff")

    with pytest.raises(ProjectError, match="cannot be read"):
        Project(tmp_path).timeline()


# What goes wrong on the way


def ended(
    code: int | None, *, timed_out: bool = False, output: str = "", error: str = ""
) -> Completed:
    """How a run of a program ended, as a test says it."""
    return Completed(
        exit_code=code,
        timed_out=timed_out,
        output=output,
        output_bytes=len(output),
        omitted_bytes=0,
        saved_bytes=0,
        error_output=error,
    )


@pytest.mark.parametrize(
    ("done", "said"),
    [
        (ended(None, timed_out=True), "took more than"),
        (ended(1, error="Invalid data found\n"), "failed: Invalid data found"),
        (ended(1), "failed: 1"),
        (ended(0, output="not json"), "answered with no document"),
        (ended(0, output="[1]"), "answered with no document"),
    ],
)
async def test_what_ffprobe_cannot_answer_is_said(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch, done: Completed, said: str
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        return done

    monkeypatch.setattr(process, "run", run)

    with pytest.raises(UnreadableMediaError, match=said):
        await media.probed(tools, tmp_path / "x.mp4", place=tmp_path)


async def test_an_ffmpeg_that_cannot_be_started_any_more_is_said(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        raise OSError("gone")

    monkeypatch.setattr(process, "run", run)

    with pytest.raises(MediaUnavailableError, match="could not be started: gone"):
        await media.audio(tools, tmp_path / "x.mp4", tmp_path / "x.wav")


async def test_a_source_that_cannot_be_read_when_it_is_looked_at_again_is_said(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = landscape(tmp_path)
    work = Project(tmp_path / "film")
    (asset,) = (await work.imported([source], tools)).added
    os.utime(source, ns=(10**18, 10**18))

    def refused(path: Path) -> Identity:
        raise OSError("it is locked")

    monkeypatch.setattr(project, "identity_of", refused)

    with pytest.raises(ChangedSourceError, match="cannot be read: it is locked"):
        await work.checked(asset)
