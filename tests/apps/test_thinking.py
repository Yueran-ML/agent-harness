"""Which model an assistant asks, and how hard it thinks: ``columbina.apps.thinking``."""

import pytest

from columbina.apps.thinking import EFFORTS, chosen, is_a_choice


def test_the_levels_go_from_asking_nothing_to_asking_the_most() -> None:
    assert EFFORTS == ("default", "off", "low", "high", "max")


@pytest.mark.parametrize(
    ("given", "choice"),
    [
        ({"model": "deepseek-flash", "effort": "off"}, True),
        ({"model": None}, True),
        ({"effort": "high"}, True),
        ({}, False),
        ({"model": "m", "writable": True}, False),
        ({"app_id": "1", "secret": "s"}, False),
    ],
)
def test_a_choice_names_the_model_or_the_level_and_nothing_else(
    given: dict[str, object], choice: bool
) -> None:
    assert is_a_choice(given) is choice


def test_a_model_and_a_level_are_read_and_none_follows_what_holds_for_all() -> None:
    assert chosen({"model": " deepseek-v4-pro ", "effort": "max"}) == ("deepseek-v4-pro", "max")
    assert chosen({"model": None, "effort": None}) == (None, None)
    assert chosen({}) == (None, None)


@pytest.mark.parametrize("model", ["", "  ", 7, "x" * 201, "two words"])
def test_what_is_no_model_is_refused(model: object) -> None:
    with pytest.raises(ValueError, match="a model is named by its id"):
        chosen({"model": model})


@pytest.mark.parametrize("effort", ["ultra", "", 3, "MAX"])
def test_what_is_no_level_is_refused(effort: object) -> None:
    with pytest.raises(ValueError, match="one of default, off, low, high, max"):
        chosen({"effort": effort})
