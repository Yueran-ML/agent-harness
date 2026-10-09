"""Plugins and non-plugins addressed as attributes of a module."""

from pydantic import BaseModel, field_validator

from columbina.kernel import Context, Plugin
from compose_demo.keys import CLOCK, STORE


class Fussy(BaseModel):
    number: int

    @field_validator("number", mode="before")
    @classmethod
    def reject(cls, value: object) -> object:
        raise TypeError(f"cannot read {value!r}")


def _cannot_start(ctx: Context, config: object) -> None:
    raise RuntimeError("cannot start")


def _nothing(ctx: Context, config: object) -> None:
    return None


broken = Plugin("broken", _cannot_start)

needy = Plugin("needy", _nothing, inject=(CLOCK, STORE))

fussy = Plugin("fussy", _nothing, config=Fussy)

not_a_plugin = 42
