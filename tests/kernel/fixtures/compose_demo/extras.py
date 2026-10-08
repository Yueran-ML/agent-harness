"""Plugins and non-plugins addressed as attributes of a module."""

from compose_demo.keys import CLOCK, STORE
from tack.kernel import Context, Plugin


def _cannot_start(ctx: Context, config: object) -> None:
    raise RuntimeError("cannot start")


def _nothing(ctx: Context, config: object) -> None:
    return None


broken = Plugin("broken", _cannot_start)

needy = Plugin("needy", _nothing, inject=(CLOCK, STORE))

not_a_plugin = 42
