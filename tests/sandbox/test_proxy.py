"""The proxy that lets the commands of a container reach some hosts, and the config of it."""

import asyncio
from collections.abc import AsyncIterator

import pytest
from pydantic import ValidationError

from columbina.sandbox import docker, proxy
from columbina.sandbox.docker import DockerConfig


@pytest.mark.parametrize(
    ("host", "allowed"),
    [
        ("pypi.org", True),
        ("PyPI.org.", True),
        ("files.pythonhosted.org", True),
        ("pythonhosted.org", False),
        ("evilpypi.org", False),
        ("example.com", False),
    ],
)
def test_a_host_is_allowed_where_a_pattern_names_it(host: str, allowed: bool) -> None:
    assert proxy.allows(host, ["pypi.org", "*.pythonhosted.org"]) is allowed


class Upstream:
    """A server of this machine that answers what it is sent with what it was sent."""

    def __init__(self) -> None:
        self.heard: list[bytes] = []

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        said = await reader.read(4096)
        self.heard.append(said)
        writer.write(b"echo:" + said)
        await writer.drain()
        writer.close()


@pytest.fixture
async def upstream() -> AsyncIterator[tuple[Upstream, int]]:
    made = Upstream()
    server = await asyncio.start_server(made.handle, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    async with server:
        yield made, port


@pytest.fixture
async def through() -> AsyncIterator[int]:
    server = await proxy.serve("127.0.0.1", 0, ["localhost", "127.0.0.1"])
    async with server:
        yield server.sockets[0].getsockname()[1]


@pytest.fixture
def closed(monkeypatch: pytest.MonkeyPatch) -> int:
    """Refuse an upstream connection without depending on the host's closed-port policy.

    The client still reaches the real proxy over TCP. Only its connection to this upstream
    fails: a bound socket that is not listening can leave connect waiting on macOS.
    """
    connect = asyncio.open_connection

    async def refused(host: str, port: int) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
        if host == "localhost" and port == 80:
            raise ConnectionRefusedError("the test upstream refuses connections")
        return await connect(host, port)

    monkeypatch.setattr(asyncio, "open_connection", refused)
    return 80


async def asked(port: int, request: bytes) -> bytes:
    reader, writer = await asyncio.open_connection("127.0.0.1", port)
    try:
        writer.write(request)
        await writer.drain()
        writer.write_eof()
        return await asyncio.wait_for(reader.read(), timeout=20)
    finally:
        writer.close()
        await writer.wait_closed()


async def test_a_tunnel_to_an_allowed_host_is_made(
    through: int, upstream: tuple[Upstream, int]
) -> None:
    _, port = upstream
    reader, writer = await asyncio.open_connection("127.0.0.1", through)
    writer.write(f"CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: x\r\n\r\n".encode())
    await writer.drain()

    assert await reader.readuntil(b"\r\n\r\n") == b"HTTP/1.1 200 Connection established\r\n\r\n"
    writer.write(b"hello")
    await writer.drain()
    assert await asyncio.wait_for(reader.read(), timeout=5) == b"echo:hello"
    writer.close()


async def test_a_plain_request_to_an_allowed_host_is_passed_on(
    through: int, upstream: tuple[Upstream, int]
) -> None:
    made, port = upstream

    answer = await asked(
        through, f"GET http://localhost:{port}/simple/ HTTP/1.1\r\nHost: localhost\r\n\r\n".encode()
    )

    assert made.heard == [b"GET /simple/ HTTP/1.1\r\nHost: localhost\r\n\r\n"]
    assert answer.startswith(b"echo:GET /simple/")


@pytest.mark.parametrize(
    ("request_line", "status", "said"),
    [
        (
            b"CONNECT example.com:443 HTTP/1.1\r\n\r\n",
            b"403",
            "does not let commands reach example.com",
        ),
        (b"GET http://[::1]/ HTTP/1.1\r\n\r\n", b"403", "does not let commands reach ::1"),
        (b"GET / HTTP/1.1\r\n\r\n", b"400", "Only a proxy is listening here."),
        (b"nonsense\r\n\r\n", b"400", "That is no request of HTTP."),
        (
            b"GET http://x/" + b"a" * 70_000 + b" HTTP/1.1\r\n\r\n",
            b"400",
            "The request is too long.",
        ),
        (b"CONNECT localhost:{closed} HTTP/1.1\r\n\r\n", b"502", "localhost cannot be reached"),
        (b"GET http://localhost:{closed}/ HTTP/1.1\r\n\r\n", b"502", "localhost cannot be reached"),
    ],
    ids=["elsewhere", "an-address", "no-proxy", "nonsense", "too-long", "unreachable", "plain"],
)
async def test_what_is_not_passed_on_is_refused_in_words(
    through: int, closed: int, request_line: bytes, status: bytes, said: str
) -> None:
    answer = await asked(through, request_line.replace(b"{closed}", str(closed).encode()))

    assert answer.split(b" ")[1] == status
    assert said in answer.decode()


async def test_a_request_that_breaks_off_is_refused(through: int) -> None:
    assert b"400" in await asked(through, b"GET http://localhost/ HTTP/1.1\r\n")


# The config


def test_a_network_that_is_allowed_names_its_hosts() -> None:
    config = DockerConfig(network="allowed", allowed_hosts=("pypi.org", "*.pythonhosted.org"))

    assert config.proxy_image == docker.DEFAULT_IMAGE
    with pytest.raises(ValidationError, match="names the hosts in allowed_hosts"):
        DockerConfig(network="allowed")
    with pytest.raises(ValidationError, match="is no name of a host"):
        DockerConfig(network="allowed", allowed_hosts=("https://pypi.org",))


def test_the_model_is_told_which_hosts_it_reaches() -> None:
    config = DockerConfig(network="allowed", allowed_hosts=("pypi.org", "*.npmjs.org"))

    (said,) = docker.told(config, "linux").values()

    assert (
        "a network through a proxy, which lets commands reach these hosts and no others: " in said
    )
    assert "pypi.org, *.npmjs.org." in said


def test_a_command_is_told_of_the_proxy_and_others_are_not() -> None:
    allowed = DockerConfig(network="allowed", allowed_hosts=("pypi.org",))

    told = docker.called("docker", allowed, "c", "1000:1000", "/w", ["true"])
    plain = docker.called("docker", DockerConfig(), "c", "1000:1000", "/w", ["true"])

    assert "HTTPS_PROXY=http://columbina-proxy:3128" in told
    assert "NO_PROXY=localhost,127.0.0.1" in told
    assert not any("PROXY" in word for word in plain)
