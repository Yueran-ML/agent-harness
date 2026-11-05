"""The agent reaches the engine that has a project open, as one client among others."""

import asyncio
import contextlib
import json
import os
import subprocess
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from columbina.video import edits, engine
from columbina.video.engine import (
    NOT_FOUND,
    UNEXPECTED,
    UNREACHABLE,
    UNSTARTED,
    Applied,
    Changed,
    Changes,
    Channel,
    Chroma,
    Engine,
    EngineError,
    Lut,
    Normalization,
    Progress,
    RenderAudio,
    not_found,
    program_of,
)
from support import fake_engine
from support.fake_engine import LIMIT, served
from support.media import made, real_engine

FAKE = Path(fake_engine.__file__)
ADD = edits.add_timeline("Cut", edits.Rate(num=25, den=1), id="$t")


def as_program(monkeypatch: pytest.MonkeyPatch, *mode: str) -> None:
    """Start the fake engine as the program, in a mode."""
    monkeypatch.setattr(engine, "_argv", fake_engine.argv_of(*mode))


def not_an_engines() -> str:
    """An address that is no engine's, where a test may listen all the same."""
    digits = os.urandom(8).hex()
    return rf"\\.\pipe\not-an-engine-{digits}" if sys.platform == "win32" else f"/tmp/{digits}.sock"


async def attached(directory: Path, **more: Any) -> Engine:
    return await Engine.attach(directory, program=str(FAKE), **more)


async def until(done: Callable[[], bool], seconds: float = 10.0) -> None:
    deadline = time.monotonic() + seconds
    while not done():
        assert time.monotonic() < deadline, "it did not happen in time"
        await asyncio.sleep(0.02)


async def next_of(changes: Changes) -> Changed:
    return await asyncio.wait_for(anext(changes), 5)


def logs(project: Path) -> list[Path]:
    return sorted((project / "cache").glob("engine-*.log"))


# The protocol


