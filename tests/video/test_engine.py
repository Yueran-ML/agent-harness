"""The agent reaches the engine that has a project open, as one client among others."""

import asyncio
import json
import os
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from columbina.video import engine
from columbina.video.engine import (
    UNEXPECTED,
    UNREACHABLE,
    UNSTARTED,
    Applied,
    Changed,
    Changes,
    Engine,
    EngineError,
    program_of,
)
from support import fake_engine
from support.fake_engine import LIMIT, served
from support.media import made, real_engine

FAKE = Path(fake_engine.__file__)
TIMELINE: dict[str, Any] = {"id": "$t", "name": "Cut", "rate": {"num": 25, "den": 1}, "tracks": []}


def as_program(monkeypatch: pytest.MonkeyPatch, *mode: str) -> None:
    """Start the fake engine as the program, in a mode: run by this Python, as it is a script."""

    def argv(program: str, directory: Path, idle_seconds: float | None) -> list[str]:
        idle = ["--idle-seconds", str(idle_seconds)] if idle_seconds is not None else []
        return [sys.executable, str(FAKE), "--project", str(directory), "--headless", *idle,
                "--mode", *(mode or ("listen",))]  # fmt: skip

    monkeypatch.setattr(engine, "_argv", argv)


async def attached(directory: Path, **more: Any) -> Engine:
    return await Engine.attach(directory, program=str(FAKE), **more)


async def until(done: Callable[[], bool], seconds: float = 10.0) -> None:
    deadline = time.monotonic() + seconds
    while not done():
        assert time.monotonic() < deadline, "it did not happen in time"
        await asyncio.sleep(0.02)


async def next_of(changes: Changes) -> Changed:
    return await asyncio.wait_for(anext(changes), 5)


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
            connected.apply([{"op": "add_timeline", "timeline": TIMELINE}]),
            connected.project(),
            connected.imported([FAKE]),
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

        applied = await one.apply([{"op": "add_timeline", "timeline": TIMELINE}])
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
        assert other.revision == 3


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


