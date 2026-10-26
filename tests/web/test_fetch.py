"""Taking a page from the web."""

import asyncio
from collections.abc import Callable

import httpx
import pytest

from columbina.tools import ToolFailure
from columbina.web import fetch as pages
from support.web import responding

OUTSIDE = ["93.184.215.14"]


async def outside(host: str, port: int) -> list[str]:
    return OUTSIDE


def served(answer: Callable[[httpx.Request], httpx.Response]) -> httpx.MockTransport:
    return httpx.MockTransport(answer)


async def taken(
    address: str,
    answer: Callable[[httpx.Request], httpx.Response],
    *,
    most_bytes: int = 10_000,
    seconds: float = 5.0,
    inside: bool = False,
    look_up: pages.Lookup = outside,
) -> pages.Fetched:
    return await pages.fetch(
        address,
        {},
        most_bytes=most_bytes,
        seconds=seconds,
        inside=inside,
        look_up=look_up,
        transport=served(answer),
    )


async def refused(
    address: str, answer: Callable[[httpx.Request], httpx.Response], **more: object
) -> ToolFailure:
    with pytest.raises(ToolFailure) as caught:
        await taken(address, answer, **more)  # type: ignore[arg-type]
    return caught.value


def html(body: str, **headers: str) -> Callable[[httpx.Request], httpx.Response]:
    return responding(
        200, headers={"content-type": "text/html; charset=utf-8", **headers}, text=body
    )


async def test_a_page_of_html_comes_as_what_a_reader_sees() -> None:
    page = await taken(
        "https://example.com/a", html("<title>Moon</title><p>Bright <a href='/b'>b</a></p>")
    )

    assert page == pages.Fetched(
        "https://example.com/a", "Moon", "Bright [b](https://example.com/b)", cut=False
    )


async def test_the_request_says_who_asks_and_what_it_takes() -> None:
    seen: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, text="ok")

    await taken("https://example.com/", answer)

    assert "Columbina/" in seen[0].headers["user-agent"]
    assert seen[0].headers["accept"].startswith("text/html")


async def test_text_that_is_no_html_comes_as_it_stands() -> None:
    answer = responding(200, headers={"content-type": "application/json"}, text='{"a": "<p>1</p>"}')

    page = await taken("https://example.com/a.json", answer)

    assert page == pages.Fetched("https://example.com/a.json", "", '{"a": "<p>1</p>"}', False)


@pytest.mark.parametrize(
    "kind", ["text/plain", "application/xml", "application/ld+json", "image/svg+xml"]
)
async def test_text_of_every_kind_is_taken(kind: str) -> None:
    answer = responding(200, headers={"content-type": kind}, text="words")

    assert (await taken("https://example.com/", answer)).text == "words"


async def test_a_page_that_names_no_kind_is_read_as_html_where_it_looks_like_it() -> None:
    page = await taken("https://example.com/", responding(200, content=b"<HTML><p>hi</p>"))
    plain = await taken("https://example.com/", responding(200, content=b"<p>hi</p>"))

    assert page.text == "hi"
    assert plain.text == "<p>hi</p>"


async def test_a_picture_or_a_pdf_is_not_taken() -> None:
    answer = responding(200, headers={"content-type": "application/pdf"}, content=b"%PDF")

    failure = await refused("https://example.com/a.pdf", answer)

    assert failure.code == "NOT_TEXT"
    assert str(failure) == (
        "https://example.com/a.pdf is application/pdf, and this tool takes only text and pages "
        "of HTML."
    )


async def test_a_page_is_read_in_the_encoding_that_it_names() -> None:
    body = '<meta charset="gbk"><p>月亮</p>'.encode("gbk")
    named = await taken(
        "https://example.com/",
        responding(200, headers={"content-type": "text/html"}, content=body),
    )
    told = await taken(
        "https://example.com/",
        responding(
            200, headers={"content-type": "text/plain; charset=gbk"}, content="月".encode("gbk")
        ),
    )
    unknown = await taken(
        "https://example.com/",
        responding(
            200,
            headers={"content-type": "text/html"},
            content='<meta charset="nonsense"><p>月</p>'.encode(),
        ),
    )

    assert named.text == "月亮"
    assert told.text == "月"
    assert unknown.text == "月"


async def test_a_long_page_is_cut() -> None:
    answer = responding(200, text="x" * 50)

    page = await taken("https://example.com/", answer, most_bytes=10)

    assert page.text == "x" * 10
    assert page.cut


