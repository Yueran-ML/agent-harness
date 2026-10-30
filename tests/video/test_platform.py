"""The editing studio runs on Windows only, for now; elsewhere it says so instead of failing."""

import argparse
import json
import logging
import sys
from pathlib import Path
from typing import cast

import pytest

import columbina.tools
import columbina.video
from columbina.agent import RUNTIME_FACTS, Agent, Facts
from columbina.apps import video_command
from columbina.kernel import Context
from columbina.tools import TOOLS
from columbina.video.engine import NOT_FOUND, ONLY_WINDOWS

ELSEWHERE = ["darwin", "linux"]


def parsed(*argv: str) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    video_command.arguments(parser)
    return parser.parse_args(argv)


def told(ctx: Context) -> dict[str, str]:
    facts = Facts(cast("Agent", object()))
    ctx.emit(RUNTIME_FACTS, facts)
    return facts.lines


async def test_on_windows_the_tools_are_offered(
    ctx: Context, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(sys, "platform", "win32")
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.video, {"enabled": True})
    offered = ctx.use(TOOLS).registry
    assert {"video_info", "video_edit", "video_render", "video_apply"} <= set(offered)
    assert "video editing" not in told(ctx)


@pytest.mark.parametrize("platform", ELSEWHERE)
async def test_elsewhere_no_tool_is_offered_and_the_model_is_told_why(
    ctx: Context,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
    platform: str,
) -> None:
    monkeypatch.setattr(sys, "platform", platform)
    await ctx.plugin(columbina.tools)
    with caplog.at_level(logging.WARNING, logger="columbina.video"):
        await ctx.plugin(columbina.video, {"enabled": True})
    assert not [name for name in ctx.use(TOOLS).registry if name.startswith("video_")]
    fact = told(ctx)["video editing"]
    assert "currently needs Windows" in fact
    assert "macOS support will come later" in fact
    assert caplog.messages == [f"columbina: {ONLY_WINDOWS}"]


async def test_a_plugin_that_is_not_enabled_says_nothing_anywhere(
    ctx: Context, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(sys, "platform", "darwin")
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.video, {"enabled": False})
    assert "video editing" not in told(ctx)


@pytest.mark.parametrize("platform", ELSEWHERE)
@pytest.mark.parametrize(
    "argv",
    [
        ("info", "film"),
        ("render", "film"),
        ("engine", "--project", "film"),
        ("migrate", "--workspace", "."),
    ],
)
def test_elsewhere_every_command_says_windows_is_needed(
    monkeypatch: pytest.MonkeyPatch, platform: str, argv: tuple[str, ...]
) -> None:
    monkeypatch.setattr(sys, "platform", platform)
    said: list[str] = []
    assert video_command.run(parsed(*argv), said.append) == 2
    assert said == [
        "columbina: video editing currently needs Windows; macOS support will come later"
    ]


@pytest.mark.parametrize("what", ["engine", "migrate"])
def test_elsewhere_a_command_that_answers_in_json_says_it_in_json(
    monkeypatch: pytest.MonkeyPatch, what: str
) -> None:
    monkeypatch.setattr(sys, "platform", "darwin")
    said: list[str] = []
    argv = ("--project", "film") if what == "engine" else ("--workspace", ".")
    assert video_command.run(parsed(what, *argv, "--json"), said.append) == 2
    assert [json.loads(line) for line in said] == [{"error": ONLY_WINDOWS}]


def test_on_windows_a_command_goes_on_to_look_for_the_engine(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(sys, "platform", "win32")
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("COLUMBINA_ENGINE", raising=False)
    monkeypatch.setenv("PATH", "")
    said: list[str] = []
    assert video_command.run(parsed("engine", "--project", "film"), said.append) == 2
    assert said == [f"columbina: {NOT_FOUND}"]
