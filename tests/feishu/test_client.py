"""The REST API of Feishu: ``columbina.feishu.client``."""

import json
from collections.abc import AsyncIterator, Callable

import httpx
import pytest

from columbina.feishu import client as client_module
from columbina.feishu.app import FeishuError, Host
from columbina.feishu.client import TOKEN, Feishu, field

HOST = Host("feishu", "https://open.feishu.cn")
SECRET = "s3cr3t-0123456789abcdef"
MESSAGES = "/open-apis/im/v1/messages"

Answer = Callable[[httpx.Request], httpx.Response]


def token_answer(token: str = "t-first", expire: object = 7200) -> httpx.Response:
    return httpx.Response(
        200, json={"code": 0, "msg": "ok", "tenant_access_token": token, "expire": expire}
    )


class Platform:
    """Feishu as a test has it answer: a token for every request for one, and the rest in a row."""

    def __init__(self, *answers: Answer, tokens: list[str] | None = None) -> None:
        self.answers = list(answers)
        self.tokens = tokens or ["t-first", "t-second", "t-third"]
        self.asked: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.asked.append(request)
        if request.url.path == TOKEN:
            return token_answer(self.tokens.pop(0))
        return self.answers.pop(0)(request)

    def calls(self) -> list[httpx.Request]:
        return [request for request in self.asked if request.url.path != TOKEN]


class Clock:
    def __init__(self) -> None:
        self.now = 1_000_000.0

    def __call__(self) -> float:
        return self.now


class Pauses:
    def __init__(self) -> None:
        self.taken: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.taken.append(seconds)


def feishu(platform: Platform, clock: Clock | None = None, pauses: Pauses | None = None) -> Feishu:
    return Feishu(
        HOST,
        "cli_a1b2c3",
        SECRET,
        {},
        transport=httpx.MockTransport(platform),
        now=clock or Clock(),
        sleep=pauses or Pauses(),
    )


def limited(seconds: str | None = None, code: int = 230020) -> Answer:
    headers = {} if seconds is None else {"x-ogw-ratelimit-reset": seconds}
    return lambda request: httpx.Response(
        400, json={"code": code, "msg": "too often"}, headers=headers
    )


async def test_a_request_that_may_be_made_again_is_made_again_after_a_pause() -> None:
    pauses = Pauses()
    platform = Platform(failing(429, b""), limited("5"), failing(503, b""), done({"ok": 1}))
    api = feishu(platform, pauses=pauses)

    assert await api.call("GET", MESSAGES, tries=4) == {"ok": 1}
    # Twice as long each time, unless Feishu says how long.
    assert pauses.taken == [2.0, 5.0, 8.0]
    await api.aclose()


async def test_a_request_is_made_no_more_often_than_it_may_be() -> None:
    pauses = Pauses()
    api = feishu(Platform(limited(), limited()), pauses=pauses)

    with pytest.raises(FeishuError) as refused:
        await api.call("GET", MESSAGES, tries=2)

    assert (refused.value.code, refused.value.number) == ("RATE_LIMITED", 230020)
    assert pauses.taken == [2.0]
    await api.aclose()


async def test_a_request_is_made_once_unless_it_may_be_made_again() -> None:
    pauses = Pauses()
    api = feishu(Platform(limited()), pauses=pauses)

    with pytest.raises(FeishuError):
        await api.call("POST", MESSAGES)

    assert pauses.taken == []
    await api.aclose()


async def test_what_does_not_pass_is_not_tried_again() -> None:
    pauses = Pauses()
    api = feishu(Platform(failing(400, {"code": 230025, "msg": "too large"})), pauses=pauses)

    with pytest.raises(FeishuError):
        await api.call("POST", MESSAGES, tries=3)

    assert pauses.taken == []
    await api.aclose()


