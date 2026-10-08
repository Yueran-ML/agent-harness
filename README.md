# Tack

Tack is a plugin-based agent harness written in Python. Every capability is a plugin:
the model adapter, the tool registry, the session log, and the agent loop itself.

**Status:** early development. The plugin kernel is in place. The agent loop, the tools
and the command-line interface are being built on top of it, so there is nothing to
install and run yet.

Tack 是一个用 Python 写的插件化 Agent harness。模型适配、工具注册、会话日志和 Agent
循环都是插件。项目处于早期开发阶段：插件内核已经完成，Agent 循环、工具和命令行界面
正在其上搭建，目前还没有可以安装运行的程序。

## The kernel

`tack.kernel` is a small asyncio plugin framework. It is the only part of Tack that
exists today.

- A **context** is a service repository. A plugin offers a service with `provide` and
  reads one with `use`.
- A **plugin** declares the services it needs in `inject`. It loads when all of them
  are available, unloads when one is withdrawn, and loads again when it returns.
- Every registration made through a context is an **effect** that is undone when the
  plugin unloads, in reverse order.
- **Events** are declared with one of five dispatch modes: `emit`, `parallel`,
  `serial`, `bail` and `waterfall`.
- An application is **composed** from YAML layers of plugin entries. A later layer
  changes an entry by its id.

```python
import asyncio

from tack.kernel import Context, EmitEvent, Plugin, ServiceKey

CLOCK = ServiceKey[int]("clock")
TICK = EmitEvent[int]("demo/tick")


def announcer(ctx: Context, config: object) -> None:
    now = ctx.use(CLOCK)
    ctx.on(TICK, lambda step: print(f"tick {now + step}"))


async def main() -> None:
    root = Context()
    root.plugin(Plugin("announcer", announcer, inject=(CLOCK,)))

    root.emit(TICK, 1)  # nothing is printed: the plugin is waiting for the clock

    clock = root.provide(CLOCK, 100)
    await root.settled()
    root.emit(TICK, 1)  # prints "tick 101"

    await clock.dispose()  # the plugin unloads and its listener is removed
    root.emit(TICK, 2)  # nothing is printed

    await root.fiber.dispose()


asyncio.run(main())
```

## Development

Tack uses [uv](https://docs.astral.sh/uv/) for environments and needs Python 3.11 or newer.

```bash
uv sync
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Design notes live in `docs/plans/`.

## License

MIT. See [LICENSE](LICENSE).
