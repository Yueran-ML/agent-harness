"""What a message to the bot says.

WeCom sends a message to the bot as an object of JSON with its type in ``msgtype`` and what it
holds under a key of that name. Text is read as it is, and so is a voice message, which WeCom
has written down as text already. A message of text and pictures is read as its text. A file
is a link that holds for some minutes and a key that it is encrypted with, see
:mod:`columbina.wecom.media`. What else a person may send, a picture or a video for one, is no
text, and is said to be so.

A message that quotes another one keeps what it quotes apart from what it says: a command is
what the person wrote, and the agent is given both.
"""

from dataclasses import dataclass
from typing import Any, cast

__all__ = ["File", "Said", "Text", "Unread", "read"]


@dataclass(frozen=True, slots=True)
class Text:
    """Words, for the agent.

    Attributes:
        text: What the person wrote, or said.
        quoted: The words of the message that it quotes, where it quotes one that holds some.
    """

    text: str
    quoted: str | None = None

    def asked(self) -> str:
        """Return what the agent is given: the words, after what they quote."""
        if not self.quoted:
            return self.text
        lines = "\n".join(f"> {line}" for line in self.quoted.splitlines())
        return f"{lines}\n\n{self.text}"


@dataclass(frozen=True, slots=True)
class File:
    """A file that was sent: where it can be downloaded, and the key it is encrypted with."""

    url: str
    key: str


@dataclass(frozen=True, slots=True)
class Unread:
    """A message of a kind that is not read, such as a picture."""

    kind: str


Said = Text | File | Unread


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def _text(value: object) -> str:
    return value if isinstance(value, str) else ""


def _words(body: dict[str, Any]) -> str | None:
    """Return the words of a message, or ``None`` where it holds none to read."""
    kind = body.get("msgtype")
    if kind == "text":
        return _text(_object(body.get("text")).get("content"))
    if kind == "voice":
        return _text(_object(body.get("voice")).get("content"))
    if kind == "mixed":
        items = _object(body.get("mixed")).get("msg_item")
        listed = cast("list[object]", items) if isinstance(items, list) else []
        parts = [
            _text(_object(_object(item).get("text")).get("content"))
            for item in listed
            if _object(item).get("msgtype") == "text"
        ]
        return "\n".join(part for part in parts if part)
    return None


def read(body: dict[str, Any]) -> Said:
    """Return what a message says, from the body of the frame that brought it."""
    kind = body.get("msgtype")
    if kind == "file":
        sent = _object(body.get("file"))
        url, key = sent.get("url"), sent.get("aeskey")
        if isinstance(url, str) and url and isinstance(key, str) and key:
            return File(url, key)
        return Unread("file")
    words = _words(body)
    if words is None:
        return Unread(kind if isinstance(kind, str) else "")
    return Text(words, _words(_object(body.get("quote"))) or None)