@pytest.mark.parametrize(("said", "after"), [("100", 100.0), ("soon", None), ("-2", 0.0)])
async def test_the_pause_that_feishu_asks_for_is_known(said: str, after: float | None) -> None:
    api = feishu(Platform(limited(said, code=99991400)))

    with pytest.raises(FeishuError) as refused:
        await api.call("GET", MESSAGES)

    assert refused.value.after == after
    await api.aclose()


async def test_a_long_pause_is_made_no_longer_than_half_a_minute() -> None:
    pauses = Pauses()
    api = feishu(Platform(limited("100"), done({})), pauses=pauses)

    await api.call("GET", MESSAGES, tries=2)

    assert pauses.taken == [30.0]
    await api.aclose()


def done(data: object = None) -> Answer:
    body: dict[str, object] = {"code": 0, "msg": "success"}
    if data is not None:
        body["data"] = data
    return lambda request: httpx.Response(200, json=body)


def failing(status: int, body: object | bytes = b"") -> Answer:
    if isinstance(body, bytes):
        return lambda request: httpx.Response(status, content=body)
    return lambda request: httpx.Response(status, json=body)


def raising(error: Exception) -> Answer:
    def answer(request: httpx.Request) -> httpx.Response:
        raise error

    return answer


# The token


async def test_the_token_is_got_with_the_app_and_kept_while_it_holds() -> None:
    platform = Platform()
    clock = Clock()
    api = feishu(platform, clock)

    first = await api.token()
    clock.now += 7200 - 601
    again = await api.token()

    assert (first, again) == ("t-first", "t-first")
    asked = platform.asked[0]
    assert (asked.method, asked.url.path) == ("POST", TOKEN)
    assert json.loads(asked.content) == {"app_id": "cli_a1b2c3", "app_secret": SECRET}
    assert "authorization" not in asked.headers
    await api.aclose()


async def test_a_token_is_got_anew_ten_minutes_before_it_stops_holding() -> None:
    clock = Clock()
    api = feishu(Platform(), clock)

    await api.token()
    clock.now += 7200 - 599

    assert await api.token() == "t-second"
    await api.aclose()


async def test_a_token_that_feishu_turned_down_is_got_anew() -> None:
    api = feishu(Platform())

    turned = await api.token()

    assert await api.token(turned_down=turned) == "t-second"
    assert await api.token(turned_down="t-old") == "t-second"
    await api.aclose()


@pytest.mark.parametrize("given", [{"code": 0}, {"code": 0, "tenant_access_token": ""}])
async def test_an_answer_without_a_token_is_said_to_be_so(given: object) -> None:
    api = Feishu(
        HOST,
        "cli_a1b2c3",
        SECRET,
        {},
        transport=httpx.MockTransport(lambda request: httpx.Response(200, json=given)),
    )

    with pytest.raises(FeishuError) as failed:
        await api.token()

    assert failed.value.code == "UNKNOWN"
    await api.aclose()


async def test_a_token_without_a_lifetime_is_got_anew_each_time() -> None:
    tokens = iter(["t-a", "t-b"])
    api = Feishu(
        HOST,
        "cli_a1b2c3",
        SECRET,
        {},
        transport=httpx.MockTransport(lambda request: token_answer(next(tokens), expire="soon")),
    )

    assert [await api.token(), await api.token()] == ["t-a", "t-b"]
    await api.aclose()


async def test_an_app_that_feishu_gives_no_token_hears_why_without_its_secret() -> None:
    said = {"code": 10014, "msg": f"app secret invalid: {SECRET}"}
    api = Feishu(
        HOST,
        "cli_a1b2c3",
        SECRET,
        {},
        transport=httpx.MockTransport(lambda request: httpx.Response(200, json=said)),
    )

    with pytest.raises(FeishuError) as refused:
        await api.token()

    assert refused.value.code == "REFUSED"
    assert refused.value.number == 10014
    assert str(refused.value) == "Feishu did not do it: app secret invalid: [secret] (code 10014)"
    await api.aclose()


# Requests


