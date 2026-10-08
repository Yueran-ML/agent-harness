"""A plugin written as a module: it provides the clock."""

from pydantic import BaseModel

from compose_demo.keys import CLOCK
from tack.kernel import Context

name = "clock"


class Config(BaseModel):
    now: int = 0


def apply(ctx: Context, config: Config) -> None:
    ctx.provide(CLOCK, config.now)
