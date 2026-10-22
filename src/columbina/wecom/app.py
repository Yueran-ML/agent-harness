"""Where WeCom is, and what goes wrong on the way there.

The assistant answers through a smart bot of WeCom whose API is set to the long connection.
Such a bot is reached at one address, where a WebSocket is opened and the bot signs in with
its Bot ID and its Secret. A private deployment of WeCom has an address of its own, which
``COLUMBINA_WECOM_WS`` names. So do the tests, which put WeCom on this machine. Only this
machine may be reached without encryption, also for the files that WeCom lets be downloaded.
"""

from collections.abc import Mapping
from urllib.parse import urlsplit

__all__ = ["ADDRESS", "WecomError", "address", "safe_to_fetch"]

ADDRESS = "wss://openws.work.weixin.qq.com"
"""Where the long connection of a smart bot is opened."""

_HERE = ("localhost", "127.0.0.1", "::1")


class WecomError(Exception):
    """What WeCom, or the way to it, did not do. The text is for a person.

    Attributes:
        code: A stable code for programs, such as ``UNREACHABLE`` or ``REFUSED``.
        number: The errcode that WeCom answered with, where it answered with one.
    """

    def __init__(self, message: str, *, code: str, number: int | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.number = number


def address(env: Mapping[str, str]) -> str:
    """Return where the long connection is opened.

    Raises:
        WecomError: The variable that puts WeCom elsewhere names a place that cannot be
            reached safely: one without encryption that is not this machine.
    """
    named = (env.get("COLUMBINA_WECOM_WS") or ADDRESS).rstrip("/")
    parts = urlsplit(named)
    if parts.scheme != "wss" and not (parts.scheme == "ws" and parts.hostname in _HERE):
        raise WecomError(
            f"COLUMBINA_WECOM_WS names {named}. WeCom is reached over wss, and only this "
            "machine without encryption",
            code="SETUP",
        )
    return named


def safe_to_fetch(url: str) -> bool:
    """Tell whether a file that WeCom names may be downloaded from where it lies.

    That is over https, or from this machine.
    """
    parts = urlsplit(url)
    return parts.scheme == "https" or (parts.scheme == "http" and parts.hostname in _HERE)
