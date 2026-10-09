"""A plugin written as a module: it needs the clock and reports what it was given."""

from pydantic import BaseModel

from columbina.kernel import Context
from compose_demo.keys import CLOCK, REPORT

name = "reporter"
inject = [CLOCK]


class Config(BaseModel):
    label: str = "report"
    port: int = 0


def apply(ctx: Context, config: Config) -> None:
    ctx.provide(REPORT, {"label": config.label, "port": config.port, "now": ctx.use(CLOCK)})
