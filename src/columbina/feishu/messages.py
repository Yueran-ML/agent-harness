"""What a message of Feishu says, and how an answer is written as one.

A message comes as its type and its content, which is JSON written into a string. Text is
read as it is. Rich text is read from the Markdown that it was written as, where Feishu keeps
that, and otherwise from its pieces: words, links and code, the way Markdown writes them. A
file is named, to be downloaded. What else a person may send, a picture or a voice message
for one, is no text, and is said to be so.

An answer goes out as rich text with one paragraph of Markdown, which Feishu shows rendered:
headings, lists, fenced code and tables, as a model writes them. A request may hold 30 KB at
most, so a long answer goes out in parts, cut between lines. A cut that falls inside fenced
code closes the fence and opens it again in the next part, so that each part is shown right.
"""

import json
from dataclasses import dataclass
from typing import Any, cast

__all__ = ["File", "Said", "Text", "Unread", "parts", "posted", "read"]

_MOST = 28_000
"""How many bytes the body of a request that sends a part may take, under the 30 KB that
Feishu takes, with room for what the request says besides."""

_FENCES = ("```", "~~~")


@dataclass(frozen=True, slots=True)
class Text:
    """Words, for the agent."""

    text: str


@dataclass(frozen=True, slots=True)
class File:
    """A file that was sent, which can be downloaded with its key."""

    key: str
    name: str


@dataclass(frozen=True, slots=True)
class Unread:
    """A message of a kind that is not read, such as a picture."""

    kind: str


Said = Text | File | Unread


def _object(content: str) -> dict[str, Any]:
    try:
        found: object = json.loads(content)
    except ValueError:
        return {}
    return cast("dict[str, Any]", found) if isinstance(found, dict) else {}


def _node(node: object) -> str:
    """Return a piece of rich text as Markdown, or nothing for a piece that holds no words."""
    if not isinstance(node, dict):
        return ""
    given = cast("dict[str, Any]", node)
    tag, text = given.get("tag"), given.get("text")
    words = text if isinstance(text, str) else ""
    if tag in ("text", "md"):
        return words
    if tag == "a":
        href = given.get("href")
        return f"[{words}]({href})" if isinstance(href, str) and href else words
    if tag == "at":
        name = given.get("user_name")
        return f"@{name}" if isinstance(name, str) and name else ""
    if tag == "code_block":
        language = given.get("language")
        info = language.lower() if isinstance(language, str) else ""
        return f"```{info}\n{words}\n```"
    if tag == "hr":
        return "---"
    if tag in ("img", "media"):
        return "[a picture that was not read]" if tag == "img" else "[a video that was not read]"
    return ""


def _paragraphs(paragraphs: object) -> list[str]:
    if not isinstance(paragraphs, list):
        return []
    lines: list[str] = []
    for paragraph in cast("list[object]", paragraphs):
        nodes = cast("list[object]", paragraph) if isinstance(paragraph, list) else []
        line = "".join(_node(node) for node in nodes)
        if line:
            lines.append(line)
    return lines


def read(kind: str, content: str) -> Said:
    """Return what a message says, from its type and its content."""
    said = _object(content)
    if kind == "text":
        text = said.get("text")
        return Text(text if isinstance(text, str) else "")
    if kind == "post":
        lines = _paragraphs(said.get("content_v2")) or _paragraphs(said.get("content"))
        title = said.get("title")
        if isinstance(title, str) and title:
            lines.insert(0, title)
        return Text("\n".join(lines))
    if kind == "file":
        key, name = said.get("file_key"), said.get("file_name")
        if isinstance(key, str) and key:
            return File(key, name if isinstance(name, str) and name else "file")
    return Unread(kind)


def posted(markdown: str, language: str = "zh_cn") -> str:
    """Return the content of a message of rich text that holds one paragraph of Markdown."""
    post = {language: {"content": [[{"tag": "md", "text": markdown}]]}}
    return json.dumps(post, ensure_ascii=False, separators=(",", ":"))


def _cost(text: str) -> int:
    """Return how many bytes a text takes in the body of a request, as content in content."""
    inner = json.dumps(text, ensure_ascii=False)[1:-1]
    return len(json.dumps(inner, ensure_ascii=False)[1:-1].encode())


def _fence(line: str) -> str | None:
    """Return the marker of fenced code that a line opens or closes, or ``None``."""
    stripped = line.lstrip(" ")
    if len(line) - len(stripped) > 3:
        return None
    for mark in _FENCES:
        if stripped.startswith(mark):
            return mark[0] * (len(stripped) - len(stripped.lstrip(mark[0])))
    return None


def _pieces(text: str, budget: int) -> list[str]:
    """Cut a text that is too long for a part by itself into pieces that fit, or leave it whole."""
    if _cost(text) <= budget:
        return [text]
    pieces: list[str] = []
    piece = ""
    for character in text:
        if piece and _cost(piece + character) > budget:
            pieces.append(piece)
            piece = ""
        piece += character
    pieces.append(piece)
    return pieces


def parts(markdown: str, most: int = _MOST) -> list[str]:
    """Cut Markdown into parts that each fit in a message, between lines where it can.

    Args:
        markdown: The whole answer.
        most: How many bytes the body of a request may take at most.

    Returns:
        The parts, in their order. A part that was cut between two lines does not end with
        the line break between them. Fenced code that a cut falls in is closed at the end of
        one part and opened again at the beginning of the next. Only a line too long for a
        part of its own is cut inside.
    """
    budget = most - _cost(posted("")) - 200
    done: list[str] = []
    current = ""
    size = 0
    opened: str | None = None
    marker: str | None = None
    lines = markdown.split("\n")
    for index, whole in enumerate(lines):
        unit = whole if index == len(lines) - 1 else f"{whole}\n"
        for piece in _pieces(unit, budget // 2):
            cost = _cost(piece)
            closing = _cost(f"\n{marker}") if marker is not None else 0
            if current and size + cost + closing > budget:
                ending = current.removesuffix("\n")
                done.append(ending if marker is None else f"{ending}\n{marker}")
                current = "" if opened is None else f"{opened}\n"
                size = _cost(current)
            current += piece
            size += cost
        mark = _fence(whole)
        if mark is not None:
            if marker is None:
                opened, marker = whole, mark
            elif mark.startswith(marker) and not whole.strip().strip(marker[0]):
                opened, marker = None, None
    done.append(current)
    return done
