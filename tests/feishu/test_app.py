"""Where Feishu is: ``columbina.feishu.app``."""

import pytest

from columbina.feishu.app import DOMAINS, FeishuError, Host, host


def test_there_are_two_places_an_app_can_be_in() -> None:
    assert DOMAINS == ("feishu", "lark")


def test_feishu_and_lark_are_reached_at_their_own_hosts() -> None:
    assert host("feishu", {}) == Host("feishu", "https://open.feishu.cn")
    assert host("lark", {}) == Host("lark", "https://open.larksuite.com")


def test_the_console_is_where_the_user_makes_the_app() -> None:
    assert host("feishu", {}).console == "https://open.feishu.cn/app"
    assert host("lark", {}).console == "https://open.larksuite.com/app"


@pytest.mark.parametrize(
    "api", ["http://127.0.0.1:8123", "http://localhost:8123/", "https://feishu.example"]
)
def test_a_variable_puts_feishu_elsewhere(api: str) -> None:
    found = host("feishu", {"COLUMBINA_FEISHU_API": api})

    assert found.api == api.rstrip("/")
    assert found.console == "https://open.feishu.cn/app"


@pytest.mark.parametrize("api", ["http://feishu.example", "ftp://127.0.0.1"])
def test_feishu_is_not_reached_without_encryption_unless_it_is_on_this_machine(api: str) -> None:
    with pytest.raises(FeishuError) as refused:
        host("lark", {"COLUMBINA_FEISHU_API": api})

    assert refused.value.code == "SETUP"
    assert api in str(refused.value)
