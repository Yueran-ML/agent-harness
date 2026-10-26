"""A session as a page to read or to hand on: HTML that stands alone, or Markdown.

What was said is written as it was said: what the user wrote, what the model
answered, which tools it called and what they gave back, and how a turn ended
where it did not end well. What the program told the model of itself, and the
thinking of the model, are left out, but for a summary of an earlier part,
which stands where that part stood. A long result of a tool is cut to its
beginning.

The page is made here and written nowhere: whoever asks for it says where it
goes. Nothing of it leaves the machine. It holds what the conversation held,
the contents of files that were read among it, so whoever hands it on should
look at it first.

A page of HTML has its styles in it and asks for nothing else, and follows the
light or dark look of the system that shows it. Text is written as text: what
a model wrote in Markdown is shown as it was written.
"""

import html
import json
import time
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Literal

from columbina.compaction import SUMMARY_SOURCE
from columbina.llm import TextBlock, ToolCallBlock
from columbina.session import (
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)

__all__ = ["RESULT_MOST", "About", "Format", "exported"]

Format = Literal["html", "md"]

RESULT_MOST = 2000
"""How many characters of the result of a tool are shown."""

_SAID_BY_THE_USER = ("user", "steer")


@dataclass(frozen=True, slots=True)
class About:
    """What a page says of the session at its top.

    Attributes:
        id: The id of the session.
        created_at: When it began, in milliseconds since 1970.
        cwd: Its working directory.
    """

    id: str
    created_at: int
    cwd: str


@dataclass(frozen=True, slots=True)
class _Part:
    """One thing that happened, as the page shows it.

    Attributes:
        kind: ``turn``, ``user``, ``assistant``, ``call``, ``result``,
            ``summary`` or ``end``.
        text: What was said.
        name: The tool of a call or a result, the source of what the user said.
        failed: Whether a result is that of a call that failed.
    """

    kind: str
    text: str
    name: str = ""
    failed: bool = False


def _cut(text: str) -> str:
    if len(text) <= RESULT_MOST:
        return text
    return f"{text[:RESULT_MOST]}\n… ({len(text) - RESULT_MOST} more characters)"


def _arguments(written: str) -> str:
    try:
        return json.dumps(json.loads(written), ensure_ascii=False, indent=1)
    except ValueError:
        return written


def _parts(events: Sequence[SessionEvent[EventData]]) -> tuple[list[_Part], str | None]:
    """Return what happened, in order, and the model that the conversation spoke to last."""
    parts: list[_Part] = []
    model = None
    for event in events:
        data = event.data
        if isinstance(data, RequestHeaderEvent):
            model = f"{data.config.provider}/{data.config.model}"
        elif isinstance(data, TurnStartEvent):
            parts.append(_Part("turn", str(data.turn)))
        elif isinstance(data, UserMessageEvent):
            text = "\n\n".join(block.text for block in data.message.content).strip()
            if data.source == SUMMARY_SOURCE:
                parts.append(_Part("summary", text))
            elif data.source in _SAID_BY_THE_USER:
                parts.append(_Part("user", text, data.source))
        elif isinstance(data, AssistantMessageEvent):
            for block in data.message.content:
                if isinstance(block, TextBlock) and block.text.strip():
                    parts.append(_Part("assistant", block.text.strip()))
                elif isinstance(block, ToolCallBlock):
                    parts.append(_Part("call", _arguments(block.arguments), block.name))
        elif isinstance(data, ToolResultEvent):
            text = "\n".join(block.text for block in data.message.content)
            parts.append(_Part("result", _cut(text), failed=data.error is not None))
        elif isinstance(data, TurnEndEvent) and data.reason != "completed":
            detail = data.reason if data.detail is None else f"{data.reason}: {data.detail}"
            parts.append(_Part("end", detail))
    return parts, model


def _when(moment_ms: int) -> str:
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(moment_ms / 1000))


def _fence(text: str) -> str:
    """Return a fence of backticks longer than any run of them in the text."""
    longest = run = 0
    for character in text:
        run = run + 1 if character == "`" else 0
        longest = max(longest, run)
    return "`" * max(3, longest + 1)