async def test_a_request_goes_with_the_token_and_gives_the_data() -> None:
    platform = Platform(done({"message_id": "om_1"}))
    api = feishu(platform)

    data = await api.call(
        "POST", MESSAGES, params={"receive_id_type": "open_id"}, body={"receive_id": "ou_1"}
    )

    assert data == {"message_id": "om_1"}
    asked = platform.calls()[0]
    assert str(asked.url) == f"https://open.feishu.cn{MESSAGES}?receive_id_type=open_id"
    assert asked.headers["authorization"] == "Bearer t-first"
    assert asked.headers["content-type"] == "application/json; charset=utf-8"
    assert asked.headers["user-agent"].startswith("Columbina/")
    assert json.loads(asked.content) == {"receive_id": "ou_1"}
    await api.aclose()


@pytest.mark.parametrize("data", [None, [1], "x"])
async def test_an_answer_without_an_object_of_data_gives_none(data: object) -> None:
    api = feishu(Platform(done(data)))

    assert await api.call("GET", MESSAGES) == {}
    await api.aclose()


@pytest.mark.parametrize("gone", [99991661, 99991663, 99991664, 99991665])
async def test_a_token_that_holds_no_more_is_got_anew_and_the_request_made_again(
    gone: int,
) -> None:
    platform = Platform(failing(400, {"code": gone, "msg": "token invalid"}), done({"ok": 1}))
    api = feishu(platform)

    assert await api.call("GET", MESSAGES) == {"ok": 1}
    assert [request.headers["authorization"] for request in platform.calls()] == [
        "Bearer t-first",
        "Bearer t-second",
    ]
    await api.aclose()


async def test_a_token_that_holds_no_more_twice_is_said_to_be_so() -> None:
    gone = failing(400, {"code": 99991663, "msg": "token invalid"})
    api = feishu(Platform(gone, gone))

    with pytest.raises(FeishuError) as refused:
        await api.call("GET", MESSAGES)

    assert refused.value.number == 99991663
    await api.aclose()


@pytest.mark.parametrize(
    ("answer", "code", "said"),
    [
        (failing(429, b""), "RATE_LIMITED", "Feishu asks for a pause: 429"),
        (
            failing(400, {"code": 99991400, "msg": "request trigger frequency limit"}),
            "RATE_LIMITED",
            "Feishu asks for a pause: request trigger frequency limit",
        ),
        (failing(503, b"<html>"), "BUSY", "Feishu answered with HTTP 503. Try again in a moment"),
        (failing(400, b"not json"), "UNKNOWN", f"Feishu answered {MESSAGES} with HTTP 400"),
        (failing(200, {"code": True}), "UNKNOWN", f"Feishu answered {MESSAGES} with HTTP 200"),
        (
            failing(403, {"code": 1770032, "msg": "forbidden"}),
            "FORBIDDEN",
            "Feishu does not allow it: forbidden (code 1770032)",
        ),
        (
            failing(404, {"code": 230001}),
            "NOT_FOUND",
            "Feishu has nothing there: it gave no reason (code 230001)",
        ),
        (
            failing(400, {"code": 230025, "msg": "too large"}),
            "REFUSED",
            "Feishu did not do it: too large (code 230025)",
        ),
        (failing(500, {"code": 0, "msg": "odd"}), "REFUSED", "Feishu did not do it: odd (code 0)"),
    ],
)
async def test_what_feishu_did_not_do_is_said_with_its_reason(
    answer: Answer, code: str, said: str
) -> None:
    api = feishu(Platform(answer))

    with pytest.raises(FeishuError) as refused:
        await api.call("GET", MESSAGES)

    assert (refused.value.code, str(refused.value)) == (code, said)
    await api.aclose()


async def test_the_token_is_not_said_where_feishu_repeats_it() -> None:
    api = feishu(Platform(failing(400, {"code": 1, "msg": "bad token t-first"})))

    with pytest.raises(FeishuError) as refused:
        await api.call("GET", MESSAGES)

    assert "t-first" not in str(refused.value)
    await api.aclose()


