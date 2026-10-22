"""The files that are sent to the bot: ``columbina.wecom.media``."""

import base64
from collections.abc import Callable

import httpx
import pytest

from columbina.wecom.app import WecomError
from columbina.wecom.media import Fetched, decrypted, fetched, named
from support.fake_wecom import KEY, encrypted

URL = "https://wework.qpic.cn/file/1"


def client(answer: Callable[[httpx.Request], httpx.Response]) -> httpx.AsyncClient:
    return httpx.AsyncClient(transport=httpx.MockTransport(answer))


# Decrypting


@pytest.mark.parametrize("size", [0, 1, 31, 32, 33, 1000])
def test_a_file_is_decrypted_with_its_key_whatever_its_size(size: int) -> None:
    content = bytes(index % 251 for index in range(size))

    assert decrypted(encrypted(content), KEY) == content


def test_a_key_is_taken_with_its_padding_and_with_blanks_around_it() -> None:
    padded = f" {KEY}= "

    assert decrypted(encrypted(b"notes"), padded) == b"notes"


@pytest.mark.parametrize(
    "key",
    ["", "not base64!", base64.b64encode(b"sixteen bytes!!!").decode()],
    ids=["nothing", "no base64", "too short"],
)
def test_a_key_that_is_no_key_of_aes_256_decrypts_nothing(key: str) -> None:
    with pytest.raises(WecomError) as refused:
        decrypted(encrypted(b"notes"), key)

    assert (refused.value.code, str(refused.value)) == (
        "UNREADABLE",
        "the key of the file is no key of AES-256",
    )


def test_what_was_not_encrypted_with_the_key_is_not_taken() -> None:
    other = base64.b64encode(bytes(range(1, 33))).decode()
    for content in (b"", b"short", encrypted(b"notes", other), encrypted(b"notes")[:-16]):
        with pytest.raises(WecomError) as refused:
            decrypted(content, KEY)
        assert refused.value.code == "UNREADABLE"


def test_a_filling_that_is_longer_than_what_it_fills_is_not_taken() -> None:
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

    raw = base64.b64decode(KEY + "=" * (-len(KEY) % 4))
    # Sixteen bytes that say that 32 were filled in.
    sealed = Cipher(algorithms.AES(raw), modes.CBC(raw[:16])).encryptor()
    content = sealed.update(bytes([32]) * 16) + sealed.finalize()

    with pytest.raises(WecomError):
        decrypted(content, KEY)


# Naming


@pytest.mark.parametrize(
    ("disposition", "name"),
    [
        ('attachment; filename="notes.md"', "notes.md"),
        ("attachment; filename=notes.md", "notes.md"),
        ("attachment; filename*=UTF-8''%E7%AC%94%E8%AE%B0.md", "笔记.md"),
        ("attachment; filename=\"a.txt\"; filename*=UTF-8''b.txt", "b.txt"),
        ('attachment; filename="%E7%AC%94.txt"', "笔.txt"),
        ("attachment", None),
        ('attachment; filename=""', None),
        ("attachment; filename=  ;", None),
        ("", None),
        (None, None),
    ],
)
def test_a_file_is_named_as_the_answer_of_its_download_names_it(
    disposition: str | None, name: str | None
) -> None:
    assert named(disposition) == name


# Downloading


async def test_a_file_is_downloaded_with_its_name() -> None:
    asked: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        asked.append(request)
        return httpx.Response(
            200, content=b"sealed", headers={"Content-Disposition": 'attachment; filename="a.md"'}
        )

    async with client(answer) as http:
        got = await fetched(http, URL, most=100)

    assert got == Fetched(b"sealed", "a.md")
    assert [str(request.url) for request in asked] == [URL]


async def test_a_file_without_a_name_has_none() -> None:
    async with client(lambda request: httpx.Response(200, content=b"x")) as http:
        assert (await fetched(http, URL, most=100)).name is None


async def test_a_file_is_taken_up_to_its_size_with_its_filling() -> None:
    async with client(lambda request: httpx.Response(200, content=b"x" * 132)) as http:
        assert len((await fetched(http, URL, most=100)).content) == 132
    async with client(lambda request: httpx.Response(200, content=b"x" * 133)) as http:
        with pytest.raises(WecomError) as large:
            await fetched(http, URL, most=100)

    assert (large.value.code, str(large.value)) == (
        "TOO_LARGE",
        "the file is larger than 100 bytes, more than is taken",
    )


async def test_a_file_that_lies_where_it_is_not_reached_safely_is_not_downloaded() -> None:
    asked: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        asked.append(request)
        return httpx.Response(200)

    async with client(answer) as http:
        with pytest.raises(WecomError) as unsafe:
            await fetched(http, "http://wework.qpic.cn/file/1", most=100)

    assert unsafe.value.code == "UNSAFE"
    assert asked == []


async def test_a_download_that_fails_says_why() -> None:
    async with client(lambda request: httpx.Response(404)) as http:
        with pytest.raises(WecomError) as missing:
            await fetched(http, URL, most=100)

    def late(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("late", request=request)

    def broken(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("refused", request=request)

    async with client(late) as http:
        with pytest.raises(WecomError) as timed_out:
            await fetched(http, URL, most=100)
    async with client(broken) as http:
        with pytest.raises(WecomError) as unreached:
            await fetched(http, URL, most=100)

    assert (missing.value.code, str(missing.value)) == (
        "DOWNLOAD",
        "the file could not be downloaded: HTTP 404",
    )
    assert str(timed_out.value) == "WeCom did not give the file in time"
    assert str(unreached.value) == "the file could not be downloaded: refused"
