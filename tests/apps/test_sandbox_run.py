"""The end of stage 7: the commands of a task run in a container, and cannot reach the network.

These go through the command line, ``columbina run``, with a model that is
scripted and a real engine. What the provider does with a container is tried
piece by piece in ``tests/sandbox``. Here it is the whole program: the tool
that the model is offered, the permissions that let a held command through,
the container, and the result that the model reads.

They need the images of :mod:`support.engines` and are passed over without an
engine. One test needs none: with something in the place of the docker
program that is no engine, no command is run anywhere.
"""

import os
import sys
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from support.engines import BARE, IMAGE, containers, left_behind, real
from support.launch import lines, profile

pytestmark = pytest.mark.timeout(300)

WINDOWS = sys.platform == "win32"
NOBODY_TO_ASK = "APPROVAL_UNAVAILABLE"
SLEEPERS = "grep -lx sleep /proc/[0-9]*/comm 2>/dev/null | wc -l"
"""Counts the processes of a container that are a ``sleep``, by the name of their program."""


def call(name: str, **arguments: object) -> dict[str, Any]:
    """A reply of the model that calls one tool."""
    return {"calls": [{"name": name, "arguments": arguments}]}


def bash(command: str, **arguments: object) -> dict[str, Any]:
    return call("bash", command=command, **arguments)


@pytest.fixture(scope="session")
def engine() -> str:
    return real()


@dataclass(frozen=True)
class Held:
    """``columbina run`` with the commands of the task in a container."""

    work: Path
    tmp_path: Path
    capsys: pytest.CaptureFixture[str]
    program: str

    def run(
        self,
        *replies: dict[str, Any],
        flags: tuple[str, ...] = (),
        docker: dict[str, Any] | None = None,
        extra: list[dict[str, Any]] | None = None,
    ) -> list[tuple[str, str | None]]:
        """Run a task with these replies of the model, and return the result of each call."""
        config = {"program": self.program, "image": IMAGE, **(docker or {})}
        sandbox = {"id": "sandbox", "config": {"provider": "docker", "docker": config}}
        chosen = profile(
            self.tmp_path, *replies, {"text": "Done."}, extra=[sandbox, *(extra or [])]
        )
        code = main(
            ["run", "Do it.", "--profile", chosen, "--cwd", str(self.work), "--json", *flags]
        )
        captured = self.capsys.readouterr()
        assert code == 0, captured.err
        return [
            (line["text"], line["error"])
            for line in lines(captured.out)
            if line["type"] == "tool_result"
        ]


@pytest.fixture
def held(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str], engine: str
) -> Iterator[Held]:
    before = containers(engine)
    yield Held(work, tmp_path, capsys, engine)
    # When the program has ended, none of its containers is left. One that is left is removed
    # before the test fails for it.
    assert not left_behind(engine, before, tmp_path)


def said(result: tuple[str, str | None]) -> list[str]:
    """The lines that a command wrote, without the line about how it ended."""
    text, error = result
    assert error is None, text
    return text.splitlines()[:-1]


# 1. A command reaches the working directory and nothing else


def test_what_a_command_writes_in_the_working_directory_is_on_this_machine(held: Held) -> None:
    (result,) = held.run(bash("cat notes.txt; echo made > made.txt; echo $?"))

    assert said(result) == ["The answer is 42.", "0"]
    assert (held.work / "made.txt").read_text(encoding="utf-8") == "made\n"


def test_what_a_command_writes_outside_the_working_directory_changes_nothing_here(
    held: Held,
) -> None:
    beside = held.work.parent / "beside.txt"
    beside.write_text("as it was\n", encoding="utf-8")

    (result,) = held.run(
        bash(
            "echo x 2>/dev/null > ../beside.txt; echo $?; echo x 2>/dev/null > /etc/planted; "
            "echo $?; echo kept > /tmp/kept && cat /tmp/kept; ls .. | wc -l"
        )
    )

    # Above the working directory the container has a directory of its own, with nothing of
    # this machine in it. A command is no root, so it writes where anybody may, and nowhere
    # else.
    assert said(result)[:3] == ["1", "1", "kept"]
    assert beside.read_text(encoding="utf-8") == "as it was\n"
    # Beside the working directory lie what this test put there and what the program keeps.
    assert sorted(path.name for path in held.work.parent.iterdir()) == [
        "beside.txt",
        "home",
        "profile.yml",
        "work",
    ]