async def test_redirects_are_followed_and_each_is_checked() -> None:
    asked: list[str] = []

    async def look_up(host: str, port: int) -> list[str]:
        asked.append(f"{host}:{port}")
        return OUTSIDE

    def answer(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/old":
            return httpx.Response(301, headers={"location": "http://other.org/new"})
        return httpx.Response(200, text="here")

    page = await taken("https://example.com/old", answer, look_up=look_up)

    assert page.address == "http://other.org/new"
    assert asked == ["example.com:443", "other.org:80"]


async def test_a_redirect_into_this_machine_is_refused() -> None:
    async def look_up(host: str, port: int) -> list[str]:
        return ["127.0.0.1"] if host == "inside.example" else OUTSIDE

    answer = responding(302, headers={"location": "http://inside.example/"})

    failure = await refused("https://example.com/", answer, look_up=look_up)

    assert failure.code == "DENIED"


async def test_redirects_end_somewhere() -> None:
    answer = responding(302, headers={"location": "/again"})

    failure = await refused("https://example.com/", answer)

    assert failure.code == "TOO_MANY_REDIRECTS"


async def test_a_redirect_that_names_no_place_is_the_page() -> None:
    page = await taken("https://example.com/", responding(302, text="moved"))

    assert page.text == "moved"


@pytest.mark.parametrize(
    "address",
    [
        "127.0.0.1",
        "10.1.2.3",
        "192.168.1.1",
        "169.254.169.254",
        "::1",
        "fe80::1%3",
        "::ffff:127.0.0.1",
        "224.0.0.1",
        "100.64.0.1",
    ],
)
async def test_places_of_this_machine_and_its_networks_are_not_asked(address: str) -> None:
    async def look_up(host: str, port: int) -> list[str]:
        return [*OUTSIDE, address]

    failure = await refused("https://example.com/", html("hi"), look_up=look_up)

    assert failure.code == "DENIED"
    assert str(failure) == (
        "example.com leads to this machine or to a network of its own, which this tool does "
        "not ask."
    )


async def test_a_name_that_leads_nowhere_is_not_asked() -> None:
    async def nowhere(host: str, port: int) -> list[str]:
        return []

    async def unknown(host: str, port: int) -> list[str]:
        raise OSError("no such host")

    assert (await refused("https://example.com/", html("hi"), look_up=nowhere)).code == "DENIED"
    failure = await refused("https://example.com/", html("hi"), look_up=unknown)
    assert failure.code == "NOT_FOUND"
    assert str(failure) == "example.com could not be found: no such host"


async def test_the_config_may_let_places_inside_be_asked() -> None:
    async def never(host: str, port: int) -> list[str]:
        raise AssertionError("not looked up")

    page = await taken("http://localhost:8080/", html("<p>local</p>"), inside=True, look_up=never)

    assert page.text == "local"


@pytest.mark.parametrize(
    ("address", "said"),
    [
        ("file:///etc/passwd", "file:///etc/passwd is no address of the web"),
        ("example.com", "example.com is no address of the web"),
        ("https:///path", "https:///path names no host."),
    ],
)
async def test_an_address_that_is_no_page_of_the_web_is_refused(address: str, said: str) -> None:
    failure = await refused(address, html("hi"))

    assert failure.code == "INVALID_ARGS"
    assert str(failure).startswith(said)


async def test_an_error_of_http_is_said() -> None:
    answer = responding(404)

    failure = await refused("https://example.com/gone", answer)

    assert failure.code == "HTTP_ERROR"
    assert str(failure) == "https://example.com/gone answered 404 Not Found."


async def test_a_place_that_cannot_be_reached_is_said() -> None:
    def answer(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("refused")

    failure = await refused("https://example.com/", answer)

    assert failure.code == "UNREACHABLE"
    assert str(failure) == "https://example.com/ could not be reached: ConnectError: refused"


async def test_a_page_that_takes_too_long_is_given_up() -> None:
    async def slow(host: str, port: int) -> list[str]:
        await asyncio.sleep(5)
        return OUTSIDE

    failure = await refused("https://example.com/", html("hi"), seconds=0.01, look_up=slow)

    assert failure.code == "TIMEOUT"
    assert str(failure) == "https://example.com/ did not come within 0.01 seconds."


async def test_a_host_is_looked_up_on_this_machine() -> None:
    assert "127.0.0.1" in await pages.lookup("localhost", 80) or "::1" in await pages.lookup(
        "localhost", 80
    )