async def test_answers_that_are_not_of_the_protocol_are_refused(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.results = {
            "project.get": {"revision": 0},
            "transaction.apply": {"revision": 1, "done": "x", "next_id": 2, "names": {}},
            "media.import": {"revision": 1, "added": [1]},
            "history.undo": {"revision": "x", "done": None},
            "history.redo": None,
        }

        for call in (
            connected.project(),
            connected.apply([{"op": "add_timeline", "timeline": TIMELINE}]),
            connected.imported([FAKE]),
            connected.undo(),
            connected.redo(),
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

        await asyncio.gather(
            connected.apply([{"op": "add_timeline", "timeline": TIMELINE}]),
            connected.imported([FAKE]),
        )
        fetched = await next_of(changes)
        await connected.apply([{"op": "remove_timeline", "id": "t1"}])
        after = await next_of(changes)

        assert fetched.method == "project.fetched"
        assert fetched.revision == 3
        assert [one["id"] for one in fetched.params["project"]["media"]] == ["m2"]
        assert after.revision == 4
        assert (connected.revision, connected.next_id) == (4, 3)
        assert fake.asked.count("project.get") == 2


async def test_a_lost_connection_fails_what_waits_and_is_made_again(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        fake.drop = "transaction.apply"

        with pytest.raises(EngineError, match="closed the connection") as lost:
            await connected.apply([{"op": "add_timeline", "timeline": TIMELINE}])
        with pytest.raises(EngineError):
            await next_of(changes)
        revision, _ = await connected.project()
        fetched = await next_of(changes)

        assert lost.value.code == UNREACHABLE
        assert revision == 0
        assert fetched.method == "project.fetched"
        assert fake.asked[-3:] == ["hello", "project.get", "project.get"]
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


async def test_a_line_longer_than_16_mib_ends_the_connection(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        connected = await attached(tmp_path / "film")
        fake.huge = "project.get"

        with pytest.raises(EngineError, match="longer than 16 MiB") as huge:
            await connected.project()

        assert huge.value.code == UNREACHABLE
        assert (await connected.project())[0] == 0
        await connected.aclose()


@pytest.mark.parametrize("parts", [[b"x" * (LIMIT + 1) + b"\n{}\n"], [b"x" * LIMIT, b"xx"]])
def test_a_line_too_long_is_not_read_and_nothing_is_heard_after_the_end(
    parts: list[bytes],
) -> None:
    heard: list[bytes] = []
    lost: list[str] = []
    wire = engine._Wire(lambda _, line: heard.append(line), lambda _, why: lost.append(why))  # pyright: ignore[reportPrivateUsage]

    class Transport(asyncio.Transport):
        def close(self) -> None:
            wire.connection_lost(None)

    wire.connection_made(Transport())
    wire.data_received(b'{"a": 1}\n')
    for part in parts:
        wire.data_received(part)
    wire.data_received(b"{}\n")

    assert heard == [b'{"a": 1}']
    assert lost == ["the engine sent a line longer than 16 MiB"]


# Reaching the engine


async def test_an_address_where_nobody_listens_is_looked_up_again_and_an_engine_started(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    project = tmp_path / "film"
    project.mkdir()
    (project / "engine.lock").write_text(
        json.dumps({"pid": 1, "address": fake_engine.address_of()}), encoding="utf-8"
    )
    as_program(monkeypatch)

    connected = await attached(project, idle_seconds=0.3)
    pid = connected.hello["pid"]
    locked = json.loads((project / "engine.lock").read_text(encoding="utf-8"))
    await connected.aclose()
    await until(lambda: not (project / "engine.lock").exists())
    log = project / "cache" / f"engine-{pid}.log"
    await until(lambda: '"connected"' in log.read_text(encoding="utf-8"))

    assert pid != os.getpid()
    assert locked["pid"] == pid
    assert '"listening"' in log.read_text(encoding="utf-8")


async def test_a_log_that_cannot_be_kept_is_passed_over(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    project = tmp_path / "film"
    project.mkdir()
    (project / "cache").write_text("a file", encoding="utf-8")
    as_program(monkeypatch)

    async with await attached(project, idle_seconds=0.3) as connected:
        assert connected.hello["pid"] != os.getpid()
    await until(lambda: not (project / "engine.lock").exists())


async def test_an_engine_that_finds_the_project_held_leads_to_the_one_that_holds_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
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

    async with await attached(tmp_path / "film", idle_seconds=0.3) as connected:
        assert (tmp_path / "film" / "contended").exists()
        assert connected.hello["pid"] != os.getpid()


@pytest.mark.parametrize(
    ("mode", "said"),
    [
        (("held",), "the engine that has it open, at None, did not answer"),
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
        ("babble", "did not say where it listens in protocol 1: b'hello there", None),
        ("silent", "did not say where it listens within 0.5 seconds", None),
    ],
)
async def test_an_engine_that_cannot_start_says_why(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str, said: str, code: int | None
) -> None:
    as_program(monkeypatch, mode)
    monkeypatch.setattr(engine, "_LISTENING_SECONDS", 0.5)

    with pytest.raises(EngineError, match=said) as failed:
        await attached(tmp_path / "film")

    assert failed.value.code == UNSTARTED
    assert failed.value.data == (None if code is None else {"exit_code": code})


@pytest.mark.skipif(sys.platform != "win32", reason="a program of Windows")
async def test_an_engine_without_its_libraries_is_said_to_lack_them(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    as_program(monkeypatch, "libraries")

    with pytest.raises(EngineError, match="libraries of FFmpeg"):
        await attached(tmp_path / "film")


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
        fake.refusals["hello"] = {"code": -32602, "message": "no", "data": {"protocol": 2}}
        with pytest.raises(EngineError) as protocol:
            await attached(tmp_path / "film")
        await until(lambda: not fake.connections)

        assert other.value.code == UNEXPECTED
        assert (protocol.value.code, protocol.value.data) == (-32602, {"protocol": 2})


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
    found: list[str | None] = []
    for written in ("{", "[1]", '{"address": ""}', '{"address": 7}', '{"address": "here"}'):
        lock.write_text(written, encoding="utf-8")
        found.append(engine._locked(tmp_path))  # pyright: ignore[reportPrivateUsage]

    assert found == [None, None, None, None, "here"]
    assert engine._argv("e", tmp_path, None) == ["e", "--project", str(tmp_path), "--headless"]  # pyright: ignore[reportPrivateUsage]
    assert engine._argv("e", tmp_path, 1.5)[-2:] == ["--idle-seconds", "1.5"]  # pyright: ignore[reportPrivateUsage]


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
    assert program_of(str(trusted / name), "", [work]) == str(trusted / name)
    assert program_of(str(work / name), "", [work]) is None
    assert program_of(name, "", []) is None
    assert program_of(str(trusted / "none"), search, []) is None
    monkeypatch.setenv("COLUMBINA_ENGINE", str(trusted / name))
    assert program_of(None, "", [work]) == str(trusted / name)
    assert program_of("", str(work), []) == str(trusted / name)


# The engine itself


@pytest.fixture
def clip(tmp_path: Path) -> Path:
    return made(
        tmp_path / "raw", "clip.mp4",
        "-f", "lavfi", "-i", "color=c=blue:s=160x120:r=25:d=2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p",
    )  # fmt: skip


async def test_the_engine_edits_a_project_for_two_clients_and_ends_by_itself(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, clip: Path
) -> None:
    program = real_engine(monkeypatch)
    project = tmp_path / "film"

    first = await Engine.attach(project, program=program, idle_seconds=1)
    assert (project / "engine.lock").exists()
    revision, added = await first.imported([clip])
    _, imported = await first.project()
    scale = imported["media"][0]["facts"]["duration"]["scale"]
    applied = await first.apply(
        [
            {"op": "add_timeline", "timeline": TIMELINE},
            {"op": "add_track", "timeline": "$t",
             "track": {"id": "$v", "kind": "video", "clips": []}},
            {"op": "insert_clip", "timeline": "$t", "track": "$v",
             "clip": {"id": "$c", "media": added[0], "source_in": {"ticks": 0, "scale": scale},
                      "start": 0, "length": 25}},
        ]
    )  # fmt: skip
    _, edited = await first.project()
    second = await Engine.attach(project, program=program, client="cli")
    told = second.changes()
    undone = await first.undo()
    changed = await next_of(told)
    _, after = await second.project()
    with pytest.raises(EngineError) as refused:
        await second.apply([{"op": "remove_media", "id": "m99"}])
    pid = first.hello["pid"]
    await asyncio.gather(first.aclose(), second.aclose())
    await until(lambda: not (project / "engine.lock").exists(), 15)

    assert revision == 1
    assert set(applied.names) == {"$t", "$v", "$c"}
    assert edited["timelines"][0]["tracks"][0]["clips"][0]["media"] == added[0]
    assert second.hello["pid"] == pid
    assert undone is not None
    assert changed.revision == undone.revision == 3
    assert after["timelines"] == []
    assert refused.value.code == -32001
    assert refused.value.data is not None
    assert refused.value.data["reason"] == "missing"
    assert (project / "project.json").exists()
    assert (project / "cache" / f"engine-{pid}.log").exists()
