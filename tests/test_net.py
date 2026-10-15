"""Where the requests that Columbina makes itself go: ``columbina.net``."""

from collections.abc import Mapping

import pytest

from columbina.net import proxy_for

CLASH = {"https": "http://127.0.0.1:7890", "http": "http://127.0.0.1:7891"}
"""What a tool such as Clash leaves in the settings of the system."""

OPENAI = "https://api.openai.com/v1"


def nothing_excepted(host: str) -> bool:
    return False


def through(url: str, said: Mapping[str, str], env: Mapping[str, str] | None = None) -> str | None:
    """The proxy of a request, where the system says ``said`` and lets no host go straight."""
    return proxy_for(url, env or {}, system=lambda: said, bypassed=nothing_excepted)


@pytest.mark.parametrize(
    "name", ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "ALL_PROXY", "all_proxy"]
)
def test_a_variable_of_the_environment_leaves_the_proxy_to_httpx(name: str) -> None:
    assert through(OPENAI, CLASH, {name: "http://10.0.0.1:3128"}) is None


def test_a_variable_that_is_empty_names_no_proxy() -> None:
    assert through(OPENAI, CLASH, {"HTTPS_PROXY": ""}) == CLASH["https"]


def test_the_proxy_of_the_system_is_taken_for_the_scheme_of_the_address() -> None:
    assert through(OPENAI, CLASH) == CLASH["https"]
    assert through("http://10.0.0.5:8000/v1", CLASH) == CLASH["http"]


def test_a_proxy_for_every_scheme_is_taken_where_there_is_none_for_this_one() -> None:
    assert through(OPENAI, {"all": "http://127.0.0.1:7892"}) == "http://127.0.0.1:7892"


@pytest.mark.parametrize("proxy", ["socks5://127.0.0.1:7892", "socks://127.0.0.1:1080"])
def test_a_proxy_that_httpx_does_not_speak_here_is_not_taken(proxy: str) -> None:
    assert through(OPENAI, {"https": proxy}) is None


def test_a_proxy_without_a_scheme_is_taken_for_one_of_http() -> None:
    assert through(OPENAI, {"https": "127.0.0.1:7890"}) == "http://127.0.0.1:7890"


def test_a_host_that_the_system_lets_go_straight_goes_straight() -> None:
    def excepted(host: str) -> bool:
        return host == "api.deepseek.com"

    deepseek = proxy_for("https://api.deepseek.com", {}, system=lambda: CLASH, bypassed=excepted)
    openai = proxy_for(OPENAI, {}, system=lambda: CLASH, bypassed=excepted)

    assert deepseek is None
    assert openai == CLASH["https"]


@pytest.mark.parametrize(
    "address", ["http://localhost:11434/v1", "http://127.0.0.1:1234/v1", "http://[::1]:8080/v1"]
)
def test_this_machine_is_reached_without_a_proxy(address: str) -> None:
    def asked() -> Mapping[str, str]:
        raise AssertionError("the system is not asked about this machine")

    assert proxy_for(address, {}, system=asked, bypassed=nothing_excepted) is None


def test_without_a_proxy_anywhere_a_request_goes_straight() -> None:
    assert through(OPENAI, {}) is None


def test_a_system_that_cannot_be_asked_lets_the_request_go_straight() -> None:
    def broken() -> Mapping[str, str]:
        raise OSError("the settings cannot be read")

    assert proxy_for(OPENAI, {}, system=broken, bypassed=nothing_excepted) is None


def test_the_system_is_asked_by_default() -> None:
    # Whatever this machine says: an address of a proxy, or nothing.
    found = proxy_for(OPENAI, {})

    assert found is None or "://" in found