def _markdown(header: About, parts: list[_Part], model: str | None) -> str:
    lines = [f"# Session {header.id}", ""]
    lines.append(f"- Started: {_when(header.created_at)}")
    lines.append(f"- Working directory: `{header.cwd}`")
    if model is not None:
        lines.append(f"- Model: {model}")
    for part in parts:
        if part.kind == "turn":
            lines += ["", f"## Turn {part.text}"]
        elif part.kind == "user":
            said = "You, on the way" if part.name == "steer" else "You"
            lines += ["", f"**{said}:**", "", part.text]
        elif part.kind == "assistant":
            lines += ["", "**Columbina:**", "", part.text]
        elif part.kind == "summary":
            lines += ["", "**Summary of the earlier part:**", "", part.text]
        elif part.kind == "end":
            lines += ["", f"*The turn ended early: {part.text}*"]
        else:
            fence = _fence(part.text)
            head = (
                f"Call of `{part.name}`:"
                if part.kind == "call"
                else ("It failed:" if part.failed else "It gave back:")
            )
            lines += ["", head, "", fence, part.text, fence]
    return "\n".join(lines) + "\n"


_STYLE = """
:root { --bg: #fbfaf8; --fg: #1d1b20; --muted: #6b6672; --line: #e4e0ea; --you: #efeaf8;
  --tool: #f3f1f6; --bad: #b3261e; color-scheme: light dark; }
@media (prefers-color-scheme: dark) { :root { --bg: #141218; --fg: #e8e2ec; --muted: #a39dab;
  --line: #2f2b35; --you: #2a2436; --tool: #1e1b23; --bad: #f2b8b5; } }
body { margin: 0; background: var(--bg); color: var(--fg);
  font: 15px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 52rem; margin: 0 auto; padding: 2rem 1rem 4rem; }
h1 { font-size: 1.4rem; margin: 0 0 .25rem; } h2 { font-size: 1rem; color: var(--muted);
  border-top: 1px solid var(--line); padding-top: 1rem; margin-top: 2rem; }
.meta { color: var(--muted); font-size: .9rem; }
.said { white-space: pre-wrap; overflow-wrap: anywhere; margin: .75rem 0; }
.user { background: var(--you); border-radius: .75rem; padding: .6rem .9rem; }
.who { font-weight: 600; font-size: .85rem; color: var(--muted); display: block; }
details { background: var(--tool); border-radius: .5rem; padding: .4rem .7rem; margin: .5rem 0; }
summary { cursor: pointer; font-size: .9rem; } pre { white-space: pre-wrap; overflow-wrap: anywhere;
  font: 13px/1.5 ui-monospace, Consolas, monospace; margin: .4rem 0; }
.failed summary { color: var(--bad); } .end { color: var(--bad); font-style: italic; }
"""


def _html(header: About, parts: list[_Part], model: str | None) -> str:
    e = html.escape
    body: list[str] = [f"<h1>Session {e(header.id)}</h1>"]
    meta = [f"Started {e(_when(header.created_at))}", f"in {e(header.cwd)}"]
    if model is not None:
        meta.append(f"with {e(model)}")
    body.append(f'<p class="meta">{" · ".join(meta)}</p>')
    for part in parts:
        if part.kind == "turn":
            body.append(f"<h2>Turn {e(part.text)}</h2>")
        elif part.kind == "user":
            said = "You, on the way" if part.name == "steer" else "You"
            body.append(
                f'<div class="said user"><span class="who">{said}</span>{e(part.text)}</div>'
            )
        elif part.kind == "assistant":
            body.append(f'<div class="said"><span class="who">Columbina</span>{e(part.text)}</div>')
        elif part.kind == "summary":
            body.append(
                f"<details><summary>Summary of the earlier part</summary>"
                f"<pre>{e(part.text)}</pre></details>"
            )
        elif part.kind == "end":
            body.append(f'<p class="end">The turn ended early: {e(part.text)}</p>')
        elif part.kind == "call":
            body.append(
                f"<details><summary>Call of {e(part.name)}</summary>"
                f"<pre>{e(part.text)}</pre></details>"
            )
        else:
            said = "It failed" if part.failed else "It gave back"
            failed = ' class="failed"' if part.failed else ""
            body.append(
                f"<details{failed}><summary>{said}</summary><pre>{e(part.text)}</pre></details>"
            )
    return (
        '<!doctype html>\n<html lang="en"><head><meta charset="utf-8">\n'
        '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
        f"<title>Session {e(header.id)}</title>\n<style>{_STYLE}</style></head>\n"
        "<body><main>\n" + "\n".join(body) + "\n</main></body></html>\n"
    )


def exported(header: About, events: Sequence[SessionEvent[EventData]], form: Format) -> str:
    """Return a session as a page of HTML or of Markdown."""
    parts, model = _parts(events)
    return _html(header, parts, model) if form == "html" else _markdown(header, parts, model)