def test_a_session_that_is_read_only_cannot_write_the_working_directory(held: Held) -> None:
    (result,) = held.run(
        bash("cat notes.txt; echo x 2>/dev/null > made.txt; echo $?"),
        flags=("--permission", "read-only"),
    )

    assert said(result) == ["The answer is 42.", "1"]
    assert sorted(path.name for path in held.work.iterdir()) == ["notes.txt"]


def test_what_belongs_to_the_repository_cannot_be_written(held: Held) -> None:
    (held.work / ".git").mkdir()
    (held.work / ".git" / "config").write_text("[core]\n", encoding="utf-8")

    (result,) = held.run(
        bash("echo x 2>/dev/null >> .git/config; echo $?; echo x 2>/dev/null > .git/hook; echo $?")
    )

    assert said(result) == ["1", "1"]
    assert (held.work / ".git" / "config").read_text(encoding="utf-8") == "[core]\n"
    assert [path.name for path in (held.work / ".git").iterdir()] == ["config"]


def test_a_file_that_a_command_made_is_read_by_the_path_that_the_container_has_it_at(
    held: Held,
) -> None:
    # A model reads that path in what its commands write, and hands it to the file tools.
    there = "/workspace" if WINDOWS else Path(os.path.realpath(held.work)).as_posix()

    first, second = held.run(
        bash("echo from the container > made.txt; pwd"),
        call("read", path=f"{there}/made.txt"),
    )

    assert said(first) == [there]
    assert second[1] is None
    assert "from the container" in second[0]


# 2. No network


def test_a_command_reaches_no_network(held: Held) -> None:
    (result,) = held.run(bash('(exec 3<>/dev/tcp/1.1.1.1/53) 2>&1; echo "status $?"'))

    wrote = said(result)
    assert any("Network is unreachable" in line for line in wrote)
    assert wrote[-1] == "status 1"


# 3. A command that is held is not asked about


def test_a_command_runs_although_nobody_is_there_to_ask(held: Held) -> None:
    (result,) = held.run(bash("echo held"))

    assert said(result) == ["held"]


def test_a_rule_that_asks_still_asks(held: Held) -> None:
    asking = {"id": "permissions", "config": {"preset": "workspace-write", "ask": ["run(rm *)"]}}

    kept, asked = held.run(bash("echo held"), bash("rm notes.txt"), extra=[asking])

    assert said(kept) == ["held"]
    assert asked[1] == NOBODY_TO_ASK
    assert (held.work / "notes.txt").exists()


def test_a_command_in_a_container_with_a_network_is_asked_about(held: Held) -> None:
    (result,) = held.run(bash("echo x > made.txt"), docker={"network": "bridge"})

    assert result[1] == NOBODY_TO_ASK
    assert not (held.work / "made.txt").exists()


# 4. What a command started is stopped


def test_a_command_whose_time_is_up_is_stopped_with_all_that_it_started(held: Held) -> None:
    stopped, after = held.run(
        bash("setsid sleep 300 & sleep 300 & sleep 300", timeout_seconds=2),
        bash(SLEEPERS),
    )

    assert stopped == ("[the command was stopped after 2 seconds]", None)
    assert said(after) == ["0"]


# 5. Without an engine no command runs, here or anywhere


def test_an_image_that_is_not_on_this_machine_is_said_and_nothing_is_run(held: Held) -> None:
    (result,) = held.run(bash("echo x > made.txt"), docker={"image": "columbina-no-such-image:1"})

    text, error = result
    assert error == "SANDBOX_UNAVAILABLE"
    assert "docker pull columbina-no-such-image:1" in text
    assert not (held.work / "made.txt").exists()


def test_an_image_without_bash_is_said_and_nothing_is_run(held: Held) -> None:
    (result,) = held.run(bash("echo x > made.txt"), docker={"image": BARE})

    text, error = result
    assert error == "SANDBOX_UNAVAILABLE"
    assert f"the image {BARE} has no bash" in text
    assert not (held.work / "made.txt").exists()


def test_with_no_engine_behind_the_docker_program_no_command_runs_on_this_machine(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Python stands in the place of the docker program: it starts, and it is no engine.
    # Whatever the command would do on this machine, it must not be run here instead.
    marker = work / "ran-here.txt"
    # A program in the directory that this one was started in would not be run at all.
    (tmp_path / "started").mkdir()
    monkeypatch.chdir(tmp_path / "started")
    program = Held(work, tmp_path, capsys, sys.executable)

    (result,) = program.run(bash("echo x > ran-here.txt"))

    text, error = result
    assert error == "SANDBOX_UNAVAILABLE"
    assert text.startswith("The command was not run: ")
    assert not marker.exists()
