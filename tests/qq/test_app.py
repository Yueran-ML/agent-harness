"""The open platform of QQ over HTTP: ``columbina.qq.app``."""

import json
from typing import Any

import httpx
import pytest

from columbina.qq.app import API, TOKEN_URL, Api, QqError, token_url
from support.fake_qq import APP_ID, GATEWAY, SECRET, TOKEN, FakeApi


class Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


def api_of(fake: FakeApi, clock: Clock | None = None, env: dict[str, str] | None = None) -> Api:
    return Api(APP_ID, SECRET, env or {}, transport=fake.transport, clock=clock or Clock())


async def test_a_token_is_asked_for_once_and_again_before_it_runs_out() -> None:
    fake, clock = FakeApi(), Clock()
    api = api_of(fake, clock)

    assert await api.token() == TOKEN
    clock.now += 7200 - 61
    assert await api.token() == TOKEN
    assert fake.tokens == 1
    clock.now += 1
    await api.token()

    assert fake.tokens == 2
    first = fake.requests[0]
    assert str(first.url) == TOKEN_URL
    assert json.loads(first.content) == {"appId": APP_ID, "clientSecret": SECRET}
    assert "Authorization" not in first.headers
    await api.aclose()


async def test_a_token_that_qq_no_longer_takes_is_asked_for_anew() -> None:
    fake = FakeApi()
    api = api_of(fake)
    await api.token()

    api.forget_token()
    await api.token()

    assert fake.tokens == 2
    await api.aclose()


@pytest.mark.parametrize(
    "answer",
    [
        {"code": 100016, "message": "invalid appid or secret"},
        {"access_token": "", "expires_in": "7200"},
        {"access_token": TOKEN, "expires_in": "soon"},
        {"access_token": TOKEN, "expires_in": 0},
        {"access_token": TOKEN},
    ],
    ids=["refused", "empty", "no number", "none left", "no time"],
)
async def test_an_answer_that_gives_no_token_says_that_qq_does_not_take_the_bot(
    answer: dict[str, Any],
) -> None:
    transport = httpx.MockTransport(lambda request: httpx.Response(200, json=answer))
    api = Api(APP_ID, SECRET, {}, transport=transport)

    with pytest.raises(QqError, match="QQ does not take this AppID with this AppSecret") as raised:
        await api.token()

    assert raised.value.code == "REFUSED"
    await api.aclose()


async def test_the_gateway_is_asked_for_with_the_token() -> None:
    fake = FakeApi()
    api = api_of(fake)

    assert await api.gateway() == GATEWAY

    asked = fake.requests[-1]
    assert str(asked.url) == f"{API}/gateway"
    assert asked.headers["Authorization"] == f"QQBot {TOKEN}"
    assert asked.headers["X-Union-Appid"] == APP_ID
    await api.aclose()


async def test_a_gateway_of_no_address_is_said_to_be_so() -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/gateway":
            return httpx.Response(200, json={"shards": 1})
        return httpx.Response(200, json={"access_token": TOKEN, "expires_in": 7200})

    api = Api(APP_ID, SECRET, {}, transport=httpx.MockTransport(handle))

    with pytest.raises(QqError, match="QQ gave no address of its gateway") as raised:
        await api.gateway()

    assert raised.value.code == "MALFORMED"
    await api.aclose()


async def test_an_answer_goes_to_the_group_or_the_person_that_it_answers() -> None:
    fake = FakeApi()
    api = api_of(fake)

    await api.answer("在呢", message_id="m1", number=1, group="GROUP01", user=None)
    await api.answer("你好", message_id="m2", number=2, group=None, user="USER01")

    assert fake.answers == [
        (
            "/v2/groups/GROUP01/messages",
            {"content": "在呢", "msg_type": 0, "msg_id": "m1", "msg_seq": 1},
        ),
        (
            "/v2/users/USER01/messages",
            {"content": "你好", "msg_type": 0, "msg_id": "m2", "msg_seq": 2},
        ),
    ]
    await api.aclose()