async def test_a_client_says_hello_and_is_given_the_project(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")

        async with connected:
            revision, project = await connected.project()

        assert connected.hello["pid"] == os.getpid()
        assert connected.hello["engine"] == "0.1.0-fake"
        assert (connected.revision, connected.next_id, revision) == (0, 1, 0)
        assert project["media"] == []
        assert fake.asked == ["hello", "project.get", "project.get"]


async def test_answers_are_matched_to_their_requests_and_changes_come_before_them(
    tmp_path: Path,
) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        fake.hold = 3

        timeline, project, sound = await asyncio.gather(
            connected.apply([ADD]), connected.project(), connected.imported([FAKE])
        )
        first, second = await next_of(changes), await next_of(changes)
        fake.tell("media.proxy", {"media": "m2", "state": "queued", "progress": 0})
        proxy = await next_of(changes)
        await connected.aclose()

        assert timeline.names == {"$t": "t1"}
        assert timeline.done[0]["timeline"]["id"] == "t1"
        assert project[0] == 1
        assert sound == (2, ["m2"])
        assert (first.method, first.revision, first.params["by"]) == ("project.changed", 1, "agent")
        assert second.revision == 2
        assert (proxy.method, proxy.revision) == ("media.proxy", None)
        assert connected.next_id == 3


async def test_every_client_is_told_of_every_change(tmp_path: Path) -> None:
    async with served(tmp_path / "film"):
        one = await attached(tmp_path / "film")
        other = await attached(tmp_path / "film", client="cli")
        told = other.changes()

        applied = await one.apply([ADD])
        changed = await next_of(told)
        undone = await other.undo()
        nothing = await other.undo()
        redone = await one.redo()
        again, third = await next_of(told), await next_of(told)
        await asyncio.gather(one.aclose(), other.aclose())

        assert changed.params["done"] == applied.done
        assert changed.params["made_by"] == "agent"
        assert undone is not None
        assert undone.revision == 2
        assert nothing is None
        assert isinstance(redone, Applied)
        assert redone.revision == 3
        assert (again.revision, third.revision) == (2, 3)
        assert (again.params["by"], again.params["made_by"]) == ("cli", "agent")
        assert other.revision == 3


async def test_inspection_does_not_import_or_change_history(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        items = await connected.inspected([FAKE])
        assert len(items) == 1
        assert items[0]["id"] == ""
        assert items[0]["path"] == str(FAKE)
        assert fake.project["media"] == []
        assert fake.steps == []
        fake.results["media.inspect"] = {"media": [None]}
        with pytest.raises(EngineError, match="no media"):
            await connected.inspected([FAKE])


async def test_the_statistics_of_a_monitor_are_read_into_their_figures(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        with pytest.raises(EngineError) as headless:
            await connected.statistics()
        assert headless.value.code == -32601
        fake.headless = False
        figures = await connected.statistics("source", bins=16)
        assert fake.looked[-1] == ("scopes.statistics", {"view": "source", "bins": 16})
        assert (figures.view, figures.frame, figures.width, figures.height) == ("source", 12, 4, 2)
        assert figures.samples == 8
        grey = 128 / 255
        assert figures.luma == Channel(min=grey, max=grey, mean=grey)
        assert figures.chroma == Chroma(
            centroid=(0.0, 0.0), spread=(0.0, 0.0), radius=0.0, saturation=0.0, hue=0.0
        )
        assert figures.percentiles["red"]["50"] == grey
        assert figures.bins == 16
        assert sum(figures.histogram["luma"]) == 8
        await connected.statistics()
        assert fake.looked[-1] == ("scopes.statistics", {"view": "timeline"})
        fake.results["scopes.statistics"] = {"view": "timeline", "frame": 0}
        with pytest.raises(EngineError) as odd:
            await connected.statistics()
        assert odd.value.code == UNEXPECTED


async def test_the_library_of_luts_is_read_into_its_roots_and_files(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        library = await connected.luts()
        assert fake.looked[-1] == ("luts.list", {})
        assert [(root.place, root.exists) for root in library.roots] == [
            ("project", True),
            ("library", False),
        ]
        assert library.luts == [
            Lut(
                name="LUTs/film.cube",
                root=0,
                size=120,
                modified_us=1_790_000_000_000_000,
                shadowed=False,
                kind="3d",
                status="ok",
            )
        ]
        assert not library.truncated
        await connected.luts("LUTs")
        assert fake.looked[-1] == ("luts.list", {"folder": "LUTs"})
        with pytest.raises(EngineError) as outside:
            await connected.luts("../elsewhere")
        assert outside.value.code == -32602
        assert outside.value.data == {"reason": "outside_roots", "folder": "../elsewhere"}
        fake.results["luts.list"] = {"roots": [], "luts": [{"name": "x.cube"}], "truncated": False}
        with pytest.raises(EngineError) as odd:
            await connected.luts()
        assert odd.value.code == UNEXPECTED


async def test_each_lut_says_its_kind_and_whether_it_parses(tmp_path: Path) -> None:
    root = {"place": "project", "path": str(tmp_path), "exists": True}
    lut = {"name": "a.cube", "root": 0, "size": 9, "modified_us": None, "shadowed": False}
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        fake.results["luts.list"] = {
            "roots": [root],
            "luts": [
                {**lut, "kind": "shaper+3d", "status": "ok"},
                {**lut, "kind": "3d", "status": "error", "message": "line 3: too few"},
                {**lut, "kind": None, "status": "unchecked"},
                # An engine from before the kind and the status: unchecked, of no kind.
                lut,
            ],
            "truncated": False,
        }
        said = [(one.kind, one.status, one.message) for one in (await connected.luts()).luts]
        assert said == [
            ("shaper+3d", "ok", None),
            ("3d", "error", "line 3: too few"),
            (None, "unchecked", None),
            (None, "unchecked", None),
        ]
        for odd in (
            {**lut, "kind": "2d", "status": "ok"},
            {**lut, "status": "broken"},
            {**lut, "status": "error"},
            {**lut, "status": "error", "message": 3},
        ):
            fake.results["luts.list"] = {"roots": [root], "luts": [odd], "truncated": False}
            with pytest.raises(EngineError) as unexpected:
                await connected.luts()
            assert unexpected.value.code == UNEXPECTED, odd


async def test_a_refusal_keeps_what_the_engine_said_of_it(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")

        with pytest.raises(EngineError, match="no operations") as empty:
            await connected.apply([])
        with pytest.raises(EngineError) as unknown:
            await connected.apply([{"op": "add_track", "timeline": "$t", "track": {"id": "$v"}}])
        with pytest.raises(EngineError) as relative:
            await connected.imported([Path("raw/a.mp4")])
        with pytest.raises(EngineError) as unreadable:
            await connected.imported([tmp_path / "nothing.mp4"])
        fake.refusals["history.undo"] = {"code": "odd", "data": [1]}
        with pytest.raises(EngineError, match="the engine refused") as odd:
            await connected.undo()

        assert (empty.value.code, empty.value.data) == (
            -32001,
            {"reason": "empty", "message": "the transaction has no operations"},
        )
        assert unknown.value.code == -32602
        assert unknown.value.data == {"index": 0, "name": "$t", "reason": "unknown_name"}
        assert relative.value.data == {
            "index": 0,
            "path": str(Path("raw/a.mp4")),
            "reason": "relative",
        }
        assert unreadable.value.code == -32002
        assert unreadable.value.data is not None
        assert unreadable.value.data["reason"] == "not_found"
        assert (odd.value.code, odd.value.data) == (UNEXPECTED, None)


async def test_a_request_that_the_engine_cannot_read_is_not_sent(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")

        with pytest.raises(EngineError, match="none longer than 16 MiB") as long:
            await connected.apply([{"op": "x", "pad": "x" * LIMIT}])
        with pytest.raises(EngineError, match="cannot be written in JSON") as nan:
            await connected.apply([{"op": "x", "at": float("nan")}])
        with pytest.raises(EngineError, match="cannot be written in JSON"):
            await connected.apply([{"op": "x", "at": object()}])
        fake.anonymous = "history.undo"
        with pytest.raises(EngineError, match="could not be read") as unread:
            await connected.undo()
        fake.anonymous = "project.get"
        oldest, newest = await asyncio.wait_for(
            asyncio.gather(connected.project(), connected.undo(), return_exceptions=True), 5
        )
        revision, _ = await connected.project()
        await connected.aclose()

        assert long.value.code == -32600
        assert long.value.data is not None
        assert long.value.data["bytes"] > LIMIT
        assert nan.value.code == -32602
        assert unread.value.code == -32600
        # An error that names no request is the oldest one's.
        assert isinstance(oldest, EngineError)
        assert oldest.code == -32600
        assert newest is None
        assert revision == 0
        assert "transaction.apply" not in fake.asked


async def test_answers_that_are_not_of_the_protocol_are_refused(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.results = {
            "project.get": {"revision": 0},
            "transaction.apply": {"revision": 1, "done": "x", "next_id": 2, "names": {}},
            "media.import": {"revision": 1, "added": [1]},
            "history.undo": {"revision": "x", "done": None},
            "history.redo": None,
            "timeline.duration": {"frames": "x"},
            "render.start": {"job": 7},
        }

        for call in (
            connected.project(),
            connected.apply([ADD]),
            connected.imported([FAKE]),
            connected.undo(),
            connected.redo(),
            connected.duration("t1"),
            connected.render("t1", tmp_path / "film.mp4"),
        ):
            with pytest.raises(EngineError) as refused:
                await call
            assert refused.value.code == UNEXPECTED


async def test_what_is_not_a_message_is_passed_over(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()

        fake.say(
            b"",
            b"\r",
            b"not json",
            b"[1]",
            b'{"jsonrpc": "2.0", "id": 999, "result": {}}',
            b'{"jsonrpc": "2.0", "id": "x", "result": {}}',
            b'{"jsonrpc": "2.0", "id": null, "result": {}}',
            b'{"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "no"}}',
            b'{"jsonrpc": "2.0"}',
            b'{"jsonrpc": "2.0", "method": "project.changed", "params": {}}',
            b'{"jsonrpc": "2.0", "method": "project.changed", "params": {"revision": 0}}',
            b'{"jsonrpc": "2.0", "method": "engine.error", "params": [1]}',
            b'{"jsonrpc": "2.0", "method": "project.changed", "params": {"revision": 1}}',
        )
        error = await next_of(changes)
        changed = await next_of(changes)

        assert error == Changed("engine.error", {})
        assert changed.revision == 1
        assert (connected.revision, connected.next_id) == (1, 1)
        await connected.aclose()


async def test_a_change_that_was_missed_brings_the_whole_project_again(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        fake.gap = True

        await asyncio.gather(connected.apply([ADD]), connected.imported([FAKE]))
        fetched = await next_of(changes)
        await connected.apply([{"op": "remove_timeline", "id": "t1"}])
        after = await next_of(changes)

        assert fetched.method == "project.fetched"
        assert fetched.revision == 3
        assert [one["id"] for one in fetched.params["project"]["media"]] == ["m2"]
        assert after.revision == 4
        assert (connected.revision, connected.next_id) == (4, 3)
        assert fake.asked.count("project.get") == 2


async def test_a_reader_that_reads_too_little_is_told_what_it_missed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(engine, "_LISTENED", 2)
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")

        async with connected.changes() as changes:
            for number in range(4):
                fake.tell("media.proxy", {"media": f"m{number}"})
            # Its answer comes after what was told.
            await connected.project()
            missed, third, fourth = [await next_of(changes) for _ in range(3)]
        fake.tell("media.proxy", {"media": "m9"})
        await connected.project()
        await connected.aclose()

        assert missed == Changed("changes.missed", {"missed": 2})
        assert (third.params["media"], fourth.params["media"]) == ("m2", "m3")
        assert [one async for one in changes] == []
        assert [one async for one in connected.changes()] == []


async def test_a_lost_connection_fails_what_waits_and_is_made_again(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        fake.drop = "transaction.apply"

        with pytest.raises(EngineError, match="closed the connection") as lost:
            await connected.apply([ADD])
        with pytest.raises(EngineError):
            await next_of(changes)
        fake.then["project.get"] = lambda: fake.change_by("desktop")
        revision, _ = await connected.project()
        fetched, changed = await next_of(changes), await next_of(changes)

        assert lost.value.code == UNREACHABLE
        assert revision == 1
        assert (fetched.method, fetched.revision) == ("project.fetched", 0)
        assert (changed.method, changed.revision) == ("project.changed", 1)
        assert changed.params["by"] == "desktop"
        assert fake.asked[-3:] == ["hello", "project.get", "project.get"]
        await connected.aclose()


async def test_a_revision_read_from_another_engine_is_refused_without_sending(
    tmp_path: Path,
) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        revision, _ = await connected.project()
        session = connected.session
        # An engine started anew counts its revisions from 0 again: the same number of another
        # engine says nothing of the project that was read.
        connected._drop()  # pyright: ignore[reportPrivateUsage]

        with pytest.raises(EngineError) as refused:
            await connected.apply([ADD], revision=revision, session=session)
        done = await connected.apply([ADD], revision=revision, session=connected.session)

        assert refused.value.data == {"reason": "stale_revision", "session": session + 1}
        assert fake.asked.count("transaction.apply") == 1
        assert done.revision == revision + 1
        await connected.aclose()


async def test_a_closed_engine_answers_nothing_more(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        fake.hold = 3
        waiting = asyncio.create_task(connected.project())
        given_up = asyncio.create_task(connected.project())
        await until(lambda: fake.hold == 1)

        given_up.cancel()
        await connected.aclose()
        await connected.aclose()

        with pytest.raises(EngineError, match="was closed"):
            await waiting
        with pytest.raises(asyncio.CancelledError):
            await given_up
        with pytest.raises(EngineError, match="was closed"):
            await connected.project()
        assert [one async for one in changes] == []
        assert [one async for one in changes] == []
        await until(lambda: not fake.connections)


async def test_an_attach_given_up_while_greeting_leaves_no_connection(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        fake.hold = 5

        with pytest.raises(TimeoutError):
            await asyncio.wait_for(attached(tmp_path / "film"), 0.5)

        await until(lambda: not fake.connections)


async def test_an_engine_closed_while_it_connects_again_leaves_no_connection(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    async with served(tmp_path / "film") as fake:
        greeting = await attached(tmp_path / "film")
        connecting = await attached(tmp_path / "film")
        fake.drop = "project.get"
        with pytest.raises(EngineError):
            await greeting.project()
        fake.drop = "project.get"
        with pytest.raises(EngineError):
            await connecting.project()
        fake.hold = 3
        call = asyncio.create_task(greeting.project())
        await until(lambda: fake.hold == 2)
        await greeting.aclose()
        with pytest.raises(EngineError, match="was closed"):
            await call
        fake.hold = 0
        original = engine._connected  # pyright: ignore[reportPrivateUsage]

        async def closed_meanwhile(*args: Any) -> Any:
            wire = await original(*args)
            await connecting.aclose()
            return wire

        monkeypatch.setattr(engine, "_connected", closed_meanwhile)
        with pytest.raises(EngineError, match="was closed"):
            await connecting.project()

        await until(lambda: not fake.connections)


async def test_a_connection_is_used_and_closed_before_it_is_told_that_it_was_made(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A pipe of Windows is handed over before its connection_made runs, from Python 3.12 on; here
    # it never runs, on any system.
    made: list[asyncio.BaseTransport] = []

    def told_never(_: object, transport: asyncio.BaseTransport) -> None:
        made.append(transport)

    monkeypatch.setattr(engine._Wire, "connection_made", told_never)  # pyright: ignore[reportPrivateUsage]
    async with served(tmp_path / "film") as fake:
        fake.hold = 5
        with pytest.raises(TimeoutError):
            await asyncio.wait_for(attached(tmp_path / "film"), 0.5)
        await until(lambda: not fake.connections)
        fake.hold = 0

        connected = await attached(tmp_path / "film")
        revision, _ = await connected.project()
        await connected.aclose()

        await until(lambda: not fake.connections)
        assert revision == 0
        assert len(made) == 2


async def test_a_hello_that_is_refused_as_if_no_engine_answered_leaves_no_connection(
    tmp_path: Path,
) -> None:
    async with served(tmp_path / "film") as fake:
        fake.refusals["hello"] = {"code": UNREACHABLE, "message": "not now"}

        with pytest.raises(EngineError, match="could be reached: not now"):
            await attached(tmp_path / "film")

        assert fake.asked.count("hello") > 1
        await until(lambda: not fake.connections)


async def test_a_line_too_long_to_read_says_the_project_is_too_large(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(engine, "_READ_BYTES", 1 << 20)
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.huge, fake.huge_bytes = "project.get", 2 << 20

        with pytest.raises(EngineError, match="too large to be read") as huge:
            await connected.project()
        revision, _ = await connected.project()
        await connected.aclose()
        fake.huge = "hello"
        with pytest.raises(EngineError, match="too large to be read") as greeting:
            await attached(tmp_path / "film")

        assert huge.value.code == greeting.value.code == UNEXPECTED
        assert revision == 0
        assert fake.asked[-1] == "hello"
        assert fake.asked.count("hello") == 3


@pytest.mark.parametrize("parts", [[b"x" * 1001 + b"\n{}\n"], [b"x" * 1000, b"xx"]])
def test_a_line_too_long_is_not_read_and_nothing_is_heard_after_the_end(
    parts: list[bytes], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(engine, "_READ_BYTES", 1000)
    heard: list[bytes] = []
    lost: list[EngineError] = []
    wire = engine._Wire(lambda _, line: heard.append(line), lambda _, why: lost.append(why))  # pyright: ignore[reportPrivateUsage]

    class Transport(asyncio.Transport):
        def close(self) -> None:
            wire.connection_lost(None)

    wire.connection_made(Transport())
    wire.data_received(b'{"a": 1}\n')
    for part in parts:
        wire.data_received(part)
    # It is given up as soon as it is too long, whether it has ended or not.
    given_up = [one.code for one in lost]
    wire.data_received(b"{}\n")

    assert heard == [b'{"a": 1}']
    assert given_up == [UNEXPECTED]
    assert [one.code for one in lost] == [UNEXPECTED]
    assert "the project is too large to be read here" in str(lost[0])


# Edits, renders and durations


async def timeline_with_a_clip(connected: Engine) -> str:
    """A timeline with a track of picture and a clip of a colour on it, and its id."""
    applied = await connected.apply(
        [
            ADD,
            edits.add_track("$t", "video"),
            edits.append("$t", edits.of_color("#000000ff", 50)),
        ]
    )
    return applied.names["$t"]


async def test_a_transaction_of_edits_is_answered_with_operations_and_what_was_clamped(
    tmp_path: Path,
) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        timeline = await timeline_with_a_clip(connected)
        await next_of(changes)
        fake.clamped = [1]

        applied = await connected.apply(
            [edits.blade(timeline, 10), edits.trim(timeline, "c2", "out", 100)]
        )
        told = await next_of(changes)
        await connected.apply([edits.set_track(timeline, "V1", locked=True)])
        with pytest.raises(EngineError) as locked:
            await connected.apply([edits.ripple_delete(timeline, ["c2"])])
        fake.results["transaction.apply"] = {
            "revision": 9, "done": [], "next_id": 3, "names": {}, "clamped": ["1"],
        }  # fmt: skip
        with pytest.raises(EngineError) as odd:
            await connected.apply([ADD])
        undone = await connected.undo()
        await connected.aclose()

        assert applied.clamped == [1]
        assert all("op" in one and "edit" not in one for one in applied.done)
        assert told.params["done"] == applied.done
        assert [one["edit"] for one in fake.edits[:3]] == ["add_track", "append", "blade"]
        assert locked.value.code == -32001
        assert locked.value.data == {
            "index": 0, "reason": "locked", "track": "V1", "message": "track V1 is locked",
        }  # fmt: skip
        assert odd.value.code == UNEXPECTED
        assert undone is not None
        assert undone.clamped == []


async def test_how_long_a_timeline_lasts_is_asked_of_the_engine(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        timeline = await timeline_with_a_clip(connected)

        frames = await connected.duration(timeline)
        with pytest.raises(EngineError) as missing:
            await connected.duration("t99")
        await connected.aclose()

        assert frames == 50
        assert missing.value.code == -32602
        assert fake.asked.count("timeline.duration") == 2


async def test_a_render_is_followed_until_it_is_done(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        timeline = await timeline_with_a_clip(connected)
        await connected.apply([edits.add_track(timeline, "subtitle")])
        film = tmp_path / "out" / "cut.mp4"
        film.parent.mkdir()
        heard: list[Progress] = []

        rendering = await connected.render(
            timeline,
            film,
            codec="hevc",
            quality="small",
            frames=(0, 50),
            subtitle_tracks=["S1"],
        )
        ended = await asyncio.wait_for(rendering.finished(heard.append), 5)
        again = [one async for one in rendering.progress()]
        await connected.aclose()

        assert fake.rendered == [
            {
                "timeline": timeline,
                "path": str(film),
                "codec": "hevc",
                "quality": "small",
                "subtitle_tracks": ["S1"],
                "replace": False,
                "range": {"from": 0, "to": 50},
            }
        ]
        assert [(one.state, one.frame) for one in heard] == [
            ("running", 0),
            ("running", 25),
            ("done", 50),
        ]
        assert ended == heard[-1] == rendering.last
        assert (ended.path, ended.encoder, ended.frames, ended.over) == (
            str(film),
            "h264_fake",
            50,
            True,
        )
        assert again == [ended]
        assert film.read_bytes() == b"a film of the fake engine"
        # What the render told was heard by a reader of its own, which is closed with it.
        assert list(connected._readers) == []  # pyright: ignore[reportPrivateUsage]


async def test_a_render_that_fails_or_is_cancelled_says_so(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        timeline = await timeline_with_a_clip(connected)
        fake.render_end = "failed"

        failed = await (await connected.render(timeline, tmp_path / "a.mp4")).finished()
        fake.render_hold = True
        held = await connected.render(timeline, tmp_path / "b.mp4")
        first = await asyncio.wait_for(anext(held.progress()), 5)
        await held.cancel()
        cancelled = await asyncio.wait_for(held.finished(), 5)
        with pytest.raises(EngineError) as unknown:
            await connected._call("render.cancel", {"job": "r9"})  # pyright: ignore[reportPrivateUsage]
        await connected.aclose()

        assert (failed.state, failed.message, failed.path) == (
            "failed",
            "the encoder failed",
            None,
        )
        assert (first.state, first.over) == ("running", False)
        assert cancelled.state == "cancelled"
        assert unknown.value.data is not None
        assert unknown.value.data["reason"] == "missing"
        assert not (tmp_path / "a.mp4").exists()
        assert not (tmp_path / "b.mp4").exists()


async def test_an_analysis_of_sound_is_followed_until_it_is_done(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.sound_found = {"results": [{"target": {"bus": "B1"}, "integrated": -23.0}]}
        params = {"timeline": "t1", "targets": [{"bus": "B1"}]}

        found = await asyncio.wait_for(connected.analyze_sound("audio.analyze", params), 5)
        fake.sound_end = "failed"
        with pytest.raises(EngineError) as failed:
            await asyncio.wait_for(
                connected.analyze_sound("audio.normalize", {"timeline": "t1", "clips": ["c1"]}), 5
            )
        fake.sound_end = "done"
        fake.sound_hold = True
        waiting = asyncio.create_task(
            connected.analyze_sound(
                "audio.level_match", {"timeline": "t1", "reference": "c1", "clips": ["c2"]}
            )
        )
        await until(lambda: "audio.level_match" in fake.asked and "audio-3" in fake.jobs)
        waiting.cancel()
        with pytest.raises(asyncio.CancelledError):
            await waiting
        await until(lambda: fake.jobs["audio-3"] == "cancelled")
        await connected.aclose()

        assert found["method"] == "audio.analyze"
        assert found["results"] == [{"target": {"bus": "B1"}, "integrated": -23.0}]
        assert [method for method, _ in fake.sounds] == [
            "audio.analyze",
            "audio.normalize",
            "audio.level_match",
        ]
        assert fake.sounds[0][1] == params
        assert failed.value.code == UNEXPECTED
        assert failed.value.data is not None
        assert failed.value.data["state"] == "failed"
        assert "a clip cannot be read" in str(failed.value)
        assert "audio.cancel" in fake.asked


async def test_an_analysis_given_up_before_it_is_named_is_cancelled(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.sound_hold = True
        # Its answer is held until another request comes.
        fake.hold = 2
        waiting = asyncio.create_task(
            connected.analyze_sound("audio.analyze", {"timeline": "t1", "targets": [{"bus": "B1"}]})
        )
        await until(lambda: "audio.analyze" in fake.asked)
        waiting.cancel()
        with contextlib.suppress(EngineError):
            await connected.duration("t9")
        with pytest.raises(asyncio.CancelledError):
            await waiting
        await until(lambda: fake.jobs.get("audio-1") == "cancelled")
        await connected.aclose()

        assert "audio.cancel" in fake.asked


async def test_a_render_that_is_refused_is_followed_no_further(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        timeline = await timeline_with_a_clip(connected)
        (tmp_path / "there.mp4").write_bytes(b"")

        with pytest.raises(EngineError) as there:
            await connected.render(timeline, tmp_path / "there.mp4")
        with pytest.raises(EngineError) as relative:
            await connected.render(timeline, Path("film.mp4"))
        revision, _ = await connected.project()
        with pytest.raises(EngineError) as stale:
            await connected.render(
                timeline, tmp_path / "there.mp4", replace=True, revision=revision - 1
            )
        replaced = await connected.render(
            timeline, tmp_path / "there.mp4", replace=True, revision=revision
        )
        done = await asyncio.wait_for(replaced.finished(), 5)
        fake.render_hold = True
        lost = await connected.render(timeline, tmp_path / "lost.mp4")
        await asyncio.wait_for(anext(lost.progress()), 5)
        fake.drop = "project.get"
        with pytest.raises(EngineError):
            await connected.project()
        with pytest.raises(EngineError, match="closed the connection") as gone:
            await asyncio.wait_for(lost.finished(), 5)
        await lost.aclose()
        await connected.aclose()

        assert (there.value.code, there.value.data) == (
            -32002,
            {"path": str(tmp_path / "there.mp4"), "reason": "exists"},
        )
        assert relative.value.code == -32602
        assert (stale.value.code, stale.value.data) == (
            -32001,
            {"reason": "stale_revision", "expected": revision - 1, "actual": revision},
        )
        assert fake.rendered[-3]["revision"] == revision - 1
        assert done.state == "done"
        assert gone.value.code == UNREACHABLE
        assert list(connected._readers) == []  # pyright: ignore[reportPrivateUsage]


async def test_the_sound_of_a_render_is_asked_for_as_the_protocol_says_it(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        timeline = await timeline_with_a_clip(connected)
        normalized = RenderAudio(
            bus="B2",
            rate=96000,
            codec="aac",
            bitrate=256000,
            normalize=Normalization("spotify", "optimize", target=-16.0),
        )

        film = await connected.render(timeline, tmp_path / "film.mp4", audio=normalized)
        await asyncio.wait_for(film.finished(), 5)
        alone = await connected.render(
            timeline, tmp_path / "sound.wav", audio=RenderAudio(codec="pcm_f32le"), video=False
        )
        await asyncio.wait_for(alone.finished(), 5)
        await connected.aclose()

        assert fake.rendered[0]["audio"] == {
            "bus": "B2",
            "rate": 96000,
            "codec": "aac",
            "bitrate": 256000,
            "normalize": {"standard": "spotify", "mode": "optimize", "target": -16.0},
        }
        assert (fake.rendered[0]["codec"], fake.rendered[0]["quality"]) == ("h264", "high")
        assert fake.rendered[1]["video"] is None
        assert fake.rendered[1]["audio"] == {"codec": "pcm_f32le"}
        assert {"codec", "quality"} & set(fake.rendered[1]) == set()
        # Nothing of the sound is sent where nothing is asked of it.
        assert RenderAudio().params() == {}
        assert Normalization("ebu-r128").params() == {"standard": "ebu-r128", "mode": "normalize"}


def test_what_a_render_tells_is_read_as_the_protocol_says_it() -> None:
    said = {"job": "r1", "state": "running", "frame": 3, "frames": 9, "fps": 24}

    assert Progress.of(said) == Progress("r1", "running", 3, 9, 24.0)
    measuring = {"pass": 1, "passes": 2, "frame": 4800, "frames": 96000}
    normalization = {"standard": "ebu-r128", "gain_db": 3.5, "met": True}
    read = Progress.of({**said, "measuring": measuring, "normalization": normalization})
    assert read is not None
    assert (read.measuring, read.normalization) == (measuring, normalization)
    assert Progress.of({**said, "measuring": [1], "normalization": "loud"}) == Progress(
        "r1", "running", 3, 9, 24.0
    )
    assert Progress.of({**said, "frame": "3", "fps": True, "path": 7}) == Progress(
        "r1", "running", 0, 9, 0.0
    )
    assert Progress.of({**said, "state": "paused"}) is None
    assert Progress.of({**said, "job": 1}) is None


# Reaching the engine


async def test_an_address_where_nobody_listens_is_looked_up_again_and_an_engine_started(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    project = tmp_path / "film"
    (project / "cache").mkdir(parents=True)
    for number in range(25):
        (project / "cache" / f"engine-20000101T0000{number:02}-0.log").write_bytes(b"")
    (project / "engine.lock").write_text(
        json.dumps({"pid": 1, "address": fake_engine.address_of()}), encoding="utf-8"
    )
    as_program(monkeypatch)

    connected = await attached(project, idle_seconds=1)
    pid = connected.hello["pid"]
    locked = json.loads((project / "engine.lock").read_text(encoding="utf-8"))
    await connected.aclose()
    await until(lambda: not (project / "engine.lock").exists())
    log = logs(project)[-1]
    await until(lambda: '"connected"' in log.read_text(encoding="utf-8"))

    assert pid != os.getpid()
    assert locked["pid"] == pid
    assert '"listening"' in log.read_text(encoding="utf-8")
    assert len(logs(project)) == 20


async def test_an_address_that_is_no_engines_is_not_opened(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    async with served(tmp_path / "film", address=not_an_engines()) as elsewhere:
        as_program(monkeypatch)

        async with await attached(tmp_path / "film", idle_seconds=1) as connected:
            assert connected.hello["pid"] != os.getpid()

        assert elsewhere.asked == []
    await until(lambda: not (tmp_path / "film" / "engine.lock").exists())


async def test_why_an_engine_did_not_start_is_read_where_its_log_cannot_be_kept(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    project = tmp_path / "film"
    project.mkdir()
    (project / "cache").write_text("a file", encoding="utf-8")
    as_program(monkeypatch, "error")

    with pytest.raises(EngineError, match=r"project.json cannot be read"):
        await attached(project)


async def test_an_engine_that_finds_the_project_held_leads_to_the_one_that_holds_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Within one try: no other engine is started after the one that found it held.
    monkeypatch.setattr(engine, "_TRIES", 1)
    async with served(tmp_path / "film", lock=False) as fake:
        as_program(monkeypatch, "held", fake.address)
        directly = await attached(tmp_path / "film")
        as_program(monkeypatch, "held-moved", fake.address)
        (tmp_path / "film" / "engine.lock").unlink(missing_ok=True)
        moved = await attached(tmp_path / "film")

        assert directly.hello["pid"] == moved.hello["pid"] == os.getpid()
        await asyncio.gather(directly.aclose(), moved.aclose())


async def test_an_engine_that_is_starting_elsewhere_is_waited_for(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    as_program(monkeypatch, "contended-once")

    async with await attached(tmp_path / "film", idle_seconds=1) as connected:
        assert (tmp_path / "film" / "contended").exists()
        assert connected.hello["pid"] != os.getpid()
        assert len(logs(tmp_path / "film")) == 1


@pytest.mark.parametrize(
    ("mode", "said"),
    [
        (("held",), "the engine that has it open did not answer$"),
        (("held", r"\\evil\pipe\x"), "the engine that has it open did not answer$"),
        (("held", fake_engine.address_of()), "the engine that has it open did not answer at "),
        (("contended",), "another engine is starting for it"),
    ],
)
async def test_an_engine_that_cannot_be_reached_is_given_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: tuple[str, ...], said: str
) -> None:
    as_program(monkeypatch, *mode)
    monkeypatch.setattr(engine, "_TRIES", 2)

    with pytest.raises(EngineError, match=said) as given_up:
        await attached(tmp_path / "film")

    assert given_up.value.code == UNREACHABLE


@pytest.mark.parametrize(
    ("mode", "said", "code"),
    [
        ("error", "project.json cannot be read", 2),
        ("exit", "ended with exit code 5 before it listened", 5),
        ("babble", "did not say where it listens in protocol 2, at an add", None),
        ("elsewhere", r"where an engine listens: b'{\"listening\"", None),
        ("silent", "did not say where it listens within 0.5 seconds", None),
    ],
)
async def test_an_engine_that_cannot_start_says_why(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str, said: str, code: int | None
) -> None:
    as_program(monkeypatch, mode)
    monkeypatch.setattr(engine, "_LISTENING_SECONDS", 0.5)
    started = time.monotonic()

    with pytest.raises(EngineError, match=said) as failed:
        await attached(tmp_path / "film")

    assert failed.value.code == UNSTARTED
    assert failed.value.data == (None if code is None else {"exit_code": code})
    assert time.monotonic() - started < 10


@pytest.mark.skipif(sys.platform != "win32", reason="a program of Windows")
async def test_an_engine_without_its_libraries_is_said_to_lack_them(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    as_program(monkeypatch, "libraries")

    with pytest.raises(EngineError, match="exit code 0xC0000135 before it listened: the libra"):
        await attached(tmp_path / "film")


@pytest.mark.skipif(sys.platform != "win32", reason="a job of Windows")
async def test_an_engine_that_cannot_leave_the_job_of_this_program_stays_in_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    breakaway = getattr(subprocess, "CREATE_BREAKAWAY_FROM_JOB", 0)
    original = subprocess.Popen
    asked: list[int] = []

    def popen(*args: Any, **more: Any) -> Any:
        asked.append(more["creationflags"])
        if more["creationflags"] & breakaway:
            raise PermissionError(13, "Access is denied")
        return original(*args, **more)

    monkeypatch.setattr(subprocess, "Popen", popen)
    as_program(monkeypatch)

    async with await attached(tmp_path / "film", idle_seconds=1) as connected:
        assert connected.hello["pid"] != os.getpid()
    await until(lambda: not (tmp_path / "film" / "engine.lock").exists())

    assert [bool(flags & breakaway) for flags in asked] == [True, False]


async def test_a_program_that_is_not_there_cannot_be_started(tmp_path: Path) -> None:
    with pytest.raises(EngineError, match="could not be started") as failed:
        await Engine.attach(tmp_path / "film", program=str(tmp_path / "no" / "engine"))

    assert failed.value.code == UNSTARTED


async def test_an_engine_that_drops_the_hello_is_greeted_again(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        fake.drop = "hello"

        async with await attached(tmp_path / "film") as connected:
            assert connected.revision == 0
        assert fake.asked[:3] == ["hello", "hello", "project.get"]


async def test_an_engine_of_another_project_or_protocol_is_not_used(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        fake.project_dir = str(tmp_path / "other")
        with pytest.raises(EngineError, match="another project open") as other:
            await attached(tmp_path / "film")
        fake.project_dir = None
        fake.refusals["hello"] = {
            "code": -32600,
            "message": "no",
            "data": {"reason": "protocol", "supported": [3]},
        }
        with pytest.raises(EngineError) as protocol:
            await attached(tmp_path / "film")
        await until(lambda: not fake.connections)

        assert other.value.code == UNEXPECTED
        assert (protocol.value.code, protocol.value.data) == (
            -32600,
            {"reason": "protocol", "supported": [3]},
        )


@pytest.mark.skipif(sys.platform != "win32", reason="named pipes are of Windows")
def test_a_loop_that_cannot_open_a_named_pipe_is_said(tmp_path: Path) -> None:
    loop = asyncio.SelectorEventLoop()
    try:
        with pytest.raises(EngineError, match="proactor event loop"):
            loop.run_until_complete(attached(tmp_path / "film"))
    finally:
        loop.close()


def test_what_the_engine_is_started_with_and_where_it_listens(tmp_path: Path) -> None:
    lock = tmp_path / "engine.lock"
    good = fake_engine.address_of()
    # The name of an engine's, on another machine or in another directory.
    remote = (
        good.replace("\\\\.\\", "\\\\evil\\", 1)
        if sys.platform == "win32"
        else f"/{Path(good).name}"
    )
    found: list[str | None] = []
    for address in ("", 7, r"\\evil\pipe\x", not_an_engines(), remote, f"{good}0", good):
        lock.write_text(json.dumps({"address": address}), encoding="utf-8")
        found.append(engine._locked(tmp_path))  # pyright: ignore[reportPrivateUsage]
    for written in ("{", "[1]"):
        lock.write_text(written, encoding="utf-8")
        found.append(engine._locked(tmp_path))  # pyright: ignore[reportPrivateUsage]

    assert remote != good
    assert found == [None, None, None, None, None, None, good, None, None]
    assert engine._argv("e", tmp_path, None) == ["e", "--project", str(tmp_path), "--headless"]  # pyright: ignore[reportPrivateUsage]
    assert engine._argv("e", tmp_path, 2)[-2:] == ["--idle-seconds", "2"]  # pyright: ignore[reportPrivateUsage]


def test_the_engine_program_is_never_taken_from_a_working_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    name = "columbina-engine.exe" if sys.platform == "win32" else "columbina-engine"
    trusted, work = tmp_path / "bin", tmp_path / "work"
    for place in (trusted, work):
        place.mkdir()
        (place / name).write_bytes(b"")
        (place / name).chmod(0o755)
    search = os.pathsep.join([str(work), str(trusted)])
    monkeypatch.delenv("COLUMBINA_ENGINE", raising=False)

    assert program_of(None, search, [work]) == str(trusted / name)
    assert program_of(None, str(work), [work]) is None
    assert not_found(None) == NOT_FOUND
    assert program_of(str(trusted / name), "", [work]) == str(trusted / name)
    assert program_of(str(work / name), "", [work]) is None
    monkeypatch.chdir(trusted)
    assert program_of(name, "", []) is None
    assert not_found(name).startswith(f"the config of the video plugin names {name} as the")
    assert program_of(str(trusted / "none"), search, []) is None
    monkeypatch.setenv("COLUMBINA_ENGINE", str(trusted / name))
    assert program_of(None, "", [work]) == str(trusted / name)
    assert program_of("", str(work), []) == str(trusted / name)
    # What the config names goes first, and is not given up for COLUMBINA_ENGINE.
    assert program_of(str(work / name), "", [work]) is None
    assert not_found(None).startswith("COLUMBINA_ENGINE names ")


async def test_an_engine_that_yields_to_the_desktop_is_followed_to_the_desktops(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Were an engine started here, it would say so.
    as_program(monkeypatch, "error")
    async with served(tmp_path / "film") as headless:
        connected = await attached(tmp_path / "film")
        await connected.apply([ADD])
        await connected.apply([ADD])
        changes = connected.changes()
        assert (connected.headless, connected.hello["pid"]) == (True, os.getpid())
        assert connected.revision == 2
    with pytest.raises(EngineError, match="closed the connection"):
        await next_of(changes)
    async with served(tmp_path / "film") as desktop:
        desktop.headless = False
        desktop.change_by("desktop")

        revision, project = await connected.project()
        fetched = await next_of(changes)
        await connected.aclose()

        # The desktop's engine counts its revisions anew, from below those of the other.
        assert (connected.headless, connected.revision) == (False, 1)
        assert (revision, fetched.method, fetched.revision) == (1, "project.fetched", 1)
        assert [one["name"] for one in project["timelines"]] == ["Other"]
        assert desktop.asked == ["hello", "project.get", "project.get"]
        assert headless.address != desktop.address
        desktop.headless = None
        async with await attached(tmp_path / "film") as unsaid:
            assert unsaid.headless is None


async def test_an_engine_that_yielded_is_followed_to_the_one_that_takes_over_later(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Were an engine started here, it would say so.
    as_program(monkeypatch, "error")
    async with served(tmp_path / "film") as first:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        first.tell("engine.yielding", {"to": "desktop"})
        yielding = await next_of(changes)
    with pytest.raises(EngineError, match="closed the connection"):
        await next_of(changes)
    async with contextlib.AsyncExitStack() as later:

        async def taking_over() -> fake_engine.Fake:
            await asyncio.sleep(1)
            desktop = await later.enter_async_context(served(tmp_path / "film"))
            desktop.headless = False
            return desktop

        started = time.monotonic()
        coming = asyncio.create_task(taking_over())
        revision, _ = await connected.project()
        waited = time.monotonic() - started
        desktop = await coming
        fetched = await next_of(changes)
        await connected.aclose()

        assert yielding == Changed("engine.yielding", {"to": "desktop"})
        assert 1 <= waited < 5
        assert (revision, connected.headless, fetched.method) == (0, False, "project.fetched")
        assert desktop.asked == ["hello", "project.get", "project.get"]


async def test_an_engine_that_yielded_to_none_that_comes_is_replaced_after_a_while(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(engine, "_YIELD_SECONDS", 1.0)
    as_program(monkeypatch)
    async with served(tmp_path / "film") as first:
        connected = await attached(tmp_path / "film", idle_seconds=1)
        changes = connected.changes()
        first.tell("engine.yielding", {"to": "desktop"})
        await next_of(changes)
    with pytest.raises(EngineError, match="closed the connection"):
        await next_of(changes)

    started = time.monotonic()
    await connected.project()
    waited = time.monotonic() - started
    await connected.aclose()
    await until(lambda: not (tmp_path / "film" / "engine.lock").exists())

    assert 1 <= waited < 5
    assert connected.hello["pid"] != os.getpid()


def test_an_engine_is_given_the_libraries_of_ffmpeg_dir_and_this_program_is_not(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    where, variable = engine._LIBRARIES.get(sys.platform, ("lib", "LD_LIBRARY_PATH"))  # pyright: ignore[reportPrivateUsage]
    (tmp_path / "built" / where).mkdir(parents=True)
    monkeypatch.delenv("FFMPEG_DIR", raising=False)
    assert engine._environment() is None  # pyright: ignore[reportPrivateUsage]
    monkeypatch.setenv("FFMPEG_DIR", str(tmp_path / "nothing"))
    assert engine._environment() is None  # pyright: ignore[reportPrivateUsage]
    monkeypatch.setenv("FFMPEG_DIR", str(tmp_path / "built"))
    monkeypatch.setenv(variable, "before")
    given = engine._environment()  # pyright: ignore[reportPrivateUsage]
    monkeypatch.delenv(variable)
    alone = engine._environment()  # pyright: ignore[reportPrivateUsage]

    assert given is not None
    assert alone is not None
    assert given[variable] == f"{tmp_path / 'built' / where}{os.pathsep}before"
    assert alone[variable] == str(tmp_path / "built" / where)
    assert variable not in os.environ


async def test_an_engine_that_is_started_finds_the_libraries_of_ffmpeg_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    where, variable = engine._LIBRARIES.get(sys.platform, ("lib", "LD_LIBRARY_PATH"))  # pyright: ignore[reportPrivateUsage]
    libraries = tmp_path / "built" / where
    libraries.mkdir(parents=True)
    monkeypatch.setenv("FFMPEG_DIR", str(tmp_path / "built"))
    before = os.environ.get(variable)
    as_program(monkeypatch)

    async with await attached(tmp_path / "film", idle_seconds=1):
        pass
    await until(lambda: not (tmp_path / "film" / "engine.lock").exists())

    told = [json.loads(line) for line in logs(tmp_path / "film")[-1].read_text().splitlines()]
    (seen,) = [one["environment"][variable] for one in told if "environment" in one]
    assert seen.split(os.pathsep)[0] == str(libraries)
    assert os.environ.get(variable) == before


# The engine itself


@pytest.fixture
def clip(tmp_path: Path) -> Path:
    return made(
        tmp_path / "raw", "clip.mp4",
        "-f", "lavfi", "-i", "color=c=blue:s=160x120:r=25:d=2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p",
    )  # fmt: skip


async def test_the_engine_edits_a_project_for_two_clients_and_ends_by_itself(
    tmp_path: Path, clip: Path
) -> None:
    program = real_engine()
    project = tmp_path / "film"

    first = await Engine.attach(project, program=program, idle_seconds=1)
    assert (project / "engine.lock").exists()
    revision, added = await first.imported([clip])
    timeline = (await first.apply([ADD, edits.add_track("$t", "video")])).names["$t"]
    applied = await first.apply(
        [
            edits.insert(timeline, edits.of_media(added[0], length=25), 0, audio_track=None),
            edits.blade(timeline, 10),
        ]
    )
    _, edited = await first.project()
    frames = await first.duration(timeline)
    second = await Engine.attach(project, program=program, client="cli")
    told = second.changes()
    undone = await first.undo()
    changed = await next_of(told)
    _, after = await second.project()
    with pytest.raises(EngineError) as refused:
        await second.apply([edits.remove_media("m99")])
    with pytest.raises(EngineError) as locked:
        await second.apply(
            [edits.set_track(timeline, "V1", locked=True), edits.blade(timeline, 5, tracks=["V1"])]
        )
    pid = first.hello["pid"]
    await asyncio.gather(first.aclose(), second.aclose())
    await until(lambda: not (project / "engine.lock").exists(), 15)

    assert revision == 1
    assert all("op" in one for one in applied.done)
    assert applied.clamped == []
    clips = edited["timelines"][0]["tracks"][0]["clips"]
    assert [(one["start"], one["length"]) for one in clips] == [(0, 10), (10, 15)]
    assert clips[0]["source"]["media"] == added[0]
    assert frames == 25
    assert second.hello["pid"] == pid
    assert second.hello["protocol"] == 2
    assert undone is not None
    assert changed.revision == undone.revision == 4
    assert (changed.params["by"], changed.params["made_by"]) == ("agent", "agent")
    assert after["timelines"][0]["tracks"][0]["clips"] == []
    assert refused.value.code == -32001
    assert refused.value.data is not None
    assert refused.value.data["reason"] == "missing"
    assert locked.value.data is not None
    assert (locked.value.data["index"], locked.value.data["reason"]) == (1, "locked")
    assert (project / "project.json").exists()
    assert len(logs(project)) == 1


async def test_the_engine_renders_a_timeline_and_tells_how_far_it_has_come(
    tmp_path: Path, clip: Path
) -> None:
    program = real_engine()
    film = tmp_path / "out" / "cut.mov"
    film.parent.mkdir()

    async with await Engine.attach(tmp_path / "film", program=program, idle_seconds=1) as one:
        _, added = await one.imported([clip])
        timeline = (await one.apply([ADD, edits.add_track("$t", "video")])).names["$t"]
        await one.apply([edits.append(timeline, edits.of_media(added[0], length=25))])
        heard: list[Progress] = []
        # Held to a revision, it is refused where the project is not at it, and taken where it is.
        revision, _ = await one.project()
        with pytest.raises(EngineError) as stale:
            await one.render(timeline, film, codec="prores", quality="small", revision=revision + 1)
        rendering = await one.render(
            timeline, film, codec="prores", quality="small", revision=revision
        )
        ended = await asyncio.wait_for(rendering.finished(heard.append), 120)

    assert stale.value.data == {
        "reason": "stale_revision",
        "expected": revision + 1,
        "actual": revision,
    }
    assert (ended.state, ended.frames, ended.path) == ("done", 25, str(film)), ended.message
    assert ended.encoder
    assert all(one.job == rendering.job for one in heard)
    assert film.stat().st_size > 0


async def test_the_engine_renders_the_sound_alone_normalized_as_it_is_asked(
    tmp_path: Path,
) -> None:
    program = real_engine()
    tone = made(
        tmp_path / "raw", "tone.wav",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2",
        "-af", "volume=-30dB", "-c:a", "pcm_s16le",
    )  # fmt: skip
    sound = tmp_path / "out" / "mix.wav"
    sound.parent.mkdir()

    async with await Engine.attach(tmp_path / "film", program=program, idle_seconds=1) as one:
        _, added = await one.imported([tone])
        timeline = (await one.apply([ADD, edits.add_track("$t", "audio")])).names["$t"]
        await one.apply([edits.append(timeline, edits.of_media(added[0], length=25))])
        asked = RenderAudio(codec="pcm_f32le", normalize=Normalization("ebu-r128"))
        rendering = await one.render(timeline, sound, audio=asked, video=False)
        ended = await asyncio.wait_for(rendering.finished(), 120)

    assert (ended.state, ended.frames, ended.path) == ("done", 25, str(sound)), ended.message
    assert ended.normalization is not None
    assert (ended.normalization["standard"], ended.normalization["mode"]) == (
        "ebu-r128",
        "normalize",
    )
    # A sine at -30 dBFS is brought up towards -23 LUFS.
    assert ended.normalization["gain_db"] > 0
    assert sound.read_bytes()[:4] == b"RIFF"


async def test_the_engine_takes_the_sound_of_a_timeline_as_it_is_written(tmp_path: Path) -> None:
    program = real_engine()
    second = edits.Time(ticks=48000, scale=48000)

    async with await Engine.attach(tmp_path / "film", program=program, idle_seconds=1) as one:
        applied = await one.apply(
            [ADD, edits.add_track("$t", "audio"), edits.add_track("$t", "audio")]
        )
        timeline = applied.names["$t"]
        _, before = await one.project()
        every = [
            edits.set_track_audio(timeline, "A1", {"strip": {"fader_db": -6.0}}),
            edits.set_track(timeline, "A2", mute=True),
            edits.add_bus(
                timeline, {"id": "B2", "name": "Dialogue", "color": "#ff8800ff", "format": "stereo"}
            ),
            edits.set_bus(timeline, "B2", {"strip": {"fader_db": -3.0}}),
            edits.add_insert(timeline, {"track": "A1"}, {"effect": {"kind": "gain"}}),
            edits.set_insert(timeline, {"track": "A1"}, 0, None),
            edits.add_vca(timeline, {"id": "", "name": "Music"}),
            edits.add_track_group(timeline, {"id": "", "name": "Dialogue", "members": ["A1"]}),
            edits.lane_points(
                timeline, {"track": "A1"}, "fader_db", "add", points=[(second, -10.0)]
            ),
            edits.set_lane(
                timeline,
                {"track": "A1"},
                "pan.x",
                {"points": [{"at": second.model_dump(), "value": 0.5}]},
            ),
            edits.lane_range(
                timeline,
                {"track": "A1"},
                "pan.x",
                edits.Time(ticks=0, scale=48000),
                edits.Time(ticks=96000, scale=48000),
                "shift_value",
                delta=0.1,
            ),
            edits.clear_lanes(timeline, {"track": "A1"}),
            edits.set_timeline_audio(timeline, {"follows_edit": False}),
        ]
        await one.apply(every)
        _, after = await one.project()
        await one.undo()
        _, undone = await one.project()

    (cut,) = after["timelines"]
    a1, a2 = cut["tracks"]
    assert a1["strip"]["fader_db"] == -6.0
    assert (a1["strip"]["inserts"], a1["lanes"]) == ([], {})
    assert a2["mute"] is True
    assert [bus["id"] for bus in cut["buses"]] == ["B1", "B2"]
    assert cut["buses"][1]["strip"]["fader_db"] == -3.0
    assert [vca["id"] for vca in cut["vcas"]] == ["VCA1"]
    assert cut["track_groups"][0]["members"] == ["A1"]
    assert cut["audio"]["follows_edit"] is False
    # One step, taken back whole.
    assert undone["timelines"] == before["timelines"]


async def test_the_engine_bakes_a_grade_into_a_lut_of_the_project(tmp_path: Path) -> None:
    program = real_engine()

    async with await Engine.attach(tmp_path / "film", program=program, idle_seconds=1) as one:
        timeline = (await one.apply([ADD])).names["$t"]
        grade = {"level": "timeline", "timeline": timeline}
        baked = await one.export_lut(grade, "flat.cube", size=17)
        with pytest.raises(EngineError) as there:
            await one.export_lut(grade, "flat.cube", size=17)
        with pytest.raises(EngineError) as size:
            await one.export_lut(grade, "odd.cube", size=18)  # type: ignore[arg-type]

    assert (baked["size"], baked["nodes"], baked["ignored"]) == (17, [], [])
    written = Path(baked["path"])
    assert written.is_file()
    assert "LUT_3D_SIZE 17" in written.read_text(encoding="utf-8")
    assert there.value.code == -32001
    assert size.value.code == -32602


async def test_the_engine_lists_the_luts_of_the_project_and_has_no_scopes_headless(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    program = real_engine()
    project = tmp_path / "film"
    library = tmp_path / "library"
    cube = "LUT_3D_SIZE 2\n" + "0 0 0\n" * 8
    (project / "LUTs").mkdir(parents=True)
    (project / "LUTs" / "film.cube").write_text(cube, newline="\n")
    (library / "looks").mkdir(parents=True)
    (library / "looks" / "warm.cube").write_text(cube, newline="\n")
    monkeypatch.setenv("COLUMBINA_LUTS", str(library))

    async with await Engine.attach(project, program=program, idle_seconds=1) as one:
        listed = await one.luts()
        with pytest.raises(EngineError) as headless:
            await one.statistics()
        with pytest.raises(EngineError) as outside:
            await one.luts("..")

    assert [root.place for root in listed.roots] == ["project", "library"]
    assert Path(listed.roots[1].path) == library
    assert [(lut.name, lut.root, lut.size) for lut in listed.luts] == [
        ("LUTs/film.cube", 0, len(cube)),
        ("looks/warm.cube", 1, len(cube)),
    ]
    assert headless.value.code == -32601
    assert outside.value.data == {"reason": "outside_roots", "folder": ".."}
