"""Where the requests that Columbina makes itself go: straight, or through the proxy of the system.

httpx reads ``HTTPS_PROXY``, ``HTTP_PROXY``, ``ALL_PROXY`` and ``NO_PROXY`` by itself, and a
program that is started at a terminal has them where its user set them. An application that is
opened from the Start menu or the Finder often has none of them, while the system has a proxy:
the Internet settings of Windows or the network settings of macOS, which a tool such as Clash
sets. So where none of the three variables names a proxy, a request that Columbina makes itself
follows what the system says, :func:`proxy_for`. The commands that an agent runs are not
touched: they have their environment as it is.
"""

import urllib.request
from collections.abc import Callable, Mapping
from urllib.parse import urlsplit

__all__ = ["proxy_for"]

_VARIABLES = ("https_proxy", "http_proxy", "all_proxy")
"""The variables of the environment that name a proxy, in small letters. httpx reads them."""

_HERE = ("localhost", "127.0.0.1", "::1")
"""The names of this machine. A request to it goes straight."""

_SPOKEN = ("http", "https")
"""The kinds of proxy that httpx speaks as it is installed here. A SOCKS proxy needs more."""


def proxy_for(
    url: str,
    env: Mapping[str, str],
    *,
    system: Callable[[], Mapping[str, str]] = urllib.request.getproxies,
    bypassed: Callable[[str], bool] = urllib.request.proxy_bypass,
) -> str | None:
    """Return the proxy of the system that a request to an address goes through.

    Args:
        url: The address of the request.
        env: The environment of this program.
        system: Asks the system for its proxies, by scheme, as ``urllib`` does.
        bypassed: Says whether the system lets a host go straight.

    Returns:
        The address of the proxy, or ``None``: the request goes straight, or a
        variable of the environment names a proxy, which httpx takes by itself.
        A proxy of a kind that httpx does not speak here is not taken either.
    """
    if any(name.lower() in _VARIABLES and value for name, value in env.items()):
        return None
    parts = urlsplit(url)
    host = parts.hostname or ""
    if host in _HERE:
        return None
    try:
        said = system()
    except OSError:
        # Settings that cannot be read say nothing of a proxy.
        return None
    proxy = said.get(parts.scheme) or said.get("all")
    if not proxy or bypassed(host):
        return None
    # The settings of Windows name a proxy without its scheme, and mean http.
    proxy = proxy if "://" in proxy else f"http://{proxy}"
    return proxy if urlsplit(proxy).scheme in _SPOKEN else None
