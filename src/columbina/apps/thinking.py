"""Which model an assistant asks, and how hard that model thinks, as its owner chose them.

An assistant in Feishu, WeCom or QQ talks to the service that Columbina uses, with its key. Which
model of that service it asks, and how hard the model thinks, may be chosen for each assistant
apart: a chat in QQ wants a quick answer, and work in Feishu may want a careful one. What is not
chosen follows what the user chose for the service, or else what the profile of the assistant
says.

Another program, such as the desktop application, changes the choice with a document of JSON
that holds ``model`` and ``effort`` and nothing else. ``null`` follows what holds for all.
"""

from collections.abc import Mapping
from typing import Literal, get_args

from columbina.llm.credentials import sendable

__all__ = ["EFFORTS", "Effort", "chosen", "is_a_choice"]

Effort = Literal["default", "off", "low", "high", "max"]
"""How hard the model thinks: as the service has it by default, not at all, or so much."""

EFFORTS: tuple[Effort, ...] = get_args(Effort)
"""The levels, from the one that asks for nothing to the one that asks for the most."""

_LONGEST_MODEL = 200
"""How long the id of a model may be, and more than any is."""


def is_a_choice(given: Mapping[str, object]) -> bool:
    """Tell whether a document says which model an assistant asks and how hard it thinks, alone."""
    return bool(given) and set(given) <= {"model", "effort"}


def chosen(given: Mapping[str, object]) -> tuple[str | None, Effort | None]:
    """Read the model and the level that a document chooses, ``None`` for what follows the rest.

    Raises:
        ValueError: What is given is no model or no level, said for a person.
    """
    model, effort = given.get("model"), given.get("effort")
    if model is not None:
        named = model.strip() if isinstance(model, str) else ""
        if not named or len(named) > _LONGEST_MODEL or not sendable(named):
            raise ValueError("a model is named by its id, as the service lists it")
        model = named
    if effort is not None and effort not in EFFORTS:
        raise ValueError(f"how hard the model thinks is one of {', '.join(EFFORTS)}")
    return model, effort
