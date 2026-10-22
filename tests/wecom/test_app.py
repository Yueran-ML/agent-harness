"""Where WeCom is: ``columbina.wecom.app``."""

import pytest

from columbina.wecom.app import ADDRESS, WecomError, address, safe_to_fetch


def test_the_long_connection_is_opened_at_wecom_unless_another_place_is_named() -> None:
    assert address({}) == ADDRESS == "wss://openws.work.weixin.qq.com"
    assert address({"COLUMBINA_WECOM_WS": ""}) == ADDRESS
    assert address({"COLUMBINA_WECOM_WS": "wss://wecom.example.com/ws/"}) == (
        "wss://wecom.example.com/ws"
    )
    # This machine may be reached without encryption, as a test does.
    assert address({"COLUMBINA_WECOM_WS": "ws://127.0.0.1:8765"}) == "ws://127.0.0.1:8765"
    assert address({"COLUMBINA_WECOM_WS": "ws://[::1]:8765"}) == "ws://[::1]:8765"


@pytest.mark.parametrize(
    "named", ["ws://wecom.example.com", "https://openws.work.weixin.qq.com", "openws"]
)
def test_a_place_that_is_not_reached_safely_is_not_taken(named: str) -> None:
    with pytest.raises(WecomError) as refused:
        address({"COLUMBINA_WECOM_WS": named})

    assert refused.value.code == "SETUP"
    assert str(refused.value).startswith(f"COLUMBINA_WECOM_WS names {named}.")


def test_a_file_is_downloaded_over_https_or_from_this_machine() -> None:
    assert safe_to_fetch("https://wework.qpic.cn/file/1")
    assert safe_to_fetch("http://localhost:8000/file")
    assert safe_to_fetch("http://127.0.0.1/file")
    assert not safe_to_fetch("http://wework.qpic.cn/file/1")
    assert not safe_to_fetch("ftp://127.0.0.1/file")
    assert not safe_to_fetch("file:///etc/passwd")


def test_an_error_says_its_code_and_the_number_of_wecom() -> None:
    error = WecomError("refused", code="REFUSED", number=853000)

    assert (str(error), error.code, error.number) == ("refused", "REFUSED", 853000)
    assert WecomError("x", code="X").number is None
