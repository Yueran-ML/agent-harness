"""The GitHub App that Columbina signs in with, and where GitHub is.

Columbina signs in to GitHub with the device flow of a GitHub App. The flow needs no secret
and no server of Columbina: a GitHub App has a client ID that anyone may know, and it is
written down here. A build of Columbina of another hand names its own app with the variables
``COLUMBINA_GITHUB_CLIENT_ID`` and ``COLUMBINA_GITHUB_SLUG``. The tests put GitHub on this
machine with ``COLUMBINA_GITHUB_WEB`` and ``COLUMBINA_GITHUB_API``.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from urllib.parse import urlsplit

__all__ = ["CLIENT_ID", "SLUG", "App", "GitHubError", "app", "web_of"]

CLIENT_ID = "Iv23li4xSpAf5B18565X"
"""The client ID of the GitHub App of Columbina, Columbina-Agent. It is no secret."""

SLUG = "columbina-agent"
"""The name of the app in its address on GitHub: ``https://github.com/apps/<slug>``."""

_WEB = "https://github.com"
_API = "https://api.github.com"
_HERE = ("localhost", "127.0.0.1", "::1")
"""The names of this machine, which alone may be reached without encryption."""


class GitHubError(Exception):
    """What GitHub, or the way to it, did not do. The text is for a person.

    Attributes:
        code: A stable code for programs, such as ``EXPIRED`` or ``UNREACHABLE``.
    """

    def __init__(self, message: str, *, code: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, slots=True)
class App:
    """A GitHub App, and the GitHub that it is on.

    Attributes:
        client_id: The client ID of the app.
        web: Where GitHub is for a person and for git, without a slash at the end.
        api: Where its REST API is, without a slash at the end.
        slug: The name of the app in its address.
    """

    client_id: str
    web: str
    api: str
    slug: str

    @property
    def install_url(self) -> str:
        """The page on which a user chooses the repositories that the app may reach."""
        return f"{self.web}/apps/{self.slug}/installations/new"


def _address(said: str, variable: str) -> str:
    """Return an address of GitHub as it is used: without the slash at its end.

    Raises:
        GitHubError: It is no address, or it is not encrypted and leads off this machine.
    """
    parts = urlsplit(said)
    if parts.scheme == "https" and parts.hostname:
        return said.rstrip("/")
    if parts.scheme == "http" and parts.hostname in _HERE:
        return said.rstrip("/")
    raise GitHubError(
        f"{variable} says {said!r}, and that is no address of GitHub. An address begins with "
        "https://, or with http:// for this machine",
        code="MISCONFIGURED",
    )


def web_of(env: Mapping[str, str]) -> str:
    """Return where GitHub is for a person, as the environment says, with an app or without.

    Raises:
        GitHubError: The variable names an address that cannot be one.
    """
    web = env.get("COLUMBINA_GITHUB_WEB")
    return _address(web, "COLUMBINA_GITHUB_WEB") if web else _WEB


def app(env: Mapping[str, str]) -> App:
    """Return the GitHub App that Columbina signs in with, as the environment says.

    Raises:
        GitHubError: No app is registered for this build, or a variable names
            an address that cannot be one.
    """
    client_id = env.get("COLUMBINA_GITHUB_CLIENT_ID") or CLIENT_ID
    slug = env.get("COLUMBINA_GITHUB_SLUG") or SLUG
    if not client_id or not slug:
        raise GitHubError(
            "this build of Columbina has no GitHub App to sign in with. Its maker registers "
            "one on GitHub, or COLUMBINA_GITHUB_CLIENT_ID and COLUMBINA_GITHUB_SLUG name one",
            code="NOT_REGISTERED",
        )
    api = env.get("COLUMBINA_GITHUB_API")
    return App(
        client_id=client_id,
        web=web_of(env),
        api=_address(api, "COLUMBINA_GITHUB_API") if api else _API,
        slug=slug,
    )
