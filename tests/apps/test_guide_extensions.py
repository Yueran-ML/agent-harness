"""What the guide shows of skills, hooks, subagents and servers is what the program takes.

The guide gives an example of each of the four. Each example is taken out of the guide as
it stands there and handed to the code that reads such a thing, so that an example cannot
fall behind the program. The other two languages show the same blocks, which
``tests/test_examples.py`` sees to.
"""

import re
from pathlib import Path

import pytest
import yaml

from columbina.apps.launcher import main
from columbina.hooks import config as hook_files
from columbina.kernel.compose import interpolate
from columbina.mcp.server import Config as Server
from columbina.places import Root
from columbina.skills import files as skill_files
from columbina.subagents import types as kinds

GUIDE = (Path(__file__).parents[2] / "docs" / "guide.md").read_text(encoding="utf-8")
FENCE = "`" * 3


def shown(language: str, beginning: str) -> str:
    """Return the one block of the guide in this language that begins with this text."""
    blocks = re.findall(
        rf"^{FENCE}{language}\n(.*?)^{FENCE}$", GUIDE, flags=re.DOTALL | re.MULTILINE
    )
    (found,) = (block for block in blocks if block.startswith(beginning))
    return found


def test_the_skill_that_the_guide_shows_is_a_skill(tmp_path: Path) -> None:
    root = Root(tmp_path / "skills", "home", own=True)
    directory = root.path / "release-notes"
    directory.mkdir(parents=True)
    written = shown("markdown", "---\nname: release-notes\n")
    (directory / "SKILL.md").write_text(written, encoding="utf-8", newline="\n")

    found = skill_files.scan(root)

    assert found.invalid == ()
    (skill,) = found.skills
    assert (skill.name, skill.description) == (
        "release-notes",
        "Writes the notes of a release in the form that this project uses.",
    )
    assert (skill.model_invocable, skill.user_invocable) == (True, True)
    # What follows the first lines is what a model reads, and it names the file beside it.
    assert skill_files.body(skill)[1].endswith("\nThe codenames are in codenames.txt.")


def test_the_hook_file_that_the_guide_shows_is_read_without_a_warning() -> None:
    hooks = hook_files.parsed(shown("json", '{"hooks"').encode())

    assert hooks.warnings == ()
    assert list(hooks.groups) == ["PostToolUse"]
    (group,) = hooks.groups["PostToolUse"]
    assert group.hooks == (hook_files.Hook("ruff format", 30),)
    # Its matcher names tools as Claude Code calls them, and means the tools of this program.
    for tool, meant in (("edit", True), ("write", True), ("read", False)):
        assert hook_files.matches(group.matcher, hook_files.names_of(tool)) is meant


def test_the_kind_of_subagent_that_the_guide_shows_is_a_kind(tmp_path: Path) -> None:
    root = Root(tmp_path / "agents", "home", own=True)
    root.path.mkdir()
    written = shown("markdown", "---\nname: reviewer\n")
    (root.path / "reviewer.md").write_text(written, encoding="utf-8", newline="\n")

    found = kinds.scan(root)

    assert found.invalid == ()
    (kind,) = found.types
    assert (kind.name, kind.tools, kind.max_steps, kind.model) == (
        "reviewer",
        ("read", "glob", "grep"),
        30,
        None,
    )
    assert kind.description == "Reads a change and says what is wrong with it. Changes nothing."
    assert kind.prompt is not None
    assert kind.prompt.strip() == "You review a change that somebody else made."
    # The tools that it names are tools of the program.
    assert kinds.usable(found, {"read", "write", "edit", "glob", "grep"}).invalid == ()


def test_the_servers_that_the_guide_shows_are_entries_that_the_program_takes(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = tmp_path / "config.yml"
    patch.write_text(shown("yaml", "- insert:\n"), encoding="utf-8", newline="\n")

    # The entries that would be loaded are printed, and nothing is started.
    code = main(["run", "A task.", "--patch", str(patch), "--cwd", str(tmp_path), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    files, docs = entries["mcp-files"], entries["mcp-docs"]
    assert (files["plugin"], docs["plugin"]) == ("columbina.mcp.server", "columbina.mcp.server")
    started = Server.model_validate(interpolate(files["config"], {}))
    reached = Server.model_validate(interpolate(docs["config"], {"DOCS_TOKEN": "made-up"}))
    assert (started.server, started.command, started.url) == ("files", "npx", None)
    assert started.args == ("-y", "@modelcontextprotocol/server-filesystem", ".")
    assert (reached.server, reached.command, reached.url) == (
        "docs",
        None,
        "https://example.com/mcp",
    )
    # The token is no part of the file. It comes from the environment when the program starts.
    assert reached.headers == {"Authorization": "Bearer made-up"}