async def test_other_addresses_may_be_named() -> None:
    fake = FakeApi()
    env = {
        "COLUMBINA_QQ_TOKEN_URL": "http://qq.test/app/getAppAccessToken",
        "COLUMBINA_QQ_API": "http://qq.test/",
    }
    api = api_of(fake, env=env)

    await api.gateway()

    assert [str(request.url) for request in fake.requests] == [
        "http://qq.test/app/getAppAccessToken",
        "http://qq.test/gateway",
    ]
    assert token_url(env) == "http://qq.test/app/getAppAccessToken"
    assert token_url({}) == TOKEN_URL
    await api.aclose()


@pytest.mark.parametrize(
    ("status", "body", "code", "number", "said"),
    [
        (
            400,
            {"code": 40034105, "message": "主动消息失败, 无权限"},
            "REFUSED",
            40034105,
            "主动消息失败",
        ),
        (503, {"message": "busy"}, "UNREACHABLE", None, "busy"),
        (404, None, "REFUSED", None, "Not Found"),
    ],
    ids=["refused", "busy", "no body"],
)
async def test_an_answer_that_qq_refuses_says_why(
    status: int, body: dict[str, Any] | None, code: str, number: int | None, said: str
) -> None:
    fake = FakeApi()
    if body is None:
        api = Api(
            APP_ID,
            SECRET,
            {},
            transport=httpx.MockTransport(
                lambda request: (
                    httpx.Response(200, json={"access_token": TOKEN, "expires_in": 7200})
                    if request.url.path == "/app/getAppAccessToken"
                    else httpx.Response(status, text="<html>")
                )
            ),
        )
    else:
        fake.refusing = (status, body)
        api = api_of(fake)

    with pytest.raises(QqError) as raised:
        await api.answer("hi", message_id="m1", number=1, group="GROUP01", user=None)

    assert (raised.value.code, raised.value.number) == (code, number)
    assert said in str(raised.value)
    assert f"HTTP {status}" in str(raised.value)
    await api.aclose()


async def test_a_token_that_qq_refuses_is_given_up() -> None:
    fake = FakeApi()
    api = api_of(fake)
    await api.token()
    fake.refusing = (401, {"code": 11241, "message": "token invalid"})

    with pytest.raises(QqError, match="token invalid"):
        await api.answer("hi", message_id="m1", number=1, group=None, user="U1")
    await api.token()

    assert fake.tokens == 2
    await api.aclose()


async def test_qq_that_cannot_be_reached_is_said_to_be_so_without_the_secret() -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError(f"no route for {SECRET}")

    api = Api(APP_ID, SECRET, {}, transport=httpx.MockTransport(handle))

    with pytest.raises(QqError) as raised:
        await api.token()

    assert raised.value.code == "UNREACHABLE"
    assert SECRET not in str(raised.value)
    assert "[secret]" in str(raised.value)
    await api.aclose()


async def test_the_token_is_taken_out_of_what_qq_says_too() -> None:
    fake = FakeApi()
    api = api_of(fake)
    await api.token()
    fake.refusing = (400, {"message": f"bad token {TOKEN}"})

    with pytest.raises(QqError) as raised:
        await api.answer("hi", message_id="m1", number=1, group=None, user="U1")

    assert TOKEN not in str(raised.value)
    await api.aclose()


async def test_an_error_without_a_body_or_a_reason_says_what_there_is() -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("")

    api = Api(APP_ID, SECRET, {}, transport=httpx.MockTransport(handle))

    with pytest.raises(QqError, match="ReadTimeout"):
        await api.token()
    await api.aclose()


def test_an_api_without_a_transport_of_a_test_goes_through_the_network() -> None:
    api = Api(APP_ID, SECRET, {"HTTPS_PROXY": "http://proxy.test:8080"})

    assert api.app_id == APP_ID