@pytest.mark.parametrize(
    ("error", "code", "said"),
    [
        (httpx.ReadTimeout("slow"), "TIMEOUT", "Feishu did not answer in time."),
        (
            httpx.ConnectError(f"no route, {SECRET}"),
            "UNREACHABLE",
            "Feishu could not be reached: no route, [secret].",
        ),
    ],
)
async def test_feishu_that_cannot_be_reached_is_said_to_be_so(
    error: Exception, code: str, said: str
) -> None:
    api = feishu(Platform(raising(error)))

    with pytest.raises(FeishuError) as failed:
        await api.call("GET", MESSAGES)

    assert failed.value.code == code
    assert str(failed.value).startswith(said)
    assert str(failed.value).endswith("see that the proxy is on")
    await api.aclose()


# Downloads


class Chunks(httpx.AsyncByteStream):
    def __init__(self, *chunks: bytes) -> None:
        self.chunks = chunks

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk


async def test_a_file_is_downloaded_whole() -> None:
    path = "/open-apis/im/v1/messages/om_1/resources/file_1"
    platform = Platform(lambda request: httpx.Response(200, stream=Chunks(b"ab", b"cd")))
    api = feishu(platform)

    got = await api.bytes_of(path, {"type": "file"}, most=4)

    assert got == b"abcd"
    asked = platform.calls()[0]
    assert str(asked.url) == f"https://open.feishu.cn{path}?type=file"
    assert asked.headers["authorization"] == "Bearer t-first"
    await api.aclose()


async def test_a_file_larger_than_is_taken_is_not_downloaded() -> None:
    api = feishu(Platform(lambda request: httpx.Response(200, stream=Chunks(b"abc", b"de"))))

    with pytest.raises(FeishuError) as refused:
        await api.bytes_of("/f", {}, most=4)

    assert refused.value.code == "TOO_LARGE"
    await api.aclose()


async def test_a_file_that_feishu_does_not_give_is_said_to_be_so() -> None:
    api = feishu(Platform(failing(400, {"code": 234037, "msg": "too big"})))

    with pytest.raises(FeishuError) as refused:
        await api.bytes_of("/f", {}, most=4)

    assert (refused.value.code, refused.value.number) == ("REFUSED", 234037)
    await api.aclose()


async def test_a_download_with_a_token_that_holds_no_more_is_tried_once_more() -> None:
    platform = Platform(
        failing(400, {"code": 99991663, "msg": "token invalid"}),
        lambda request: httpx.Response(200, content=b"ok"),
    )
    api = feishu(platform)

    assert await api.bytes_of("/f", {}, most=4) == b"ok"
    assert platform.calls()[1].headers["authorization"] == "Bearer t-second"
    await api.aclose()


@pytest.mark.parametrize(
    ("error", "code"),
    [(httpx.ReadTimeout("slow"), "TIMEOUT"), (httpx.ConnectError("no route"), "UNREACHABLE")],
)
async def test_a_download_that_cannot_reach_feishu_is_said_to_be_so(
    error: Exception, code: str
) -> None:
    api = feishu(Platform(raising(error)))

    with pytest.raises(FeishuError) as failed:
        await api.bytes_of("/f", {}, most=4)

    assert failed.value.code == code
    await api.aclose()


# What is left


def test_a_path_through_json_gives_what_lies_there() -> None:
    said = {"data": {"node": {"obj_type": "docx"}}}

    assert field(said, "data", "node", "obj_type") == "docx"
    assert field(said, "data", "missing", "obj_type") is None
    assert field([1], "data") is None


async def test_feishu_is_asked_through_the_proxy_of_the_system(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    asked: list[str] = []

    def system(url: str, env: object) -> None:
        asked.append(url)

    monkeypatch.setattr(client_module, "proxy_for", system)
    api = Feishu(HOST, "cli_a1b2c3", SECRET, {})

    assert asked == ["https://open.feishu.cn"]
    await api.aclose()
