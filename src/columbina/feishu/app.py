"""Where Feishu is: the two hosts of its open platform, and what goes wrong on the way there.

Feishu is called Lark outside China, and the two are run apart: an app that is made in the
developer console of one exists only there, and is reached at its host alone. The user says
which of the two their app is in, and that names the host of the REST API and of the long
connection.

The tests put Feishu on this machine with the variable ``COLUMBINA_FEISHU_API``. Only this
machine may be reached without encryption.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Literal, get_args
from urllib.parse import urlsplit

__all__ = ["DOMAINS", "Domain", "FeishuError", "Host", "host"]

Domain = Literal["feishu", "lark"]
"""Which of the two the app is in: Feishu, in China, or Lark, elsewhere."""

DOMAINS: tuple[Domain, ...] = get_args(Domain)

_APIS: dict[Domain, str] = {
    "feishu": "https://open.feishu.cn",
    "lark": "https://open.larksuite.com",
}
_HERE = ("localhost", "127.0.0.1", "::1")


class FeishuError(Exception):
    """What Feishu, or the way to it, did not do. The text is for a person.

    Attributes:
        code: A stable code for programs, such as ``UNREACHABLE`` or ``FORBIDDEN``.
        number: The code that Feishu itself answered with, where it answered with one, such
            as 1770032 for a document that the app may not read.
        after: How many seconds Feishu asks to be left alone, where it asked for a pause.
    """

    def __init__(
        self, message: str, *, code: str, number: int | None = None, after: float | None = None
    ) -> None:
        super().__init__(message)
        self.code = code
        self.number = number
        self.after = after


@dataclass(frozen=True, slots=True)
class Host:
    """The open platform that an app is in.

    Attributes:
        domain: Feishu or Lark.
        api: Where its REST API is, and where the long connection is asked for, without a
            slash at the end.
    """

    domain: Domain
    api: str

    @property
    def console(self) -> str:
        """The developer console, where the user makes and sets up the app."""
        return f"{_APIS[self.domain]}/app"


def host(domain: Domain, env: Mapping[str, str]) -> Host:
    """Return the host of the open platform of a domain.

    Raises:
        FeishuError: The variable that puts Feishu elsewhere names a place that cannot be
            reached safely: one without encryption that is not this machine.
    """
    api = (env.get("COLUMBINA_FEISHU_API") or _APIS[domain]).rstrip("/")
    parts = urlsplit(api)
    if parts.scheme != "https" and not (parts.scheme == "http" and parts.hostname in _HERE):
        raise FeishuError(
            f"COLUMBINA_FEISHU_API names {api}. Feishu is reached over https, and only this "
            "machine without it",
            code="SETUP",
        )
    return Host(domain, api)
