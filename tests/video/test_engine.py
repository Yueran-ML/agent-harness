"""The agent reaches the engine that has a project open, as one client among others."""

import asyncio
import json
import os
import subprocess
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from columbina.video import engine
from columbina.video.engine import (
    NOT_FOUND,
    UNEXPECTED,
    UNREACHABLE,
    UNSTARTED,
    Applied,
    Changed,
    Changes,
    Engine,
    EngineError,
    not_found,
    program_of,
)
from support import fake_engine
from support.fake_engine import LIMIT, served
from support.media import made, real_engine

FAKE = Path(fake_engine.__file__)
TIMELINE: dict[str, Any] = {"id": "$t", "name": "Cut", "rate": {"num": 25, "den": 1}, "tracks": []}
ADD = {"op": "add_timeline", "timeline": TIMELINE}


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
        revision, _ = await connected.project()
        await connected.aclose()

        assert long.value.code == -32600
        assert long.value.data is not None
        assert long.value.data["bytes"] > LIMIT
        assert nan.value.code == -32602
        assert unread.value.code == -32600
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
        }

        for call in (
            connected.project(),
            connected.apply([ADD]),
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
    wire.data_received(b"{}\n")

    assert heard == [b'{"a": 1}']
    assert [one.code for one in lost] == [UNEXPECTED]
    assert "the project is too large to be read here" in str(lost[0])


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
        ("babble", "did not say where it listens in protocol 1, at an add", None),
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
    good = fake_engine.address_of()
    found: list[str | None] = []
    for address in ("", 7, r"\\evil\pipe\x", not_an_engines(), good):
        lock.write_text(json.dumps({"address": address}), encoding="utf-8")
        found.append(engine._locked(tmp_path))  # pyright: ignore[reportPrivateUsage]
    for written in ("{", "[1]"):
        lock.write_text(written, encoding="utf-8")
        found.append(engine._locked(tmp_path))  # pyright: ignore[reportPrivateUsage]

    assert found == [None, None, None, None, good, None, None]
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
    assert program_of(name, "", []) is None
    assert not_found(name).startswith(f"the config of the video plugin names {name} as the")
    assert program_of(str(trusted / "none"), search, []) is None
    monkeypatch.setenv("COLUMBINA_ENGINE", str(trusted / name))
    assert program_of(None, "", [work]) == str(trusted / name)
    assert program_of("", str(work), []) == str(trusted / name)
    assert not_found(None).startswith("COLUMBINA_ENGINE names ")


async def test_an_engine_that_yields_to_the_desktop_is_followed_to_the_desktops(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Were an engine started here, it would say so.
    as_program(monkeypatch, "error")
    async with served(tmp_path / "film") as headless:
        connected = await attached(tmp_path / "film")
        changes = connected.changes()
        assert (connected.headless, connected.hello["pid"]) == (True, os.getpid())
    with pytest.raises(EngineError, match="closed the connection"):
        await next_of(changes)
    async with served(tmp_path / "film") as desktop:
        desktop.headless = False
        desktop.change_by("desktop")

        revision, project = await connected.project()
        fetched = await next_of(changes)
        await connected.aclose()

        assert connected.headless is False
        assert (revision, fetched.method, fetched.revision) == (1, "project.fetched", 1)
        assert [one["name"] for one in project["timelines"]] == ["Other"]
        assert desktop.asked == ["hello", "project.get", "project.get"]
        assert headless.address != desktop.address
        desktop.headless = None
        async with await attached(tmp_path / "film") as unsaid:
            assert unsaid.headless is None


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
    assert given[variable] == f"before{os.pathsep}{tmp_path / 'built' / where}"
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
    assert seen.split(os.pathsep)[-1] == str(libraries)
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
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, clip: Path
) -> None:
    program = real_engine()
    project = tmp_path / "film"

    first = await Engine.attach(project, program=program, idle_seconds=1)
    assert (project / "engine.lock").exists()
    revision, added = await first.imported([clip])
    _, imported = await first.project()
    scale = imported["media"][0]["facts"]["duration"]["scale"]
    applied = await first.apply(
        [
            ADD,
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
    assert (changed.params["by"], changed.params["made_by"]) == ("agent", "agent")
    assert after["timelines"] == []
    assert refused.value.code == -32001
    assert refused.value.data is not None
    assert refused.value.data["reason"] == "missing"
    assert (project / "project.json").exists()
    assert len(logs(project)) == 1
