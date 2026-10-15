"""A token for the API of GitHub that holds, whenever one is asked for.

The token of a sign-in holds for eight hours. Whoever asks for one a few minutes before its
end, or after GitHub turned it down, gets a renewed one: the pair is renewed with the token
that renews it, under the lock of :func:`columbina.github.store.renewing`, and the new pair is
kept. A program that waited for the lock finds the pair that the other one renewed, and takes
it. Where the renewing token is good no more, the sign-in is over: what is kept of it is
deleted, and the user signs in again.
"""

import asyncio
import time
from collections.abc import Callable
from pathlib import Path

import httpx

from columbina.github import store
from columbina.github.app import App, GitHubError
from columbina.github.oauth import renewed

__all__ = ["EARLY", "access_token"]

EARLY = 300.0
"""How many seconds before its end a token is renewed: none goes out that ends on the way."""

_NOBODY = (
    "nobody is signed in to GitHub. Sign in with 'columbina login github', or in the settings "
    "of the desktop application"
)


def _over() -> GitHubError:
    return GitHubError("the sign-in to GitHub has run out. Sign in again", code="SIGNED_OUT")


async def access_token(
    home: Path,
    app: App,
    http: httpx.AsyncClient,
    *,
    now: Callable[[], float] = time.time,
    turned_down: str | None = None,
) -> str:
    """Return a token for the API that holds for a few minutes at least.

    Args:
        home: The home of the program.
        app: The app that the user signed in with.
        http: The client that GitHub is asked with.
        now: The time, in seconds since 1970.
        turned_down: A token that GitHub just turned down. It is renewed
            where it should hold too, unless another program renewed it.

    Raises:
        GitHubError: Nobody is signed in, or the sign-in ran out
            (``SIGNED_OUT``), or the tokens could not be renewed.
    """
    held = await asyncio.to_thread(store.tokens, home, app)
    if held is None:
        raise GitHubError(_NOBODY, code="SIGNED_OUT")
    if held.access != turned_down and held.access_expires_at - now() > EARLY:
        return held.access
    async with store.renewing(home):
        # Another program may have renewed the pair while this one waited for the lock, or
        # signed out.
        said = await asyncio.to_thread(store.signed, home, app)
        again = await asyncio.to_thread(store.tokens, home, app)
        if said is None or again is None:
            raise GitHubError(_NOBODY, code="SIGNED_OUT")
        stale = (held.access, turned_down)
        if again.access not in stale and again.access_expires_at - now() > EARLY:
            return again.access
        if again.renewal_expires_at <= now():
            await asyncio.to_thread(store.forget, home)
            raise _over()
        try:
            fresh = await renewed(http, app, again.renewal, now=now)
        except GitHubError as error:
            if error.code == "SIGNED_OUT":
                await asyncio.to_thread(store.forget, home)
            raise
        await asyncio.to_thread(store.keep, home, app, fresh, said.login, said.name)
        return fresh.access
