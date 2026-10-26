"""A session as a page to read or to hand on."""

from pathlib import Path

import columbina.commands
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import export as export_command
from columbina.export import RESULT_MOST, About, exported
from columbina.llm.adapters.scripted import Reply
from columbina.session import (
    EventData,
    SessionEvent,
    TurnStartEvent,
)
from support.loop import Harness, idle
from support.told_sessions import HAPPENED

ABOUT = About("s-1", 1_800_000_000_000, "/work")


def events() -> list[SessionEvent[EventData]]:
    return [SessionEvent(seq, 1_800_000_000_000 + seq, data) for seq, data in enumerate(HAPPENED)]


def test_markdown_tells_what_was_said_and_done() -> None:
    page = exported(ABOUT, events(), "md")

    assert page.startswith("# Session s-1\n\n- Started: ")
    assert "- Working directory: `/work`\n- Model: deepseek/deepseek-flash\n" in page
    assert "Be brief." not in page
    assert "## Turn 1\n\n**You:**\n\nFix <b>calc.py</b> & say why.\n" in page
    assert (
        '**Columbina:**\n\nI will read it.\n\nCall of `read`:\n\n```\n{\n "path": "calc.py"\n}\n```'
        in page
    )
    assert "Call of `shell`:\n\n````\nnot json ```\n````" in page
    assert "It gave back:\n\n```\n1\tdef mean(xs):\n```" in page
    assert f"It failed:\n\n```\n{'x' * RESULT_MOST}\n… (5 more characters)\n```" in page
    assert "**You, on the way:**\n\nKeep it small." in page
    assert "**Summary of the earlier part:**\n\nEarlier, calc.py was fixed." in page
    assert "*The turn ended early: error: AUTH: no key*" in page
    assert "*The turn ended early: aborted*" in page
    assert page.count("**Columbina:**") == 2


def test_html_stands_alone_and_escapes_what_was_said() -> None:
    page = exported(ABOUT, events(), "html")

    assert page.startswith("<!doctype html>")
    assert "<title>Session s-1</title>" in page
    assert "prefers-color-scheme: dark" in page
    assert "Fix &lt;b&gt;calc.py&lt;/b&gt; &amp; say why." in page
    assert '<span class="who">You, on the way</span>Keep it small.' in page
    assert "<summary>Call of read</summary>" in page
    assert '<details class="failed"><summary>It failed</summary>' in page
    assert "<summary>It gave back</summary><pre>1\tdef mean(xs):</pre>" in page
    assert "<summary>Summary of the earlier part</summary>" in page
    assert '<p class="end">The turn ended early: error: AUTH: no key</p>' in page
    assert "with deepseek/deepseek-flash" in page
    assert "src=" not in page
    assert "<script" not in page


def test_a_session_without_requests_names_no_model() -> None:
    quiet = [SessionEvent(0, 0, TurnStartEvent(turn=1))]

    assert "Model:" not in exported(ABOUT, quiet, "md")
    assert " with " not in exported(ABOUT, quiet, "html")


# The command


async def loaded(harness: Harness, home: Path) -> Commands:
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(export_command, {"home": str(home)})
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


async def test_the_command_writes_the_page_into_the_home(harness: Harness, tmp_path: Path) -> None:
    commands = await loaded(harness, tmp_path)
    agent = await harness.agent(Reply(text="Hello there."))
    await turn(agent, "Hi.")

    said_html = await commands.run(agent, "/export")
    said_md = await commands.run(agent, "/export md")

    page = tmp_path / "exports" / f"{agent.id}.html"
    assert said_html == f"The conversation is written to {page}"
    assert "Hello there." in page.read_text(encoding="utf-8")
    assert said_md.endswith(f"{agent.id}.md")
    assert await commands.run(agent, "/export pdf") == (
        "Say /export for a page of HTML, or /export md for one of Markdown."
    )


async def test_a_page_that_cannot_be_written_is_said(harness: Harness, tmp_path: Path) -> None:
    (tmp_path / "exports").write_text("in the way", encoding="utf-8")
    commands = await loaded(harness, tmp_path)
    agent = await harness.agent()

    assert (await commands.run(agent, "/export")).startswith("The page could not be written to")
