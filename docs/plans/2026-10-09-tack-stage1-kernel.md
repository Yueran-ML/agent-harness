# Tack 第 1 阶段实施计划：仓库、工具链、内核

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 建好仓库和开发工具链，实现 Tack 的插件内核 `tack.kernel`，并用测试、类型检查和覆盖率闸门证明它可靠。

**Architecture:** 内核把 Cordis 的五个概念搬到 Python 的 asyncio 上。上下文是服务仓库，插件声明依赖并由内核决定何时加载和卸载，一切注册都可以撤销，事件在声明时固定五种分发方式之一，应用由分层的 YAML 清单组合而成。每个插件实例是一个 Fiber，它的加载和卸载在内核自己的后台任务里进行。

**Tech Stack:** Python 3.11 及以上、asyncio、pydantic v2、PyYAML。开发工具是 uv、hatchling、ruff、pyright（严格模式）、pytest、pytest-asyncio、pytest-timeout、hypothesis、coverage。

设计依据是 `docs/plans/2026-10-09-tack-design.md` 的第 3.4 节和第 4 节。

---

## 状态

本计划已经执行完毕。每个任务里的代码取自对应的提交，"Expected" 是重新检出该提交后在 Windows 11、Python 3.11.9 上量出来的结果。耗时每次不同，通过和失败的条数不变。

| 结束时的指标 | 数值 |
|---|---|
| 测试 | 245 通过，另有 3 个只在 Python 3.12 及以上运行。在 Python 3.14 上实测 248 通过 |
| 覆盖率（行与分支） | 100% |
| ruff、pyright 严格模式 | 零报错 |
| 内核代码 | 8 个模块，2065 行 |
| 测试代码 | 3862 行 |

三平台持续集成的工作流已经写好。它要等推送到 GitHub 之后才会第一次运行。

## 约定

- 仓库根目录是 `F:\agent harness\agent-harness`，所有命令在根目录执行。
- 每个任务的顺序是：写测试，看它失败，写实现，看它通过，提交。
- 新建的文件给出完整内容。修改的文件给出 unified diff，可以存成补丁文件后用 `git apply` 应用。
- `uv.lock` 由 `uv sync` 生成，计划里不列出它的内容，但要随任务 1 提交。
- 提交信息用英文。推送由作者用 GitHub Desktop 完成，计划里没有 `git push`。
- 用来确认"测试会失败"的命令都带 `--timeout=10`。异步代码出错时常见的表现是一直等待，超时能让失败干净地结束。

## 内核语义速查

后面的阶段都建立在这些规则上。每条规则都有测试。

**上下文与所有权**

- `Context()` 创建一个应用的根上下文。内核由 asyncio 驱动，必须在运行中的事件循环里使用。
- 每个上下文属于一个 Fiber。经它做的注册都记在这个 Fiber 上，Fiber 卸载时按注册的逆序逐个撤销。
- Fiber 有六个状态：`PENDING`、`LOADING`、`ACTIVE`、`FAILED`、`UNLOADING`、`DISPOSED`。根 Fiber 和作用域 Fiber 没有插件，创建时即为 `ACTIVE`。
- 每次状态变化都会发出 `FIBER_STATE_CHANGED` 事件。

**Effect**

- `ctx.effect(setup)` 立即执行 `setup`，并保存它返回的 disposer。Fiber 不在 `LOADING` 或 `ACTIVE` 时抛 `InactiveEffectError`，`setup` 不会被执行。
- 返回的 `EffectHandle` 可以提前撤销这个 effect。disposer 只运行一次。
- disposer 可以是普通函数，也可以返回可等待对象。后者在内核自己的任务里运行，调用方被取消不会打断它。
- 卸载时某个 disposer 抛异常，内核记日志并继续撤销其余的。

**服务**

- `ServiceKey[T]("name")` 是带类型的服务名，按名字比较。
- `ctx.provide(key, value)` 提供服务。同名服务只能有一个提供方，否则抛 `ServiceConflictError`。
- 提供方为 `ACTIVE` 时，服务对所有人可见。提供方自己和它下面的 Fiber 在提供方加载、卸载期间也能看到。
- `ctx.use(key)` 取服务。插件只能用自己提供的服务，或者自己及外层插件在 `inject` 里声明的服务，否则抛 `ServiceNotInjectedError`。服务不可用时抛 `ServiceUnavailableError`。
- `ctx.get(key)` 取可选服务。它不要求声明，服务不可用时返回 `None`。
- 撤回服务时，服务立即不可用，但名字保留到所有注入它的插件卸载完成。这期间再提供同名服务会抛 `ServiceConflictError`。

**插件与依赖**

- 插件有三种写法：`Plugin(name, apply, inject=..., config=...)`；带 `apply` 的模块或对象，可以带 `name`、`inject`、`Config`；一个裸的可调用对象。
- `ctx.plugin(plugin, config)` 立即返回 Fiber，加载在后台进行。`await fiber` 等它稳定下来，加载失败时抛出失败原因。
- 配置在创建 Fiber 时就用 pydantic 模型校验。校验失败的 Fiber 直接进入 `FAILED`，`apply` 不会被调用。
- `inject` 里的服务到齐后插件才加载。其中一个被撤回，插件卸载。服务回来，插件重新加载。
- `apply` 可以是协程函数，可以返回一个 disposer。`apply` 抛异常时，已做的注册全部回滚，Fiber 进入 `FAILED`。它依赖的某个服务被重新提供后，内核会再试一次。
- 提供方卸载时，先等依赖它的插件卸载完，再清理自己的资源。
- `await ctx.settled()` 等到整个应用里没有正在加载或卸载的插件。

**事件**

- 事件是模块级常量，有五种：`EmitEvent`、`ParallelEvent`、`SerialEvent`、`BailEvent`、`WaterfallEvent`。每个事件只带一个载荷对象。
- `ctx.on(event, listener, prepend=False)` 注册监听器。注册本身是一个 effect。
- `emit` 同步通知全部监听器，监听器的异常记日志后继续。`parallel` 并发执行并等待全部，异常同样被隔离。
- `serial` 逐个等待，返回第一个非 `None` 的结果。`bail` 是它的同步版本。两者把异常传给调用方。
- `waterfall(event, payload, final)` 把监听器当作包在 `final` 外面的中间件，第一个注册的在最外层。监听器调用 `next(payload)` 继续往下走。它可以换载荷，可以多次调用 `next` 来重试，不调用就是短路。
- 用错分发方式、把同一个名字声明成两种事件、给 `emit` 或 `bail` 事件注册协程函数，都会抛 `EventModeError`。

**作用域**

- `ctx.scope(key)` 返回一个带标记的上下文。经它注册的监听器只收到同一作用域内分发的事件，以及不带作用域分发的事件。
- 经带作用域的上下文分发事件，会送到该作用域的监听器和不带作用域的监听器。
- 在作用域里加载的插件继承这个作用域。作用域随创建它的 Fiber 一起销毁。

**取消与重入**

- 等待方被取消，不会打断正在进行的加载、卸载或清理。等待方指 `await fiber`、`fiber.dispose()`、`handle.dispose()` 和 `ctx.settled()` 的调用者。
- `apply` 或 disposer 自己抛出 `CancelledError`，按这段代码失败处理。
- 内核自己的任务名字以 `tack-fiber-` 或 `tack-effect-` 开头。这些任务如果被取消，Fiber 的状态保持真实，下一个等待方会把中断的工作接着做完。已经开始执行的清理被取消后无法续做，内核会记一条错误日志。
- `apply` 和 disposer 是"生命周期代码"。如果某个 Fiber 的卸载要等这段代码结束，这段代码就不能反过来等这个 Fiber。这时 `fiber.dispose()` 只发出请求就返回，`fiber.settled()` 立即返回。这条规则适用于 Fiber 自己的代码、它下面的 Fiber 的代码，以及注入了它所提供服务的插件的代码。
- 在生命周期代码里 `await ctx.settled()` 会抛 `LifecycleError`。

**组合**

- 一层（layer）是一组操作。`insert` 插入条目，带 `id` 的操作修改已有条目的 `plugin`、`config`、`disabled`。`config` 整体替换，不合并。
- 第一层可以用 `extends` 指定一个先应用的命名层，之后的层都是补丁。
- 条目的 `plugin` 是导入路径。`package.module` 表示模块本身是插件，`package.module:attribute` 表示模块里的一个对象。
- 配置里的字符串可以写 `${NAME}` 和 `${NAME:-default}`，`$$` 表示一个美元符号。
- `mount(ctx, entries)` 先导入全部插件、填好全部配置，再开始加载。前两步出错时，上下文保持原样。
- `Composition.problems()` 列出没有进入 `ACTIVE` 的条目和原因。

## 与设计文档的差异

实现时有七处细化。设计文档已在同一个提交里按下面的内容更新。

1. 服务访问写成 `ctx.use(KEY)` 和 `ctx.get(KEY)`，`KEY` 是 `ServiceKey[T]`。原设计写的是 `ctx.<name>`。带类型的键让 pyright 能检查服务的用法。
2. 每个事件只带一个载荷对象。`waterfall` 的监听器签名是 `(payload, next)`，`next(payload)` 可以多次调用。
3. 插件使用未声明的服务会报错。子插件要用父插件提供的服务，也要在 `inject` 里声明，并且在父插件进入 `ACTIVE` 之后才加载。
4. 服务被撤回期间名字保留，直到依赖方卸载完成。
5. 新增 `FIBER_STATE_CHANGED` 事件和 `LifecycleError`。
6. 内核为 2065 行，原设计估计约 600 行。多出来的部分主要在取消与重入的处理（`fiber.py`，802 行）和组合加载（`compose.py`，384 行）。
7. 清单条目用 `plugin` 字段写导入路径，层用 `extends` 引用另一个层。

## 任务总览

| # | 任务 | 仓库里的提交 | 任务结束时的全套测试 | 覆盖率 |
|---|---|---|---|---|
| 1 | 项目骨架与工具链 | `b810999` | 2 通过 | 100% |
| 2 | 内核的基础类型：错误、服务名、事件声明 | `1e205ef` | 19 通过 | 100% |
| 3 | Fiber 与可撤销的 effect | `8bfd41b` | 34 通过 | 95% |
| 4 | 同步事件：emit 与 bail | `ca1ad98` | 49 通过 | 96% |
| 5 | 异步事件：parallel、serial 与 waterfall | `a103455` | 62 通过 | 97% |
| 6 | 插件的加载与卸载 | `e760d7e` | 88 通过 | 99% |
| 7 | 服务的提供与查找 | `860d7b5` | 100 通过 | 99% |
| 8 | 依赖驱动的生命周期 | `fcdaf83` | 121 通过 | 100% |
| 9 | 作用域 | `537ef79` | 131 通过 | 100% |
| 10 | 生命周期的属性测试 | `a82d493` | 132 通过 | 100% |
| 11 | 加固：取消、重入与撤回时的竞态 | `23dc710`、`48b6e14`、`7fc007d`、`af7af12`、`52b935e`、`2cdf22c`、`f6e5157` | 182 通过, 3 跳过 | 100% |
| 12 | 组合：分层的插件清单 | `ed154d7`、`2cdf22c` | 241 通过, 3 跳过 | 100% |
| 13 | 持续集成、README 与示例 | `eeb562d` | 245 通过, 3 跳过 | 100% |

任务 1 到 10 各对应仓库里的一个提交。任务 11 到 13 的最终代码与仓库一致，但仓库里的提交是按复查的轮次分的，所以一个任务对应多个提交，`2cdf22c` 同时出现在任务 11 和任务 12 里。

任务 11 是对照 Codex 的四轮只读复查完成的。四轮共 32 条意见，逐条核实后采纳了 30 条。未采纳的两条写在文末。

---

### Task 1: 项目骨架与工具链

建立一个可以安装的包，以及之后每个任务都要通过的同一套检查。

**要点**

- 发行包名是 `tack-harness`，导入名是 `tack`。PyPI 上的 `tack` 已被一个 2017 年的占位包占用。
- 采用 `src/` 布局，构建后端是 hatchling。运行依赖这一阶段只有 pydantic 和 PyYAML，其余依赖到用到的阶段再加。
- pytest 把警告当作错误。没有被 await 的协程会因此让测试失败。
- 每个测试有 30 秒超时，防止异步死锁把测试挂住。
- 覆盖率同时统计行和分支，合计低于 90% 判为失败。
- ruff 不检查 `docs/`，否则它会改写文档里的代码块。

**Files:**

- Create: `pyproject.toml`
- Create: `.python-version`
- Create: `LICENSE`
- Create: `src/tack/__init__.py`
- Create: `src/tack/py.typed`
- Create: `README.md`
- Test（新建）: `tests/test_package.py`

**Step 1: 建立工程文件**

新建 `pyproject.toml`：

```toml
[build-system]
requires = ["hatchling>=1.27"]
build-backend = "hatchling.build"

[project]
name = "tack-harness"
version = "0.1.0.dev0"
description = "A plugin-based agent harness for Python."
readme = "README.md"
requires-python = ">=3.11"
license = "MIT"
license-files = ["LICENSE"]
authors = [{ name = "Yueran Ma" }]
keywords = ["agent", "harness", "llm", "tool-calling"]
classifiers = [
  "Development Status :: 2 - Pre-Alpha",
  "Intended Audience :: Developers",
  "Operating System :: OS Independent",
  "Programming Language :: Python :: 3",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Typing :: Typed",
]
dependencies = [
  "pydantic>=2.9",
  "pyyaml>=6.0.2",
]

[project.urls]
Repository = "https://github.com/Yueran-ML/agent-harness"

[dependency-groups]
dev = [
  "coverage>=7.6",
  "hypothesis>=6.115",
  "pyright>=1.1.400",
  "pytest>=8.3",
  "pytest-asyncio>=1.0",
  "pytest-timeout>=2.3",
  "ruff>=0.9",
  "types-pyyaml>=6.0.12",
]

[tool.hatch.build.targets.wheel]
packages = ["src/tack"]

[tool.ruff]
line-length = 100
target-version = "py311"
src = ["src", "tests"]
extend-exclude = ["docs"]

[tool.ruff.lint]
select = [
  "E",     # pycodestyle errors
  "W",     # pycodestyle warnings
  "F",     # pyflakes
  "I",     # import order
  "N",     # naming
  "D",     # docstrings
  "UP",    # modern syntax
  "B",     # likely bugs
  "C4",    # comprehensions
  "SIM",   # simplifications
  "PT",    # pytest style
  "PTH",   # pathlib over os.path
  "PERF",  # performance pitfalls
  "ASYNC", # asyncio pitfalls
  "RUF",   # ruff-specific rules
]
ignore = [
  "D105", # magic methods need no docstring
  "D107", # __init__ is documented on the class
]

[tool.ruff.lint.pydocstyle]
convention = "google"

[tool.ruff.lint.per-file-ignores]
"tests/**" = ["D"]

[tool.pyright]
include = ["src", "tests"]
pythonVersion = "3.11"
typeCheckingMode = "strict"
venvPath = "."
venv = ".venv"

[tool.pytest.ini_options]
minversion = "8.3"
testpaths = ["tests"]
addopts = ["-ra", "--strict-markers", "--strict-config", "--import-mode=importlib"]
asyncio_mode = "auto"
asyncio_default_fixture_loop_scope = "function"
timeout = 30
xfail_strict = true
filterwarnings = ["error"]

[tool.coverage.run]
branch = true
source_pkgs = ["tack"]

[tool.coverage.report]
fail_under = 90
show_missing = true
skip_covered = true
exclude_also = [
  "if TYPE_CHECKING:",
  "@overload",
  "raise NotImplementedError",
]
```

新建 `.python-version`：

```text
3.11
```

新建 `LICENSE`：

```text
MIT License

Copyright (c) 2026 Yueran Ma

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

新建 `README.md`：

````markdown
# Tack

Tack is a plugin-based agent harness written in Python. Every capability is a plugin:
the model adapter, the tool registry, the session log, and the agent loop itself.

**Status:** early development. Nothing is usable yet.

Tack 是一个用 Python 写的插件化 Agent harness。模型适配、工具注册、会话日志和 Agent
循环都是插件。项目处于早期开发阶段，目前还不能使用。

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
````

新建 `src/tack/__init__.py`。先只写一行文档字符串，版本号留到 Step 4：

```python
"""Tack: a plugin-based agent harness."""
```

**Step 2: 写冒烟测试并安装环境**

新建 `tests/test_package.py`：

```python
import re
from importlib.metadata import version

import tack


def test_version_matches_the_installed_distribution() -> None:
    assert tack.__version__ == version("tack-harness")


def test_version_looks_like_a_release_number() -> None:
    assert re.match(r"^\d+\.\d+\.\d+", tack.__version__)
```

Run: `uv sync`

Expected: 建出 `.venv` 和 `uv.lock`，安装列表里有 `tack-harness==0.1.0.dev0`。

**Step 3: 运行测试，确认失败**

Run: `uv run pytest -q`

Expected: `2 failed`。两个测试都报 `AttributeError: module 'tack' has no attribute '__version__'`。

**Step 4: 写实现**

把 `src/tack/__init__.py` 改成：

```python
"""Tack: a plugin-based agent harness."""

from importlib.metadata import version

__version__ = version("tack-harness")

__all__ = ["__version__"]
```

新建空文件 `src/tack/py.typed`。它告诉类型检查器这个包自带类型标注。

**Step 5: 运行测试和全部检查，确认通过**

Run:

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`2 passed in 0.68s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`3 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 6: 提交**

```bash
git add -A
git commit -m "build: scaffold the package and the development toolchain"
```

---

### Task 2: 内核的基础类型：错误、服务名、事件声明

先定下内核对外使用的名词。这个任务只有声明，没有行为。

**要点**

- `ServiceKey[T]` 的类型参数只给类型检查器看。两个名字相同的键指向同一个服务。
- 事件有五种，种类在声明时固定。`Event` 是它们的公共基类，每个子类带一个 `mode`。
- 监听器的类型别名和事件种类一一对应。`emit` 和 `bail` 的监听器是普通函数，其余三种可以是协程函数。
- 错误类都继承 `KernelError`。`EventModeError` 和 `InvalidPluginError` 同时是 `TypeError`，因为它们表示用法错误。

**Files:**

- Create: `src/tack/kernel/errors.py`
- Create: `src/tack/kernel/services.py`
- Create: `src/tack/kernel/events.py`
- Create: `src/tack/kernel/__init__.py`
- Test（新建）: `tests/kernel/test_declarations.py`

**Step 1: 写测试**

新建 `tests/kernel/test_declarations.py`：

```python
from dataclasses import FrozenInstanceError

import pytest

from tack.kernel import (
    BailEvent,
    CompositionError,
    EmitEvent,
    Event,
    EventModeError,
    InactiveEffectError,
    InvalidPluginError,
    KernelError,
    ParallelEvent,
    SerialEvent,
    ServiceConflictError,
    ServiceKey,
    ServiceNotInjectedError,
    ServiceUnavailableError,
    WaterfallEvent,
)


def test_service_keys_with_the_same_name_are_the_same_slot() -> None:
    assert ServiceKey[int]("clock") == ServiceKey[int]("clock")
    assert hash(ServiceKey[int]("clock")) == hash(ServiceKey[int]("clock"))
    assert ServiceKey[int]("clock") != ServiceKey[int]("timer")


def test_service_key_shows_its_name() -> None:
    assert repr(ServiceKey[int]("clock")) == "ServiceKey(name='clock')"


def test_service_key_is_immutable() -> None:
    key = ServiceKey[int]("clock")
    with pytest.raises(FrozenInstanceError):
        key.name = "timer"  # pyright: ignore[reportAttributeAccessIssue]


@pytest.mark.parametrize(
    ("kind", "mode"),
    [
        (EmitEvent, "emit"),
        (ParallelEvent, "parallel"),
        (SerialEvent, "serial"),
        (BailEvent, "bail"),
        (WaterfallEvent, "waterfall"),
    ],
)
def test_each_event_kind_has_a_fixed_dispatch_mode(kind: type[Event], mode: str) -> None:
    event = kind("demo/thing")
    assert event.mode == mode
    assert event.name == "demo/thing"
    assert repr(event) == f"{kind.__name__}('demo/thing')"


def test_event_declarations_accept_type_parameters() -> None:
    assert EmitEvent[int]("demo/count").name == "demo/count"
    assert WaterfallEvent[str, int]("demo/measure").mode == "waterfall"


@pytest.mark.parametrize(
    "error",
    [
        InactiveEffectError,
        ServiceConflictError,
        ServiceUnavailableError,
        ServiceNotInjectedError,
        EventModeError,
        InvalidPluginError,
        CompositionError,
    ],
)
def test_kernel_errors_share_a_base_class(error: type[Exception]) -> None:
    assert issubclass(error, KernelError)


def test_misuse_errors_are_also_type_errors() -> None:
    assert issubclass(EventModeError, TypeError)
    assert issubclass(InvalidPluginError, TypeError)
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_declarations.py -q --timeout=10`

Expected: `1 error in 0.58s`，第一个错误是 `ModuleNotFoundError: No module named 'tack.kernel'`

**Step 3: 写实现**

新建 `src/tack/kernel/errors.py`：

```python
"""Errors raised by the kernel."""


class KernelError(Exception):
    """Base class of every error the kernel raises deliberately."""


class InactiveEffectError(KernelError):
    """An effect was registered on a plugin instance that is not loaded."""


class ServiceConflictError(KernelError):
    """A service was provided while another provider already holds its name."""


class ServiceUnavailableError(KernelError):
    """A service was requested while no active plugin provides it."""


class ServiceNotInjectedError(KernelError):
    """A plugin used a service that it did not declare in ``inject``."""


class EventModeError(KernelError, TypeError):
    """An event was dispatched or subscribed in a way its kind does not allow."""


class InvalidPluginError(KernelError, TypeError):
    """An object passed to ``Context.plugin`` is not a plugin."""


class CompositionError(KernelError):
    """A composition layer is malformed or cannot be applied."""
```

新建 `src/tack/kernel/services.py`：

```python
"""Typed service names."""

from dataclasses import dataclass
from typing import Generic, TypeVar

ServiceT = TypeVar("ServiceT")


@dataclass(frozen=True, slots=True)
class ServiceKey(Generic[ServiceT]):
    """The name of a service slot.

    The type parameter records what the slot holds, so that type checkers can
    follow a service from ``provide`` to ``use``. Keys compare by name: two keys
    with the same name address the same slot.
    """

    name: str
```

新建 `src/tack/kernel/events.py`：

```python
"""Event declarations.

An event is declared once, as a module-level constant of one of five kinds. The
kind fixes how the event is dispatched. Every event carries a single payload.
"""

from collections.abc import Awaitable, Callable
from typing import ClassVar, Generic, TypeAlias, TypeVar

PayloadT = TypeVar("PayloadT")
ResultT = TypeVar("ResultT")


class Event:
    """Base class of the five event kinds."""

    __slots__ = ("name",)

    mode: ClassVar[str]

    def __init__(self, name: str) -> None:
        self.name = name

    def __repr__(self) -> str:
        return f"{type(self).__name__}({self.name!r})"


class EmitEvent(Event, Generic[PayloadT]):
    """A synchronous notification.

    Listeners run in order and return nothing. A listener that raises is logged
    and does not stop the others.
    """

    __slots__ = ()
    mode = "emit"


class ParallelEvent(Event, Generic[PayloadT]):
    """A notification whose listeners run concurrently and are all awaited.

    A listener that raises is logged and does not stop the others.
    """

    __slots__ = ()
    mode = "parallel"


class SerialEvent(Event, Generic[PayloadT, ResultT]):
    """A question asked of each listener in turn, awaiting each one.

    The first listener to return something other than ``None`` answers it.
    """

    __slots__ = ()
    mode = "serial"


class BailEvent(Event, Generic[PayloadT, ResultT]):
    """The synchronous form of :class:`SerialEvent`."""

    __slots__ = ()
    mode = "bail"


class WaterfallEvent(Event, Generic[PayloadT, ResultT]):
    """A chain of middleware around a final handler.

    Each listener receives the payload and a ``next`` function. Calling
    ``next`` runs the rest of the chain; returning without calling it
    short-circuits the chain.
    """

    __slots__ = ()
    mode = "waterfall"


EmitListener: TypeAlias = Callable[[PayloadT], None]
ParallelListener: TypeAlias = Callable[[PayloadT], Awaitable[None] | None]
SerialListener: TypeAlias = Callable[[PayloadT], Awaitable[ResultT | None] | ResultT | None]
BailListener: TypeAlias = Callable[[PayloadT], ResultT | None]
Next: TypeAlias = Callable[[PayloadT], Awaitable[ResultT]]
WaterfallListener: TypeAlias = Callable[[PayloadT, Next[PayloadT, ResultT]], Awaitable[ResultT]]
```

新建 `src/tack/kernel/__init__.py`：

```python
"""The plugin kernel: contexts, services, effects, events, and plugin instances."""

from tack.kernel.errors import (
    CompositionError,
    EventModeError,
    InactiveEffectError,
    InvalidPluginError,
    KernelError,
    ServiceConflictError,
    ServiceNotInjectedError,
    ServiceUnavailableError,
)
from tack.kernel.events import (
    BailEvent,
    BailListener,
    EmitEvent,
    EmitListener,
    Event,
    Next,
    ParallelEvent,
    ParallelListener,
    SerialEvent,
    SerialListener,
    WaterfallEvent,
    WaterfallListener,
)
from tack.kernel.services import ServiceKey

__all__ = [
    "BailEvent",
    "BailListener",
    "CompositionError",
    "EmitEvent",
    "EmitListener",
    "Event",
    "EventModeError",
    "InactiveEffectError",
    "InvalidPluginError",
    "KernelError",
    "Next",
    "ParallelEvent",
    "ParallelListener",
    "SerialEvent",
    "SerialListener",
    "ServiceConflictError",
    "ServiceKey",
    "ServiceNotInjectedError",
    "ServiceUnavailableError",
    "WaterfallEvent",
    "WaterfallListener",
]
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_declarations.py -q`

Expected: `17 passed in 0.47s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`19 passed in 0.72s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`8 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): declare errors, service keys and the five event kinds"
```

---

### Task 3: Fiber 与可撤销的 effect

实现所有权模型的最小形态：根上下文、根 Fiber、effect 及其逆序撤销。

**要点**

- Fiber 是所有权单位。根 Fiber 一创建就是 `ACTIVE`。
- `effect(setup)` 先检查 Fiber 是否已加载，再执行 `setup`。这样不会留下一个撤销不掉的副作用。
- 卸载在一个后台任务里进行，这个任务叫驱动。`dispose()` 用 `asyncio.shield` 等它，所以取消等待方不会打断卸载。
- `EffectHandle.dispose()` 是幂等的。并发的多次调用共享同一次清理。
- 这里的 `fiber.py` 是简化版。任务 6、7、8 会扩展它，任务 11 会重写它的大部分。
- `fiber.py` 和 `context.py` 开头有 `# pyright: reportPrivateUsage=false`。Fiber、EffectHandle 和 KernelState 互相访问下划线成员，这一行相当于声明它们互为友元。

**Files:**

- Create: `src/tack/kernel/fiber.py`
- Create: `src/tack/kernel/context.py`
- Modify: `src/tack/kernel/__init__.py`
- Test（新建）: `tests/kernel/conftest.py`
- Test（新建）: `tests/kernel/test_effects.py`

**Step 1: 写测试**

新建 `tests/kernel/conftest.py`：

```python
from collections.abc import AsyncIterator

import pytest

from tack.kernel import Context


@pytest.fixture
async def ctx() -> AsyncIterator[Context]:
    """A root context that is disposed when the test ends."""
    root = Context()
    yield root
    await root.fiber.dispose()
```

新建 `tests/kernel/test_effects.py`：

```python
import asyncio
import logging
from collections.abc import Awaitable, Callable

import pytest

from tack.kernel import Context, FiberState, InactiveEffectError


def recorder(log: list[str], label: str) -> Callable[[], None]:
    def undo() -> None:
        log.append(label)

    return undo


def broken() -> None:
    raise RuntimeError("boom")


async def test_the_root_fiber_is_active(ctx: Context) -> None:
    assert ctx.fiber.state is FiberState.ACTIVE
    assert ctx.fiber.name == "root"
    assert ctx.fiber.parent is None
    assert ctx.fiber.ctx is ctx


async def test_effect_runs_its_setup_immediately(ctx: Context) -> None:
    log: list[str] = []
    ctx.effect(lambda: log.append("setup"))
    assert log == ["setup"]


async def test_disposing_the_fiber_undoes_effects_in_reverse_order() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "first"))
    ctx.effect(lambda: recorder(log, "second"))

    await ctx.fiber.dispose()

    assert log == ["second", "first"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_async_disposers_are_awaited_one_at_a_time() -> None:
    ctx = Context()
    log: list[str] = []

    def slow(label: str) -> Callable[[], Awaitable[None]]:
        async def undo() -> None:
            log.append(f"{label} start")
            await asyncio.sleep(0)
            log.append(f"{label} end")

        return undo

    ctx.effect(lambda: slow("first"))
    ctx.effect(lambda: slow("second"))

    await ctx.fiber.dispose()

    assert log == ["second start", "second end", "first start", "first end"]


async def test_a_handle_undoes_its_effect_once(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.effect(lambda: recorder(log, "undo"))
    assert not handle.disposed

    await handle.dispose()
    await handle.dispose()

    assert log == ["undo"]
    assert handle.disposed


async def test_an_effect_disposed_early_is_not_undone_again() -> None:
    ctx = Context()
    log: list[str] = []
    handle = ctx.effect(lambda: recorder(log, "undo"))

    await handle.dispose()
    await ctx.fiber.dispose()

    assert log == ["undo"]


async def test_disposing_an_older_effect_leaves_newer_ones_in_place() -> None:
    ctx = Context()
    log: list[str] = []
    older = ctx.effect(lambda: recorder(log, "older"))
    ctx.effect(lambda: recorder(log, "newer"))

    await older.dispose()
    assert log == ["older"]

    await ctx.fiber.dispose()
    assert log == ["older", "newer"]


async def test_concurrent_disposals_share_one_run(ctx: Context) -> None:
    gate = asyncio.Event()
    runs = 0

    async def undo() -> None:
        nonlocal runs
        runs += 1
        await gate.wait()

    handle = ctx.effect(lambda: undo)
    first = asyncio.ensure_future(handle.dispose())
    second = asyncio.ensure_future(handle.dispose())
    await asyncio.sleep(0)
    assert not first.done()
    assert not second.done()

    gate.set()
    await asyncio.gather(first, second)

    assert runs == 1


async def test_disposing_a_handle_directly_raises_its_error(ctx: Context) -> None:
    handle = ctx.effect(lambda: broken)

    with pytest.raises(RuntimeError, match="boom"):
        await handle.dispose()

    assert handle.disposed


async def test_a_failing_disposer_does_not_stop_the_others(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "first"))
    ctx.effect(lambda: broken)
    ctx.effect(lambda: recorder(log, "third"))

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await ctx.fiber.dispose()

    assert log == ["third", "first"]
    assert "boom" in caplog.text


async def test_a_setup_that_raises_registers_nothing() -> None:
    ctx = Context()

    with pytest.raises(RuntimeError, match="boom"):
        ctx.effect(broken)

    await ctx.fiber.dispose()
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_effects_are_rejected_after_disposal() -> None:
    ctx = Context()
    await ctx.fiber.dispose()
    log: list[str] = []

    with pytest.raises(InactiveEffectError, match="disposed"):
        ctx.effect(lambda: log.append("setup"))

    assert log == []


async def test_effects_are_rejected_while_the_fiber_unloads() -> None:
    ctx = Context()
    seen: list[FiberState] = []
    errors: list[InactiveEffectError] = []

    def late() -> None:
        seen.append(ctx.fiber.state)
        try:
            ctx.effect(lambda: None)
        except InactiveEffectError as error:
            errors.append(error)

    ctx.effect(lambda: late)
    await ctx.fiber.dispose()

    assert seen == [FiberState.UNLOADING]
    assert len(errors) == 1


async def test_disposing_a_fiber_twice_is_harmless() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "undo"))

    await asyncio.gather(ctx.fiber.dispose(), ctx.fiber.dispose())
    await ctx.fiber.dispose()

    assert log == ["undo"]


async def test_a_cancelled_waiter_does_not_interrupt_the_unload() -> None:
    ctx = Context()
    gate = asyncio.Event()
    log: list[str] = []

    async def undo() -> None:
        await gate.wait()
        log.append("undone")

    ctx.effect(lambda: undo)
    waiter = asyncio.ensure_future(ctx.fiber.dispose())
    await asyncio.sleep(0)
    waiter.cancel()
    with pytest.raises(asyncio.CancelledError):
        await waiter

    gate.set()
    await ctx.fiber.dispose()

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_effects.py -q --timeout=10`

Expected: `E   ImportError: cannot import name 'Context' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py)`，第一个错误是 `ImportError: cannot import name 'Context' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py)`

**Step 3: 写实现**

新建 `src/tack/kernel/fiber.py`：

```python
"""Fibers and the effects they own.

A fiber is the kernel's unit of ownership. Everything registered through a
context is recorded as an effect on the fiber that owns the context, and is
undone when that fiber unloads.
"""

# Fibers, effect handles and the shared kernel state reach into one another.
# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
import enum
import inspect
import itertools
import logging
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING, TypeAlias

from tack.kernel.errors import InactiveEffectError

if TYPE_CHECKING:
    from tack.kernel.context import Context

logger = logging.getLogger("tack.kernel")

Disposer: TypeAlias = Callable[[], Awaitable[None] | None]
"""Undoes one effect. It may be a plain function or a coroutine function."""

Epoch: TypeAlias = tuple[int, ...]
"""Identifies the set of providers a fiber is loaded against."""


class FiberState(enum.Enum):
    """Where a fiber is in its lifecycle."""

    PENDING = "pending"
    LOADING = "loading"
    ACTIVE = "active"
    FAILED = "failed"
    UNLOADING = "unloading"
    DISPOSED = "disposed"


class KernelState:
    """State shared by every fiber and context of one application."""

    __slots__ = ("_uids", "drivers")

    def __init__(self) -> None:
        self._uids = itertools.count()
        self.drivers: set[asyncio.Task[None]] = set()

    def next_uid(self) -> int:
        """Return a fiber identifier that has not been used before."""
        return next(self._uids)


class EffectHandle:
    """A registered effect. Disposing the handle undoes the effect once."""

    __slots__ = ("_completion", "_disposer", "_fiber")

    def __init__(self, fiber: Fiber, disposer: Disposer | None) -> None:
        self._fiber = fiber
        self._disposer = disposer
        self._completion: asyncio.Future[None] | None = None

    @property
    def disposed(self) -> bool:
        """Whether the effect has been undone."""
        return self._completion is not None and self._completion.done()

    async def dispose(self) -> None:
        """Undo the effect.

        The first call runs the disposer and re-raises whatever it raises. A
        call that arrives later, or while the disposer is still running, waits
        for that one run and returns.
        """
        if self._completion is not None:
            await asyncio.shield(self._completion)
            return
        completion = self._completion = asyncio.get_running_loop().create_future()
        try:
            if self._disposer is not None:
                result = self._disposer()
                if inspect.isawaitable(result):
                    await result
        finally:
            self._fiber._forget(self)
            completion.set_result(None)


class Fiber:
    """The owner of a set of effects.

    Disposing a fiber undoes its effects in reverse order of registration, one
    at a time, and waits for each to finish.
    """

    __slots__ = (
        "_disposed",
        "_driver",
        "_effects",
        "_kernel",
        "_loaded",
        "_wanted",
        "ctx",
        "parent",
        "state",
        "uid",
    )

    ctx: Context

    def __init__(self, kernel: KernelState, parent: Fiber | None) -> None:
        self.uid = kernel.next_uid()
        self.parent = parent
        self.state = FiberState.ACTIVE
        self._kernel = kernel
        self._effects: list[EffectHandle] = []
        self._wanted: Epoch | None = ()
        self._loaded: Epoch | None = ()
        self._driver: asyncio.Task[None] | None = None
        self._disposed = False

    @property
    def name(self) -> str:
        """A short label for logs and error messages."""
        return "root"

    def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
        """Run ``setup`` now and keep the disposer it returns.

        Raises:
            InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
        """
        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
            raise InactiveEffectError(
                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
            )
        handle = EffectHandle(self, setup())
        self._effects.append(handle)
        return handle

    async def dispose(self) -> None:
        """Unload the fiber for good and wait until its effects are undone."""
        self._retire()
        await self.settled()

    async def settled(self) -> None:
        """Wait until no load or unload of this fiber is in flight."""
        while (driver := self._driver) is not None and driver is not asyncio.current_task():
            await asyncio.shield(driver)

    def _retire(self) -> None:
        if self._disposed:
            return
        self._disposed = True
        self._refresh()

    def _refresh(self) -> None:
        self._wanted = None if self._disposed else ()
        self._kick()

    def _needs_work(self) -> bool:
        return self._loaded is not None and self._loaded != self._wanted

    def _kick(self) -> None:
        if self._driver is not None:
            return
        if not self._needs_work():
            self._rest()
            return
        loop = asyncio.get_running_loop()
        driver = loop.create_task(self._drive(), name=f"tack-fiber-{self.uid}")
        self._driver = driver
        self._kernel.drivers.add(driver)

    async def _drive(self) -> None:
        try:
            while self._needs_work():
                await self._unload()
        finally:
            driver = self._driver
            self._driver = None
            if driver is not None:
                self._kernel.drivers.discard(driver)
            self._rest()

    async def _unload(self) -> None:
        self._set_state(FiberState.UNLOADING)
        await self._undo_effects()
        self._loaded = None

    async def _undo_effects(self) -> None:
        while self._effects:
            try:
                await self._effects[-1].dispose()
            except Exception:
                logger.exception("an effect of %r failed to clean up", self.name)

    def _forget(self, handle: EffectHandle) -> None:
        effects = self._effects
        if effects and effects[-1] is handle:
            effects.pop()
        elif handle in effects:
            effects.remove(handle)

    def _rest(self) -> None:
        if self._loaded is not None:
            self._set_state(FiberState.ACTIVE)
        elif self._disposed:
            self._set_state(FiberState.DISPOSED)
        else:
            self._set_state(FiberState.PENDING)

    def _set_state(self, state: FiberState) -> None:
        self.state = state
```

新建 `src/tack/kernel/context.py`：

```python
"""The context: the handle through which code reaches the kernel."""

# pyright: reportPrivateUsage=false

from __future__ import annotations

from collections.abc import Callable

from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState


class Context:
    """The handle through which code reaches the kernel.

    Everything registered through a context belongs to the fiber that owns the
    context and is undone when that fiber unloads. Calling ``Context()``
    creates the root context of a new application.
    """

    __slots__ = ("_kernel", "fiber")

    def __init__(self) -> None:
        self._kernel = KernelState()
        self.fiber = Fiber(self._kernel, None)
        self.fiber.ctx = self

    def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
        """Run ``setup`` now and undo it when the owning fiber unloads.

        ``setup`` performs a registration and returns the function that undoes
        it, or ``None`` when there is nothing to undo. The returned handle can
        undo the effect earlier.

        Raises:
            InactiveEffectError: The owning fiber is not loaded. ``setup`` is
                not run.
        """
        return self.fiber.effect(setup)
```

修改 `src/tack/kernel/__init__.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/__init__.py
+++ b/src/tack/kernel/__init__.py
@@ -1,5 +1,6 @@
 """The plugin kernel: contexts, services, effects, events, and plugin instances."""
 
+from tack.kernel.context import Context
 from tack.kernel.errors import (
     CompositionError,
     EventModeError,
@@ -24,16 +25,22 @@ from tack.kernel.events import (
     WaterfallEvent,
     WaterfallListener,
 )
+from tack.kernel.fiber import Disposer, EffectHandle, Fiber, FiberState
 from tack.kernel.services import ServiceKey
 
 __all__ = [
     "BailEvent",
     "BailListener",
     "CompositionError",
+    "Context",
+    "Disposer",
+    "EffectHandle",
     "EmitEvent",
     "EmitListener",
     "Event",
     "EventModeError",
+    "Fiber",
+    "FiberState",
     "InactiveEffectError",
     "InvalidPluginError",
     "KernelError",
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_effects.py -q`

Expected: `15 passed in 0.58s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`34 passed in 0.76s`
- 覆盖率合计：`95%`
- ruff check：`All checks passed!`
- ruff format：`12 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): add fibers, reversible effects and the root context"
```

---

### Task 4: 同步事件：emit 与 bail

实现监听器的注册和两种同步分发。

**要点**

- 监听器的注册是一个 effect，随所属 Fiber 卸载而移除。
- `emit` 隔离监听器的异常并记日志。`bail` 返回第一个非 `None` 的结果，异常传给调用方。
- 分发时先对监听器列表取快照。分发过程中新注册的监听器要等下一次分发才生效。
- 同一个事件名只能属于一种事件。用错分发方式和重复声明都抛 `EventModeError`。
- 这个任务同时在 pyright 配置里打开 `reportUnnecessaryTypeIgnoreComment`。测试里故意写错的调用靠 `# pyright: ignore[...]` 放行，一旦某处不再报错，多余的 ignore 会让类型检查失败。

**Files:**

- Modify: `pyproject.toml`
- Modify: `src/tack/kernel/events.py`
- Modify: `src/tack/kernel/fiber.py`
- Modify: `src/tack/kernel/context.py`
- Test（新建）: `tests/kernel/test_events_sync.py`

**Step 1: 写测试**

新建 `tests/kernel/test_events_sync.py`：

```python
import logging

import pytest

from tack.kernel import BailEvent, Context, EmitEvent, EventModeError

PING = EmitEvent[str]("test/ping")
ASK = BailEvent[str, int]("test/ask")


async def test_emit_calls_listeners_in_registration_order(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(PING, lambda payload: log.append(f"a:{payload}"))
    ctx.on(PING, lambda payload: log.append(f"b:{payload}"))

    ctx.emit(PING, "x")

    assert log == ["a:x", "b:x"]


async def test_prepend_puts_a_listener_first(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(PING, lambda payload: log.append("a"))
    ctx.on(PING, lambda payload: log.append("b"), prepend=True)

    ctx.emit(PING, "x")

    assert log == ["b", "a"]


async def test_emit_without_listeners_does_nothing(ctx: Context) -> None:
    ctx.emit(PING, "x")


async def test_a_handle_removes_its_listener(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.on(PING, log.append)
    ctx.on(PING, lambda payload: log.append("kept"))

    await handle.dispose()
    ctx.emit(PING, "x")

    assert log == ["kept"]


async def test_listeners_are_removed_when_their_fiber_is_disposed() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.on(PING, log.append)

    await ctx.fiber.dispose()
    ctx.emit(PING, "x")

    assert log == []


async def test_emit_logs_a_failing_listener_and_continues(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    def failing(payload: str) -> None:
        raise RuntimeError(f"cannot handle {payload}")

    ctx.on(PING, failing)
    ctx.on(PING, log.append)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        ctx.emit(PING, "x")

    assert log == ["x"]
    assert "cannot handle x" in caplog.text
    assert "test/ping" in caplog.text


async def test_a_listener_added_during_dispatch_waits_for_the_next_one(ctx: Context) -> None:
    log: list[str] = []

    def first(payload: str) -> None:
        log.append(f"first:{payload}")
        if payload == "one":
            ctx.on(PING, lambda late: log.append(f"late:{late}"))

    ctx.on(PING, first)

    ctx.emit(PING, "one")
    ctx.emit(PING, "two")

    assert log == ["first:one", "first:two", "late:two"]


async def test_bail_returns_the_first_answer(ctx: Context) -> None:
    asked: list[str] = []

    def undecided(payload: str) -> int | None:
        asked.append("undecided")
        return None

    def decided(payload: str) -> int | None:
        asked.append("decided")
        return len(payload)

    def never_asked(payload: str) -> int | None:
        asked.append("never")
        return -1

    ctx.on(ASK, undecided)
    ctx.on(ASK, decided)
    ctx.on(ASK, never_asked)

    assert ctx.bail(ASK, "four") == 4
    assert asked == ["undecided", "decided"]


async def test_bail_treats_zero_and_false_as_answers(ctx: Context) -> None:
    ctx.on(ASK, lambda payload: 0)
    ctx.on(ASK, lambda payload: 7)

    assert ctx.bail(ASK, "x") == 0


async def test_bail_returns_none_when_nobody_answers(ctx: Context) -> None:
    ctx.on(ASK, lambda payload: None)

    assert ctx.bail(ASK, "x") is None


async def test_bail_propagates_a_listener_error(ctx: Context) -> None:
    def failing(payload: str) -> int | None:
        raise RuntimeError("no answer")

    ctx.on(ASK, failing)

    with pytest.raises(RuntimeError, match="no answer"):
        ctx.bail(ASK, "x")


async def test_dispatching_with_the_wrong_mode_is_an_error(ctx: Context) -> None:
    with pytest.raises(EventModeError, match=r"bail\(\)"):
        ctx.emit(ASK, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"emit\(\)"):
        ctx.bail(PING, "x")  # pyright: ignore[reportArgumentType]


async def test_one_name_cannot_be_declared_as_two_kinds(ctx: Context) -> None:
    ctx.on(EmitEvent[str]("test/twice"), lambda payload: None)

    with pytest.raises(EventModeError, match="already declared as EmitEvent"):
        ctx.bail(BailEvent[str, int]("test/twice"), "x")


async def test_emit_events_reject_coroutine_functions_as_listeners(ctx: Context) -> None:
    async def listener(payload: str) -> None:
        return None

    with pytest.raises(EventModeError, match="plain functions"):
        ctx.on(PING, listener)  # pyright: ignore[reportArgumentType, reportCallIssue]


async def test_emit_closes_a_coroutine_returned_by_a_listener(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    class AsyncCallable:
        async def __call__(self, payload: str) -> None:
            return None

    ctx.on(PING, AsyncCallable())  # pyright: ignore[reportArgumentType, reportCallIssue]

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        ctx.emit(PING, "x")

    assert "returned a coroutine" in caplog.text
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_events_sync.py -q --timeout=10`

Expected: `15 failed in 0.80s`，第一个错误是 `AttributeError: 'Context' object has no attribute 'on'`

**Step 3: 写实现**

修改 `pyproject.toml`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/pyproject.toml
+++ b/pyproject.toml
@@ -83,6 +83,7 @@ convention = "google"
 include = ["src", "tests"]
 pythonVersion = "3.11"
 typeCheckingMode = "strict"
+reportUnnecessaryTypeIgnoreComment = true
 venvPath = "."
 venv = ".venv"
 
```

修改 `src/tack/kernel/events.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/events.py
+++ b/src/tack/kernel/events.py
@@ -4,8 +4,15 @@ An event is declared once, as a module-level constant of one of five kinds. The
 kind fixes how the event is dispatched. Every event carries a single payload.
 """
 
-from collections.abc import Awaitable, Callable
-from typing import ClassVar, Generic, TypeAlias, TypeVar
+import inspect
+import logging
+from collections.abc import Awaitable, Callable, Sequence
+from dataclasses import dataclass
+from typing import Any, ClassVar, Generic, TypeAlias, TypeVar
+
+from tack.kernel.errors import EventModeError
+
+logger = logging.getLogger("tack.kernel")
 
 PayloadT = TypeVar("PayloadT")
 ResultT = TypeVar("ResultT")
@@ -81,3 +88,102 @@ SerialListener: TypeAlias = Callable[[PayloadT], Awaitable[ResultT | None] | Res
 BailListener: TypeAlias = Callable[[PayloadT], ResultT | None]
 Next: TypeAlias = Callable[[PayloadT], Awaitable[ResultT]]
 WaterfallListener: TypeAlias = Callable[[PayloadT, Next[PayloadT, ResultT]], Awaitable[ResultT]]
+
+
+_PLAIN_LISTENER_MODES = frozenset({"emit", "bail"})
+
+
+@dataclass(slots=True, eq=False)
+class Hook:
+    """One registered listener."""
+
+    callback: Callable[..., Any]
+
+
+class HookTable:
+    """The listeners of every event, and the kind each event name belongs to."""
+
+    __slots__ = ("_hooks", "_kinds")
+
+    def __init__(self) -> None:
+        self._hooks: dict[str, list[Hook]] = {}
+        self._kinds: dict[str, type[Event]] = {}
+
+    def declare(self, event: Event) -> None:
+        """Record the kind of ``event``.
+
+        Raises:
+            EventModeError: The name is already in use by an event of another kind.
+        """
+        declared = self._kinds.setdefault(event.name, type(event))
+        if declared is not type(event):
+            raise EventModeError(
+                f"event {event.name!r} is already declared as {declared.__name__}, "
+                f"not {type(event).__name__}"
+            )
+
+    def add(self, event: Event, hook: Hook, *, prepend: bool) -> None:
+        """Register ``hook`` after the existing listeners, or before them.
+
+        Raises:
+            EventModeError: The event name belongs to another kind, or the
+                listener is a coroutine function and the event does not await
+                its listeners.
+        """
+        self.declare(event)
+        if event.mode in _PLAIN_LISTENER_MODES and inspect.iscoroutinefunction(hook.callback):
+            raise EventModeError(
+                f"{event!r} takes plain functions as listeners, not coroutine functions"
+            )
+        hooks = self._hooks.setdefault(event.name, [])
+        if prepend:
+            hooks.insert(0, hook)
+        else:
+            hooks.append(hook)
+
+    def remove(self, event: Event, hook: Hook) -> None:
+        """Unregister ``hook``."""
+        hooks = self._hooks[event.name]
+        hooks.remove(hook)
+        if not hooks:
+            del self._hooks[event.name]
+
+    def select(self, event: Event, kind: type[Event]) -> tuple[Hook, ...]:
+        """Return the listeners one dispatch of ``event`` should reach.
+
+        The result is a snapshot: listeners added or removed while the dispatch
+        runs do not change who it reaches.
+
+        Raises:
+            EventModeError: ``event`` is not of ``kind``.
+        """
+        if not isinstance(event, kind):
+            raise EventModeError(f"{event!r} is dispatched with {event.mode}(), not {kind.mode}()")
+        self.declare(event)
+        return tuple(self._hooks.get(event.name, ()))
+
+
+def run_emit(hooks: Sequence[Hook], event: Event, payload: object) -> None:
+    """Call every listener, logging the ones that fail."""
+    for hook in hooks:
+        try:
+            result = hook.callback(payload)
+        except Exception:
+            logger.exception("a listener of %r failed", event.name)
+            continue
+        if inspect.iscoroutine(result):
+            result.close()
+            logger.error(
+                "a listener of %r returned a coroutine; %s listeners must be plain functions",
+                event.name,
+                event.mode,
+            )
+
+
+def run_bail(hooks: Sequence[Hook], payload: object) -> Any:
+    """Return the first answer that is not ``None``."""
+    for hook in hooks:
+        result = hook.callback(payload)
+        if result is not None:
+            return result
+    return None
```

修改 `src/tack/kernel/fiber.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/fiber.py
+++ b/src/tack/kernel/fiber.py
@@ -19,6 +19,7 @@ from collections.abc import Awaitable, Callable
 from typing import TYPE_CHECKING, TypeAlias
 
 from tack.kernel.errors import InactiveEffectError
+from tack.kernel.events import HookTable
 
 if TYPE_CHECKING:
     from tack.kernel.context import Context
@@ -46,11 +47,12 @@ class FiberState(enum.Enum):
 class KernelState:
     """State shared by every fiber and context of one application."""
 
-    __slots__ = ("_uids", "drivers")
+    __slots__ = ("_uids", "drivers", "hooks")
 
     def __init__(self) -> None:
         self._uids = itertools.count()
         self.drivers: set[asyncio.Task[None]] = set()
+        self.hooks = HookTable()
 
     def next_uid(self) -> int:
         """Return a fiber identifier that has not been used before."""
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -5,7 +5,20 @@
 from __future__ import annotations
 
 from collections.abc import Callable
+from typing import Any, overload
 
+from tack.kernel.events import (
+    BailEvent,
+    BailListener,
+    EmitEvent,
+    EmitListener,
+    Event,
+    Hook,
+    PayloadT,
+    ResultT,
+    run_bail,
+    run_emit,
+)
 from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState
 
 
@@ -36,3 +49,53 @@ class Context:
                 not run.
         """
         return self.fiber.effect(setup)
+
+    @overload
+    def on(
+        self, event: EmitEvent[PayloadT], listener: EmitListener[PayloadT], *, prepend: bool = False
+    ) -> EffectHandle: ...
+
+    @overload
+    def on(
+        self,
+        event: BailEvent[PayloadT, ResultT],
+        listener: BailListener[PayloadT, ResultT],
+        *,
+        prepend: bool = False,
+    ) -> EffectHandle: ...
+
+    def on(
+        self, event: Event, listener: Callable[..., Any], *, prepend: bool = False
+    ) -> EffectHandle:
+        """Listen to ``event`` until the owning fiber unloads.
+
+        Listeners run in registration order. ``prepend`` puts this listener
+        ahead of those already registered.
+
+        Raises:
+            EventModeError: The listener does not fit the kind of the event, or
+                the event name is already declared as another kind.
+            InactiveEffectError: The owning fiber is not loaded.
+        """
+        hooks = self._kernel.hooks
+        hook = Hook(listener)
+
+        def setup() -> Disposer:
+            hooks.add(event, hook, prepend=prepend)
+            return lambda: hooks.remove(event, hook)
+
+        return self.effect(setup)
+
+    def emit(self, event: EmitEvent[PayloadT], payload: PayloadT) -> None:
+        """Notify every listener of ``event``.
+
+        A listener that raises is logged and does not stop the others.
+        """
+        run_emit(self._kernel.hooks.select(event, EmitEvent), event, payload)
+
+    def bail(self, event: BailEvent[PayloadT, ResultT], payload: PayloadT) -> ResultT | None:
+        """Ask each listener in turn and return the first answer that is not ``None``.
+
+        A listener that raises stops the dispatch and the error reaches the caller.
+        """
+        return run_bail(self._kernel.hooks.select(event, BailEvent), payload)
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_events_sync.py -q`

Expected: `15 passed in 0.49s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`49 passed in 0.97s`
- 覆盖率合计：`96%`
- ruff check：`All checks passed!`
- ruff format：`13 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): add synchronous events (emit and bail)"
```

---

### Task 5: 异步事件：parallel、serial 与 waterfall

实现三种需要等待的分发方式。

**要点**

- `parallel` 并发运行全部监听器并等它们都结束，异常被隔离。
- `serial` 按顺序等待，遇到第一个非 `None` 的结果就停。监听器可以是普通函数。
- `waterfall` 里第一个注册的监听器在最外层。`next(payload)` 每调用一次，后面的链就重新走一遍，所以可以用来重试。

**Files:**

- Modify: `src/tack/kernel/events.py`
- Modify: `src/tack/kernel/context.py`
- Test（新建）: `tests/kernel/test_events_async.py`

**Step 1: 写测试**

新建 `tests/kernel/test_events_async.py`：

```python
import asyncio
import logging

import pytest

from tack.kernel import (
    Context,
    EventModeError,
    Next,
    ParallelEvent,
    SerialEvent,
    WaterfallEvent,
)

FLUSH = ParallelEvent[str]("test/flush")
DECIDE = SerialEvent[str, int]("test/decide")
RENDER = WaterfallEvent[str, str]("test/render")


async def final(payload: str) -> str:
    return f"<{payload}>"


async def test_parallel_runs_listeners_concurrently_and_waits_for_all(ctx: Context) -> None:
    both_started = asyncio.Event()
    started: list[str] = []
    finished: list[str] = []

    async def listener(payload: str) -> None:
        started.append(payload)
        if len(started) == 2:
            both_started.set()
        await both_started.wait()
        finished.append(payload)

    ctx.on(FLUSH, listener)
    ctx.on(FLUSH, listener)

    await ctx.parallel(FLUSH, "x")

    assert finished == ["x", "x"]


async def test_parallel_accepts_plain_functions(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(FLUSH, log.append)

    await ctx.parallel(FLUSH, "x")

    assert log == ["x"]


async def test_parallel_logs_a_failing_listener_and_waits_for_the_rest(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    async def failing(payload: str) -> None:
        raise RuntimeError("flush failed")

    async def slow(payload: str) -> None:
        await asyncio.sleep(0)
        log.append(payload)

    ctx.on(FLUSH, failing)
    ctx.on(FLUSH, slow)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await ctx.parallel(FLUSH, "x")

    assert log == ["x"]
    assert "flush failed" in caplog.text


async def test_serial_stops_at_the_first_answer(ctx: Context) -> None:
    asked: list[str] = []

    async def undecided(payload: str) -> int | None:
        asked.append("undecided")
        await asyncio.sleep(0)
        return None

    def decided(payload: str) -> int | None:
        asked.append("decided")
        return len(payload)

    async def never_asked(payload: str) -> int | None:
        asked.append("never")
        return -1

    ctx.on(DECIDE, undecided)
    ctx.on(DECIDE, decided)
    ctx.on(DECIDE, never_asked)

    assert await ctx.serial(DECIDE, "four") == 4
    assert asked == ["undecided", "decided"]


async def test_serial_returns_none_when_nobody_answers(ctx: Context) -> None:
    assert await ctx.serial(DECIDE, "x") is None


async def test_serial_propagates_a_listener_error(ctx: Context) -> None:
    async def failing(payload: str) -> int | None:
        raise RuntimeError("cannot decide")

    ctx.on(DECIDE, failing)

    with pytest.raises(RuntimeError, match="cannot decide"):
        await ctx.serial(DECIDE, "x")


async def test_waterfall_without_listeners_runs_the_final_handler(ctx: Context) -> None:
    assert await ctx.waterfall(RENDER, "x", final) == "<x>"


async def test_waterfall_wraps_the_final_handler_outermost_first(ctx: Context) -> None:
    async def outer(payload: str, next: Next[str, str]) -> str:
        return f"outer({await next(payload)})"

    async def inner(payload: str, next: Next[str, str]) -> str:
        return f"inner({await next(payload)})"

    ctx.on(RENDER, outer)
    ctx.on(RENDER, inner)

    assert await ctx.waterfall(RENDER, "x", final) == "outer(inner(<x>))"


async def test_a_waterfall_listener_can_replace_the_payload(ctx: Context) -> None:
    async def shout(payload: str, next: Next[str, str]) -> str:
        return await next(payload.upper())

    ctx.on(RENDER, shout)

    assert await ctx.waterfall(RENDER, "x", final) == "<X>"


async def test_a_waterfall_listener_can_short_circuit(ctx: Context) -> None:
    reached: list[str] = []

    async def veto(payload: str, next: Next[str, str]) -> str:
        return "vetoed"

    async def unreachable(payload: str, next: Next[str, str]) -> str:
        reached.append(payload)
        return await next(payload)

    ctx.on(RENDER, veto)
    ctx.on(RENDER, unreachable)

    assert await ctx.waterfall(RENDER, "x", final) == "vetoed"
    assert reached == []


async def test_a_waterfall_listener_can_run_the_rest_of_the_chain_again(ctx: Context) -> None:
    attempts: list[str] = []

    async def retry(payload: str, next: Next[str, str]) -> str:
        try:
            return await next(payload)
        except RuntimeError:
            return await next(payload)

    async def flaky(payload: str, next: Next[str, str]) -> str:
        attempts.append(payload)
        if len(attempts) == 1:
            raise RuntimeError("try again")
        return await next(payload)

    ctx.on(RENDER, retry)
    ctx.on(RENDER, flaky)

    assert await ctx.waterfall(RENDER, "x", final) == "<x>"
    assert attempts == ["x", "x"]


async def test_waterfall_propagates_a_listener_error(ctx: Context) -> None:
    async def failing(payload: str, next: Next[str, str]) -> str:
        raise RuntimeError("cannot render")

    ctx.on(RENDER, failing)

    with pytest.raises(RuntimeError, match="cannot render"):
        await ctx.waterfall(RENDER, "x", final)


async def test_async_dispatch_checks_the_mode(ctx: Context) -> None:
    with pytest.raises(EventModeError, match=r"serial\(\)"):
        await ctx.parallel(DECIDE, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"waterfall\(\)"):
        await ctx.serial(RENDER, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"parallel\(\)"):
        await ctx.waterfall(FLUSH, "x", final)  # pyright: ignore[reportArgumentType]
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_events_async.py -q --timeout=10`

Expected: `13 failed in 0.82s`，第一个错误是 `AttributeError: 'Context' object has no attribute 'parallel'`

**Step 3: 写实现**

修改 `src/tack/kernel/events.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/events.py
+++ b/src/tack/kernel/events.py
@@ -4,6 +4,7 @@ An event is declared once, as a module-level constant of one of five kinds. The
 kind fixes how the event is dispatched. Every event carries a single payload.
 """
 
+import asyncio
 import inspect
 import logging
 from collections.abc import Awaitable, Callable, Sequence
@@ -187,3 +188,45 @@ def run_bail(hooks: Sequence[Hook], payload: object) -> Any:
         if result is not None:
             return result
     return None
+
+
+async def run_parallel(hooks: Sequence[Hook], event: Event, payload: object) -> None:
+    """Run every listener concurrently and wait for all, logging the ones that fail."""
+
+    async def call(hook: Hook) -> None:
+        try:
+            result = hook.callback(payload)
+            if inspect.isawaitable(result):
+                await result
+        except Exception:
+            logger.exception("a listener of %r failed", event.name)
+
+    await asyncio.gather(*(call(hook) for hook in hooks))
+
+
+async def run_serial(hooks: Sequence[Hook], payload: object) -> Any:
+    """Await each listener in turn and return the first answer that is not ``None``."""
+    for hook in hooks:
+        result = hook.callback(payload)
+        if inspect.isawaitable(result):
+            result = await result
+        if result is not None:
+            return result
+    return None
+
+
+async def run_waterfall(
+    hooks: Sequence[Hook], payload: Any, final: Callable[[Any], Awaitable[Any]]
+) -> Any:
+    """Run the listeners as middleware around ``final``, outermost first."""
+
+    async def step(index: int, value: Any) -> Any:
+        if index == len(hooks):
+            return await final(value)
+
+        def following(later: Any) -> Awaitable[Any]:
+            return step(index + 1, later)
+
+        return await hooks[index].callback(value, following)
+
+    return await step(0, payload)
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -14,10 +14,20 @@ from tack.kernel.events import (
     EmitListener,
     Event,
     Hook,
+    Next,
+    ParallelEvent,
+    ParallelListener,
     PayloadT,
     ResultT,
+    SerialEvent,
+    SerialListener,
+    WaterfallEvent,
+    WaterfallListener,
     run_bail,
     run_emit,
+    run_parallel,
+    run_serial,
+    run_waterfall,
 )
 from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState
 
@@ -64,6 +74,33 @@ class Context:
         prepend: bool = False,
     ) -> EffectHandle: ...
 
+    @overload
+    def on(
+        self,
+        event: ParallelEvent[PayloadT],
+        listener: ParallelListener[PayloadT],
+        *,
+        prepend: bool = False,
+    ) -> EffectHandle: ...
+
+    @overload
+    def on(
+        self,
+        event: SerialEvent[PayloadT, ResultT],
+        listener: SerialListener[PayloadT, ResultT],
+        *,
+        prepend: bool = False,
+    ) -> EffectHandle: ...
+
+    @overload
+    def on(
+        self,
+        event: WaterfallEvent[PayloadT, ResultT],
+        listener: WaterfallListener[PayloadT, ResultT],
+        *,
+        prepend: bool = False,
+    ) -> EffectHandle: ...
+
     def on(
         self, event: Event, listener: Callable[..., Any], *, prepend: bool = False
     ) -> EffectHandle:
@@ -99,3 +136,35 @@ class Context:
         A listener that raises stops the dispatch and the error reaches the caller.
         """
         return run_bail(self._kernel.hooks.select(event, BailEvent), payload)
+
+    async def parallel(self, event: ParallelEvent[PayloadT], payload: PayloadT) -> None:
+        """Run every listener of ``event`` concurrently and wait for all of them.
+
+        A listener that raises is logged and does not stop the others.
+        """
+        await run_parallel(self._kernel.hooks.select(event, ParallelEvent), event, payload)
+
+    async def serial(
+        self, event: SerialEvent[PayloadT, ResultT], payload: PayloadT
+    ) -> ResultT | None:
+        """Await each listener in turn and return the first answer that is not ``None``.
+
+        A listener that raises stops the dispatch and the error reaches the caller.
+        """
+        return await run_serial(self._kernel.hooks.select(event, SerialEvent), payload)
+
+    async def waterfall(
+        self,
+        event: WaterfallEvent[PayloadT, ResultT],
+        payload: PayloadT,
+        final: Next[PayloadT, ResultT],
+    ) -> ResultT:
+        """Run the listeners of ``event`` as middleware around ``final``.
+
+        The first listener is outermost. Each listener receives the payload and
+        a ``next`` function that runs the rest of the chain; it may pass a
+        different payload on, call ``next`` again to retry, or return without
+        calling it to short-circuit. Errors reach the caller.
+        """
+        hooks = self._kernel.hooks.select(event, WaterfallEvent)
+        return await run_waterfall(hooks, payload, final)
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_events_async.py -q`

Expected: `13 passed in 0.55s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`62 passed in 1.08s`
- 覆盖率合计：`97%`
- ruff check：`All checks passed!`
- ruff format：`14 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): add asynchronous events (parallel, serial and waterfall)"
```

---

### Task 6: 插件的加载与卸载

让上下文能加载插件。这个任务还不处理依赖，插件创建后立即开始加载。

**要点**

- `ctx.plugin()` 立即返回，加载在驱动任务里进行。调用时必须已经有运行中的事件循环。
- 配置在创建 Fiber 时校验，不等到加载。这样配置写错的插件即使依赖没到齐也会报出来。
- 加载失败时，`apply` 里已经做的注册全部回滚。
- 驱动是一个循环：只要"想要的状态"和"已加载的状态"不一致就继续加载或卸载。加载途中被要求销毁的插件会在 `apply` 返回后直接卸载，不进入 `ACTIVE`。
- 子插件的销毁是父 Fiber 的一个 effect，所以父插件卸载时子插件按逆序一起卸载。

**Files:**

- Modify: `src/tack/kernel/fiber.py`
- Create: `src/tack/kernel/plugin.py`
- Modify: `src/tack/kernel/context.py`
- Modify: `src/tack/kernel/__init__.py`
- Test（新建）: `tests/kernel/test_plugins.py`

**Step 1: 写测试**

新建 `tests/kernel/test_plugins.py`：

```python
import asyncio
import logging
from types import SimpleNamespace

import pytest
from pydantic import BaseModel, ValidationError

from tack.kernel import (
    Context,
    Disposer,
    EmitEvent,
    FiberState,
    InactiveEffectError,
    InvalidPluginError,
    Plugin,
    resolve_plugin,
)

PING = EmitEvent[str]("test/ping")


class Greeting(BaseModel):
    name: str = "world"


def nothing(ctx: Context, config: object) -> None:
    return None


async def test_a_plugin_loads_in_the_background_and_can_be_awaited(ctx: Context) -> None:
    seen: list[Context] = []

    def apply(inner: Context, config: object) -> None:
        seen.append(inner)

    fiber = ctx.plugin(Plugin("demo", apply))
    assert fiber.state is FiberState.PENDING
    assert seen == []

    assert await fiber is fiber

    assert fiber.state is FiberState.ACTIVE
    assert seen == [fiber.ctx]
    assert fiber.name == "demo"
    assert fiber.parent is ctx.fiber
    assert fiber.ctx is not ctx
    assert repr(fiber) == f"<Fiber 'demo' #{fiber.uid} active>"


async def test_apply_may_be_a_coroutine_function(ctx: Context) -> None:
    states: list[FiberState] = []

    async def apply(inner: Context, config: object) -> None:
        await asyncio.sleep(0)
        states.append(inner.fiber.state)

    fiber = await ctx.plugin(Plugin("demo", apply))

    assert states == [FiberState.LOADING]
    assert fiber.state is FiberState.ACTIVE


async def test_the_disposer_returned_by_apply_runs_last_in_first_out(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("first effect"))
        inner.effect(lambda: lambda: log.append("second effect"))
        return lambda: log.append("returned disposer")

    fiber = await ctx.plugin(Plugin("demo", apply))
    await fiber.dispose()

    assert log == ["returned disposer", "second effect", "first effect"]
    assert fiber.state is FiberState.DISPOSED


async def test_config_is_validated_against_the_model(ctx: Context) -> None:
    seen: list[Greeting] = []

    def apply(inner: Context, config: Greeting) -> None:
        seen.append(config)

    fiber = await ctx.plugin(Plugin("demo", apply, config=Greeting), {"name": "tack"})

    assert seen == [Greeting(name="tack")]
    assert fiber.config == Greeting(name="tack")


async def test_a_missing_config_uses_the_model_defaults(ctx: Context) -> None:
    fiber = await ctx.plugin(Plugin("demo", nothing, config=Greeting))

    assert fiber.config == Greeting(name="world")


async def test_config_without_a_model_is_passed_through(ctx: Context) -> None:
    fiber = await ctx.plugin(Plugin("demo", nothing), {"anything": 1})

    assert fiber.config == {"anything": 1}


async def test_an_invalid_config_fails_without_running_apply(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    calls: list[str] = []

    def apply(inner: Context, config: Greeting) -> None:
        calls.append("applied")

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("demo", apply, config=Greeting), {"name": ["not", "text"]})

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, ValidationError)
    assert "demo" in caplog.text
    with pytest.raises(ValidationError):
        await fiber
    assert calls == []


async def test_a_plugin_that_raises_fails_and_rolls_back(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        inner.effect(lambda: lambda: log.append("rolled back"))
        raise RuntimeError("cannot start")

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("demo", apply))
        with pytest.raises(RuntimeError, match="cannot start"):
            await fiber

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, RuntimeError)
    assert log == ["rolled back"]
    assert "cannot start" in caplog.text

    ctx.emit(PING, "after failure")
    assert log == ["rolled back"]


async def test_a_failed_plugin_can_be_disposed(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        raise RuntimeError("cannot start")

    fiber = ctx.plugin(Plugin("demo", apply))
    with pytest.raises(RuntimeError):
        await fiber

    await fiber.dispose()

    assert fiber.state is FiberState.DISPOSED
    assert await fiber is fiber


async def test_apply_must_return_a_disposer_or_none(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> int:
        return 42

    fiber = ctx.plugin(Plugin("demo", apply))  # pyright: ignore[reportArgumentType]

    with pytest.raises(InvalidPluginError, match="disposer or None"):
        await fiber
    assert fiber.state is FiberState.FAILED


async def test_disposing_a_plugin_removes_its_registrations(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)

    fiber = await ctx.plugin(Plugin("demo", apply))
    ctx.emit(PING, "before")

    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["before"]
    assert fiber not in ctx.fibers()


async def test_nested_plugins_are_disposed_with_their_parent(ctx: Context) -> None:
    log: list[str] = []
    children: list[object] = []

    def child(inner: Context, config: object) -> Disposer:
        return lambda: log.append("child")

    def parent(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("parent effect before child"))
        children.append(inner.plugin(Plugin("child", child)))
        return lambda: log.append("parent")

    fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    await fiber.dispose()

    assert log == ["parent", "child", "parent effect before child"]
    assert [nested.state for nested in ctx.fibers()] == []


async def test_a_parent_can_wait_for_its_child_while_loading(ctx: Context) -> None:
    states: list[FiberState] = []

    async def parent(inner: Context, config: object) -> None:
        child = await inner.plugin(Plugin("child", nothing))
        states.append(child.state)

    await ctx.plugin(Plugin("parent", parent))

    assert states == [FiberState.ACTIVE]


async def test_disposing_the_root_disposes_every_plugin() -> None:
    ctx = Context()
    first = await ctx.plugin(Plugin("first", nothing))
    second = await ctx.plugin(Plugin("second", nothing))

    await ctx.fiber.dispose()

    assert first.state is FiberState.DISPOSED
    assert second.state is FiberState.DISPOSED
    assert ctx.fibers() == ()


async def test_plugins_cannot_be_added_to_a_disposed_fiber() -> None:
    ctx = Context()
    await ctx.fiber.dispose()

    with pytest.raises(InactiveEffectError):
        ctx.plugin(Plugin("late", nothing))

    assert ctx.fibers() == ()


async def test_settled_waits_for_every_plugin(ctx: Context) -> None:
    async def slow(inner: Context, config: object) -> None:
        await asyncio.sleep(0)
        await asyncio.sleep(0)

    first = ctx.plugin(Plugin("first", slow))
    second = ctx.plugin(Plugin("second", slow))

    await ctx.settled()

    assert first.state is FiberState.ACTIVE
    assert second.state is FiberState.ACTIVE
    assert ctx.fibers() == (first, second)


async def test_a_plugin_disposed_while_loading_never_becomes_active(ctx: Context) -> None:
    entered = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def apply(inner: Context, config: object) -> Disposer:
        entered.set()
        await release.wait()
        return lambda: log.append("undone")

    fiber = ctx.plugin(Plugin("demo", apply))
    await entered.wait()
    disposal = asyncio.ensure_future(fiber.dispose())
    await asyncio.sleep(0)
    assert fiber.state is FiberState.LOADING

    release.set()
    await disposal

    assert log == ["undone"]
    assert fiber.state is FiberState.DISPOSED


async def test_a_plugin_can_dispose_itself_while_loading(ctx: Context) -> None:
    async def apply(inner: Context, config: object) -> None:
        await inner.fiber.dispose()

    fiber = ctx.plugin(Plugin("demo", apply))
    await ctx.settled()

    assert fiber.state is FiberState.DISPOSED


async def test_a_module_like_object_is_a_plugin(ctx: Context) -> None:
    seen: list[Greeting] = []

    def apply(inner: Context, config: Greeting) -> None:
        seen.append(config)

    module = SimpleNamespace(name="greeter", apply=apply, Config=Greeting)
    fiber = await ctx.plugin(module, {"name": "tack"})

    assert fiber.name == "greeter"
    assert seen == [Greeting(name="tack")]


async def test_a_bare_function_is_a_plugin(ctx: Context) -> None:
    def greeter(inner: Context, config: object) -> None:
        return None

    fiber = await ctx.plugin(greeter)

    assert fiber.name == "greeter"


def test_resolve_plugin_returns_a_plugin_unchanged() -> None:
    plugin = Plugin("demo", nothing)

    assert resolve_plugin(plugin) is plugin


def test_resolve_plugin_reads_the_module_name_when_no_name_is_given() -> None:
    module = SimpleNamespace(__name__="pkg.greeter", apply=nothing)

    assert resolve_plugin(module).name == "pkg.greeter"


@pytest.mark.parametrize(
    "candidate",
    [
        42,
        SimpleNamespace(apply="not callable"),
        SimpleNamespace(apply=nothing, Config=dict),
        SimpleNamespace(apply=nothing, name=7),
    ],
)
def test_resolve_plugin_rejects_other_objects(candidate: object) -> None:
    with pytest.raises(InvalidPluginError):
        resolve_plugin(candidate)
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_plugins.py -q --timeout=10`

Expected: `1 error in 0.72s`，第一个错误是 `ImportError: cannot import name 'Plugin' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py)`

**Step 3: 写实现**

修改 `src/tack/kernel/fiber.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/fiber.py
+++ b/src/tack/kernel/fiber.py
@@ -15,14 +15,17 @@ import enum
 import inspect
 import itertools
 import logging
-from collections.abc import Awaitable, Callable
-from typing import TYPE_CHECKING, TypeAlias
+from collections.abc import Awaitable, Callable, Generator
+from typing import TYPE_CHECKING, Any, TypeAlias
 
-from tack.kernel.errors import InactiveEffectError
+from pydantic import ValidationError
+
+from tack.kernel.errors import InactiveEffectError, InvalidPluginError
 from tack.kernel.events import HookTable
 
 if TYPE_CHECKING:
     from tack.kernel.context import Context
+    from tack.kernel.plugin import Plugin
 
 logger = logging.getLogger("tack.kernel")
 
@@ -47,11 +50,12 @@ class FiberState(enum.Enum):
 class KernelState:
     """State shared by every fiber and context of one application."""
 
-    __slots__ = ("_uids", "drivers", "hooks")
+    __slots__ = ("_uids", "drivers", "fibers", "hooks")
 
     def __init__(self) -> None:
         self._uids = itertools.count()
         self.drivers: set[asyncio.Task[None]] = set()
+        self.fibers: dict[int, Fiber] = {}
         self.hooks = HookTable()
 
     def next_uid(self) -> int:
@@ -96,42 +100,92 @@ class EffectHandle:
 
 
 class Fiber:
-    """The owner of a set of effects.
-
-    Disposing a fiber undoes its effects in reverse order of registration, one
-    at a time, and waits for each to finish.
+    """One instance of a plugin, and the owner of everything it registered.
+
+    A fiber loads by calling its plugin's ``apply`` and unloads by undoing the
+    effects registered since, in reverse order and one at a time. Loading and
+    unloading run in the background; ``await fiber`` waits for them and raises
+    the error of a failed load.
+
+    The root fiber and scope fibers have no plugin. They are active from the
+    moment they are created until they are disposed.
+
+    Attributes:
+        uid: A number unique within the application.
+        parent: The fiber whose context created this one, or ``None`` for the root.
+        plugin: The plugin this fiber is an instance of, if any.
+        config: The validated config handed to ``apply``.
+        error: Why the most recent load failed, if it did.
+        state: Where the fiber is in its lifecycle.
+        ctx: The context whose registrations this fiber owns.
     """
 
     __slots__ = (
+        "_broken",
         "_disposed",
         "_driver",
         "_effects",
+        "_failed",
+        "_handle",
         "_kernel",
         "_loaded",
         "_wanted",
+        "config",
         "ctx",
+        "error",
         "parent",
+        "plugin",
         "state",
         "uid",
     )
 
     ctx: Context
 
-    def __init__(self, kernel: KernelState, parent: Fiber | None) -> None:
+    def __init__(
+        self,
+        kernel: KernelState,
+        parent: Fiber | None,
+        plugin: Plugin | None = None,
+        config: object = None,
+    ) -> None:
         self.uid = kernel.next_uid()
         self.parent = parent
-        self.state = FiberState.ACTIVE
+        self.plugin = plugin
+        self.config: Any = config
+        self.error: Exception | None = None
+        self.state = FiberState.PENDING
         self._kernel = kernel
         self._effects: list[EffectHandle] = []
-        self._wanted: Epoch | None = ()
-        self._loaded: Epoch | None = ()
+        self._wanted: Epoch | None = None
+        self._loaded: Epoch | None = None
+        self._failed: Epoch | None = None
         self._driver: asyncio.Task[None] | None = None
+        self._handle: EffectHandle | None = None
         self._disposed = False
+        self._broken = False
+        if plugin is None:
+            self._wanted = self._loaded = ()
+            self.state = FiberState.ACTIVE
+        elif plugin.config is not None:
+            try:
+                self.config = plugin.config.model_validate({} if config is None else config)
+            except ValidationError as error:
+                logger.error("plugin %r has an invalid config: %s", plugin.name, error)
+                self.error = error
+                self._broken = True
 
     @property
     def name(self) -> str:
         """A short label for logs and error messages."""
-        return "root"
+        if self.plugin is not None:
+            return self.plugin.name
+        return "root" if self.parent is None else "scope"
+
+    def __repr__(self) -> str:
+        return f"<Fiber {self.name!r} #{self.uid} {self.state.value}>"
+
+    def __await__(self) -> Generator[Any, None, Fiber]:
+        return self._ready().__await__()
 
     def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
         """Run ``setup`` now and keep the disposer it returns.
@@ -139,17 +193,17 @@ class Fiber:
         Raises:
             InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
         """
-        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
-            raise InactiveEffectError(
-                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
-            )
+        self._require_loaded()
         handle = EffectHandle(self, setup())
         self._effects.append(handle)
         return handle
 
     async def dispose(self) -> None:
         """Unload the fiber for good and wait until its effects are undone."""
-        self._retire()
+        if self._handle is None:
+            self._retire()
+        else:
+            await self._handle.dispose()
         await self.settled()
 
     async def settled(self) -> None:
@@ -157,18 +211,46 @@ class Fiber:
         while (driver := self._driver) is not None and driver is not asyncio.current_task():
             await asyncio.shield(driver)
 
+    async def _ready(self) -> Fiber:
+        await self.settled()
+        if self.state is FiberState.FAILED and self.error is not None:
+            raise self.error
+        return self
+
+    def _require_loaded(self) -> None:
+        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
+            raise InactiveEffectError(
+                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
+            )
+
+    async def _dispose_owned(self) -> None:
+        self._retire()
+        await self.settled()
+
     def _retire(self) -> None:
         if self._disposed:
             return
         self._disposed = True
+        self._kernel.fibers.pop(self.uid, None)
         self._refresh()
 
+    def _compute_epoch(self) -> Epoch | None:
+        if self._disposed or self._broken:
+            return None
+        return ()
+
     def _refresh(self) -> None:
-        self._wanted = None if self._disposed else ()
+        wanted = self._compute_epoch()
+        if wanted != self._wanted:
+            self._wanted = wanted
+            self._failed = None
+            self.error = None
         self._kick()
 
     def _needs_work(self) -> bool:
-        return self._loaded is not None and self._loaded != self._wanted
+        if self._loaded is not None:
+            return self._loaded != self._wanted
+        return self._wanted is not None and self._wanted != self._failed
 
     def _kick(self) -> None:
         if self._driver is not None:
@@ -184,13 +266,44 @@ class Fiber:
     async def _drive(self) -> None:
         try:
             while self._needs_work():
-                await self._unload()
+                if self._loaded is not None:
+                    await self._unload()
+                else:
+                    await self._load()
+            self._rest()
         finally:
             driver = self._driver
             self._driver = None
             if driver is not None:
                 self._kernel.drivers.discard(driver)
-            self._rest()
+
+    async def _load(self) -> None:
+        plugin = self.plugin
+        epoch = self._wanted
+        assert plugin is not None
+        assert epoch is not None
+        self._set_state(FiberState.LOADING)
+        self._loaded = epoch
+        try:
+            result = plugin.apply(self.ctx, self.config)
+            if inspect.isawaitable(result):
+                result = await result
+            if result is not None:
+                returned: object = result
+                if not callable(returned):
+                    raise InvalidPluginError(
+                        f"plugin {self.name!r} returned {returned!r}; "
+                        "apply must return a disposer or None"
+                    )
+                self._effects.append(EffectHandle(self, result))
+        except Exception as error:
+            logger.error("plugin %r failed to load", self.name, exc_info=error)
+            self.error = error
+            self._failed = epoch
+            await self._unload()
+            return
+        if self._wanted == epoch:
+            self._set_state(FiberState.ACTIVE)
 
     async def _unload(self) -> None:
         self._set_state(FiberState.UNLOADING)
@@ -216,6 +329,8 @@ class Fiber:
             self._set_state(FiberState.ACTIVE)
         elif self._disposed:
             self._set_state(FiberState.DISPOSED)
+        elif self.error is not None:
+            self._set_state(FiberState.FAILED)
         else:
             self._set_state(FiberState.PENDING)
 
```

新建 `src/tack/kernel/plugin.py`：

```python
"""Plugins: the units the kernel loads and unloads."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeAlias, cast

from pydantic import BaseModel

from tack.kernel.errors import InvalidPluginError

if TYPE_CHECKING:
    from tack.kernel.context import Context
    from tack.kernel.fiber import Disposer

ApplyResult: TypeAlias = "Disposer | None"
Apply: TypeAlias = "Callable[[Context, Any], ApplyResult | Awaitable[ApplyResult]]"


@dataclass(frozen=True, slots=True)
class Plugin:
    """A unit of functionality that the kernel can load and unload.

    Attributes:
        name: A label for logs and error messages.
        apply: Called with the plugin's own context and its config each time
            the plugin loads. It may be a coroutine function. It may return a
            disposer, which runs first when the plugin unloads.
        config: A pydantic model that the raw config is validated against, or
            ``None`` to hand the raw config to ``apply`` unchanged.
    """

    name: str
    apply: Apply
    config: type[BaseModel] | None = None


def resolve_plugin(candidate: object) -> Plugin:
    """Turn any accepted plugin shape into a :class:`Plugin`.

    Three shapes are accepted: a :class:`Plugin`; a module or object with an
    ``apply`` callable and optional ``name`` and ``Config`` attributes; and a
    bare callable, which is its own ``apply``.

    Raises:
        InvalidPluginError: ``candidate`` has none of these shapes.
    """
    if isinstance(candidate, Plugin):
        return candidate
    apply = getattr(candidate, "apply", candidate)
    if not callable(apply):
        raise InvalidPluginError(
            f"{candidate!r} is not a plugin: expected a callable or an object with apply()"
        )
    name = getattr(candidate, "name", None) or getattr(candidate, "__name__", None)
    if name is None:
        name = type(candidate).__name__
    if not isinstance(name, str):
        raise InvalidPluginError(f"the name of plugin {candidate!r} must be a string")
    config = getattr(candidate, "Config", None)
    if config is not None and not (isinstance(config, type) and issubclass(config, BaseModel)):
        raise InvalidPluginError(f"the Config of plugin {name!r} must be a pydantic model")
    return Plugin(name, cast("Apply", apply), config)
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -4,6 +4,7 @@
 
 from __future__ import annotations
 
+import asyncio
 from collections.abc import Callable
 from typing import Any, overload
 
@@ -30,6 +31,7 @@ from tack.kernel.events import (
     run_waterfall,
 )
 from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState
+from tack.kernel.plugin import resolve_plugin
 
 
 class Context:
@@ -47,6 +49,53 @@ class Context:
         self.fiber = Fiber(self._kernel, None)
         self.fiber.ctx = self
 
+    @classmethod
+    def _owned_by(cls, kernel: KernelState, fiber: Fiber) -> Context:
+        ctx = cls.__new__(cls)
+        ctx._kernel = kernel
+        ctx.fiber = fiber
+        fiber.ctx = ctx
+        return ctx
+
+    def plugin(self, plugin: object, config: object = None) -> Fiber:
+        """Load ``plugin`` as a child of the owning fiber.
+
+        The plugin loads in the background, so this must be called while an
+        event loop is running. Await the returned fiber to wait for the load
+        and to receive its error if it fails. The child is disposed when the
+        owning fiber unloads.
+
+        Args:
+            plugin: A :class:`Plugin`, a module or object with ``apply``, or a
+                bare callable.
+            config: The raw config. It is validated against the plugin's config
+                model when the plugin has one.
+
+        Raises:
+            InvalidPluginError: ``plugin`` is not a plugin.
+            InactiveEffectError: The owning fiber is not loaded.
+        """
+        spec = resolve_plugin(plugin)
+        parent = self.fiber
+        parent._require_loaded()
+        fiber = Fiber(self._kernel, parent, spec, config)
+        Context._owned_by(self._kernel, fiber)
+        fiber._handle = parent.effect(lambda: fiber._dispose_owned)
+        self._kernel.fibers[fiber.uid] = fiber
+        fiber._refresh()
+        return fiber
+
+    def fibers(self) -> tuple[Fiber, ...]:
+        """Return every plugin fiber that has not been disposed, oldest first."""
+        return tuple(self._kernel.fibers.values())
+
+    async def settled(self) -> None:
+        """Wait until no plugin in the application is loading or unloading."""
+        drivers = self._kernel.drivers
+        current = asyncio.current_task()
+        while pending := [driver for driver in drivers if driver is not current]:
+            await asyncio.wait(pending)
+
     def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
         """Run ``setup`` now and undo it when the owning fiber unloads.
 
```

修改 `src/tack/kernel/__init__.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/__init__.py
+++ b/src/tack/kernel/__init__.py
@@ -26,9 +26,12 @@ from tack.kernel.events import (
     WaterfallListener,
 )
 from tack.kernel.fiber import Disposer, EffectHandle, Fiber, FiberState
+from tack.kernel.plugin import Apply, ApplyResult, Plugin, resolve_plugin
 from tack.kernel.services import ServiceKey
 
 __all__ = [
+    "Apply",
+    "ApplyResult",
     "BailEvent",
     "BailListener",
     "CompositionError",
@@ -47,6 +50,7 @@ __all__ = [
     "Next",
     "ParallelEvent",
     "ParallelListener",
+    "Plugin",
     "SerialEvent",
     "SerialListener",
     "ServiceConflictError",
@@ -55,4 +59,5 @@ __all__ = [
     "ServiceUnavailableError",
     "WaterfallEvent",
     "WaterfallListener",
+    "resolve_plugin",
 ]
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_plugins.py -q`

Expected: `26 passed in 0.53s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`88 passed in 1.61s`
- 覆盖率合计：`99%`
- ruff check：`All checks passed!`
- ruff format：`16 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): load and unload plugins as fibers"
```

---

### Task 7: 服务的提供与查找

实现 `provide`、`get`、`use` 和可见性规则。这个任务还没有依赖声明。

**要点**

- 提供服务是一个 effect。提供方卸载或 handle 被销毁时，服务被撤回。
- 提供方进入 `ACTIVE` 之前，别的 Fiber 看不到它的服务。这样依赖方不会拿到一个初始化到一半的服务。
- 提供方自己和它下面的 Fiber 在提供方加载、卸载期间也能看到这个服务。
- `test_plugins.py` 在这个任务里只改了一处等待方式，改动前后都通过。

**Files:**

- Modify: `src/tack/kernel/fiber.py`
- Modify: `src/tack/kernel/context.py`
- Test（新建）: `tests/kernel/test_services.py`
- Test（修改）: `tests/kernel/test_plugins.py`

**Step 1: 写测试**

新建 `tests/kernel/test_services.py`：

```python
import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
    ServiceUnavailableError,
)


class Clock:
    def __init__(self, now: int) -> None:
        self.now = now


CLOCK = ServiceKey[Clock]("clock")


async def test_a_provided_service_can_be_used(ctx: Context) -> None:
    clock = Clock(1)

    ctx.provide(CLOCK, clock)

    assert ctx.use(CLOCK) is clock
    assert ctx.get(CLOCK) is clock


async def test_get_returns_none_for_a_missing_service(ctx: Context) -> None:
    assert ctx.get(CLOCK) is None


async def test_use_raises_for_a_missing_service(ctx: Context) -> None:
    with pytest.raises(ServiceUnavailableError, match="'clock'"):
        ctx.use(CLOCK)


async def test_providing_a_service_twice_is_an_error(ctx: Context) -> None:
    first = Clock(1)
    ctx.provide(CLOCK, first)

    with pytest.raises(ServiceConflictError, match="already provided by 'root'"):
        ctx.provide(CLOCK, Clock(2))

    assert ctx.use(CLOCK) is first


async def test_disposing_the_handle_withdraws_the_service(ctx: Context) -> None:
    handle = ctx.provide(CLOCK, Clock(1))

    await handle.dispose()

    assert ctx.get(CLOCK) is None


async def test_a_name_can_be_provided_again_after_withdrawal(ctx: Context) -> None:
    handle = ctx.provide(CLOCK, Clock(1))
    await handle.dispose()
    second = Clock(2)

    ctx.provide(CLOCK, second)

    assert ctx.use(CLOCK) is second


async def test_a_service_is_withdrawn_when_its_provider_is_disposed(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))

    fiber = await ctx.plugin(Plugin("clock", apply))
    assert ctx.get(CLOCK) is not None

    await fiber.dispose()

    assert ctx.get(CLOCK) is None


async def test_a_service_becomes_visible_when_its_provider_is_active(ctx: Context) -> None:
    provided = asyncio.Event()
    release = asyncio.Event()
    clock = Clock(1)

    async def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        provided.set()
        await release.wait()

    fiber = ctx.plugin(Plugin("clock", apply))
    await asyncio.wait_for(provided.wait(), timeout=1)
    assert fiber.state is FiberState.LOADING
    assert ctx.get(CLOCK) is None

    release.set()
    await fiber

    assert ctx.get(CLOCK) is clock


async def test_a_provider_sees_its_own_service_while_loading(ctx: Context) -> None:
    clock = Clock(1)
    seen: list[Clock] = []

    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        seen.append(inner.use(CLOCK))

    await ctx.plugin(Plugin("clock", apply))

    assert seen == [clock]


async def test_a_child_sees_a_service_of_its_loading_parent(ctx: Context) -> None:
    clock = Clock(1)
    seen: list[Clock | None] = []

    def child(inner: Context, config: object) -> None:
        seen.append(inner.get(CLOCK))

    async def parent(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        await inner.plugin(Plugin("child", child))

    await ctx.plugin(Plugin("parent", parent))

    assert seen == [clock]


async def test_a_service_is_hidden_from_others_while_its_provider_unloads(ctx: Context) -> None:
    clock = Clock(1)
    outside: list[Clock | None] = []
    inside: list[Clock | None] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, clock)

        def undo() -> None:
            outside.append(ctx.get(CLOCK))
            inside.append(inner.get(CLOCK))

        return undo

    fiber = await ctx.plugin(Plugin("clock", apply))
    await fiber.dispose()

    assert outside == [None]
    assert inside == [clock]


async def test_a_failed_provider_withdraws_its_service(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        raise RuntimeError("cannot start")

    with pytest.raises(RuntimeError, match="cannot start"):
        await ctx.plugin(Plugin("clock", apply))

    assert ctx.get(CLOCK) is None
    ctx.provide(CLOCK, Clock(2))
```

修改 `tests/kernel/test_plugins.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/tests/kernel/test_plugins.py
+++ b/tests/kernel/test_plugins.py
@@ -264,7 +264,7 @@ async def test_a_plugin_disposed_while_loading_never_becomes_active(ctx: Context
         return lambda: log.append("undone")
 
     fiber = ctx.plugin(Plugin("demo", apply))
-    await entered.wait()
+    await asyncio.wait_for(entered.wait(), timeout=1)
     disposal = asyncio.ensure_future(fiber.dispose())
     await asyncio.sleep(0)
     assert fiber.state is FiberState.LOADING
```

**Step 2: 运行测试，确认失败**

逐个文件运行：`uv run pytest <文件> -q --timeout=10`

Expected:

| 文件 | 结果 | 第一个错误 |
|---|---|---|
| `test_plugins.py` | 26 passed in 0.65s | — |
| `test_services.py` | 12 failed in 1.73s | AttributeError: 'Context' object has no attribute 'provide' |

**Step 3: 写实现**

修改 `src/tack/kernel/fiber.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/fiber.py
+++ b/src/tack/kernel/fiber.py
@@ -15,13 +15,15 @@ import enum
 import inspect
 import itertools
 import logging
-from collections.abc import Awaitable, Callable, Generator
+from collections.abc import Awaitable, Callable, Generator, Iterator
+from dataclasses import dataclass
 from typing import TYPE_CHECKING, Any, TypeAlias
 
 from pydantic import ValidationError
 
 from tack.kernel.errors import InactiveEffectError, InvalidPluginError
 from tack.kernel.events import HookTable
+from tack.kernel.services import ServiceKey
 
 if TYPE_CHECKING:
     from tack.kernel.context import Context
@@ -47,21 +49,48 @@ class FiberState(enum.Enum):
     DISPOSED = "disposed"
 
 
+@dataclass(slots=True, eq=False)
+class Provision:
+    """A service value and the fiber that provides it."""
+
+    key: ServiceKey[Any]
+    value: Any
+    fiber: Fiber
+
+
 class KernelState:
     """State shared by every fiber and context of one application."""
 
-    __slots__ = ("_uids", "drivers", "fibers", "hooks")
+    __slots__ = ("_uids", "drivers", "fibers", "hooks", "services")
 
     def __init__(self) -> None:
         self._uids = itertools.count()
         self.drivers: set[asyncio.Task[None]] = set()
         self.fibers: dict[int, Fiber] = {}
         self.hooks = HookTable()
+        self.services: dict[ServiceKey[Any], Provision] = {}
 
     def next_uid(self) -> int:
         """Return a fiber identifier that has not been used before."""
         return next(self._uids)
 
+    def lookup(self, key: ServiceKey[Any], viewer: Fiber) -> Provision | None:
+        """Return the provision of ``key`` if ``viewer`` can see it.
+
+        A service is visible to everyone while its provider is active. The
+        provider itself and the fibers below it also see the service while the
+        provider is loading or unloading.
+        """
+        provision = self.services.get(key)
+        if provision is None:
+            return None
+        provider = provision.fiber
+        if provider.state is FiberState.ACTIVE:
+            return provision
+        if any(fiber is provider for fiber in viewer.lineage()):
+            return provision
+        return None
+
 
 class EffectHandle:
     """A registered effect. Disposing the handle undoes the effect once."""
@@ -184,6 +213,13 @@ class Fiber:
     def __repr__(self) -> str:
         return f"<Fiber {self.name!r} #{self.uid} {self.state.value}>"
 
+    def lineage(self) -> Iterator[Fiber]:
+        """Yield this fiber, then its parent, and so on up to the root."""
+        fiber: Fiber | None = self
+        while fiber is not None:
+            yield fiber
+            fiber = fiber.parent
+
     def __await__(self) -> Generator[Any, None, Fiber]:
         return self._ready().__await__()
 
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -8,6 +8,7 @@ import asyncio
 from collections.abc import Callable
 from typing import Any, overload
 
+from tack.kernel.errors import ServiceConflictError, ServiceUnavailableError
 from tack.kernel.events import (
     BailEvent,
     BailListener,
@@ -30,8 +31,9 @@ from tack.kernel.events import (
     run_serial,
     run_waterfall,
 )
-from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState
+from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState, Provision
 from tack.kernel.plugin import resolve_plugin
+from tack.kernel.services import ServiceKey, ServiceT
 
 
 class Context:
@@ -85,6 +87,49 @@ class Context:
         fiber._refresh()
         return fiber
 
+    def provide(self, key: ServiceKey[ServiceT], value: ServiceT) -> EffectHandle:
+        """Offer ``value`` as the service named by ``key``.
+
+        The service is withdrawn when the owning fiber unloads or the returned
+        handle is disposed. Other fibers see it once the owning fiber is active.
+
+        Raises:
+            ServiceConflictError: Another provider already holds the name.
+            InactiveEffectError: The owning fiber is not loaded.
+        """
+        services = self._kernel.services
+        fiber = self.fiber
+
+        def withdraw() -> None:
+            del services[key]
+
+        def setup() -> Disposer:
+            existing = services.get(key)
+            if existing is not None:
+                raise ServiceConflictError(
+                    f"service {key.name!r} is already provided by {existing.fiber.name!r}"
+                )
+            services[key] = Provision(key, value, fiber)
+            return withdraw
+
+        return self.effect(setup)
+
+    def get(self, key: ServiceKey[ServiceT]) -> ServiceT | None:
+        """Return the service named by ``key``, or ``None`` if it is not available."""
+        provision = self._kernel.lookup(key, self.fiber)
+        return None if provision is None else provision.value
+
+    def use(self, key: ServiceKey[ServiceT]) -> ServiceT:
+        """Return the service named by ``key``.
+
+        Raises:
+            ServiceUnavailableError: No active plugin provides the service.
+        """
+        provision = self._kernel.lookup(key, self.fiber)
+        if provision is None:
+            raise ServiceUnavailableError(f"service {key.name!r} is not available")
+        return provision.value
+
     def fibers(self) -> tuple[Fiber, ...]:
         """Return every plugin fiber that has not been disposed, oldest first."""
         return tuple(self._kernel.fibers.values())
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_plugins.py tests/kernel/test_services.py -q`

Expected: `38 passed in 0.68s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`100 passed in 1.76s`
- 覆盖率合计：`99%`
- ruff check：`All checks passed!`
- ruff format：`17 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): provide and look up services"
```

---

### Task 8: 依赖驱动的生命周期

实现 `inject`：服务到齐才加载，服务消失就卸载，服务回来再加载。这是内核最核心的行为。

**要点**

- 每次提供服务都分配一个新的序列号。插件"想要的状态"就是它所注入的各个服务当前的序列号，其中任何一个服务不可用时为空。
- 服务被提供、被撤回，以及提供方进出 `ACTIVE` 时，内核重新计算所有注入该服务的插件想要的状态。
- 提供方卸载时先把服务标为不可用，等依赖方全部卸载完，再撤销自己的 effect。
- `use` 检查声明。插件能用的服务只有两类：它自己提供的，以及它自己或外层插件注入的。根上下文和根下面的作用域不受限制。
- 加载失败的插件不会原地重试。它依赖的服务发生变化之后才再试一次。

**Files:**

- Modify: `src/tack/kernel/fiber.py`
- Modify: `src/tack/kernel/plugin.py`
- Modify: `src/tack/kernel/context.py`
- Test（新建）: `tests/kernel/test_dependencies.py`

**Step 1: 写测试**

新建 `tests/kernel/test_dependencies.py`：

```python
import asyncio
from types import SimpleNamespace

import pytest

from tack.kernel import (
    Context,
    Disposer,
    FiberState,
    InvalidPluginError,
    Plugin,
    ServiceKey,
    ServiceNotInjectedError,
    resolve_plugin,
)


class Clock:
    def __init__(self, now: int) -> None:
        self.now = now


CLOCK = ServiceKey[Clock]("clock")
STORE = ServiceKey[dict[str, int]]("store")
INDEX = ServiceKey[list[str]]("index")


def consumer(log: list[str]) -> Plugin:
    """A plugin that needs the clock and records each load and unload."""

    def apply(ctx: Context, config: object) -> Disposer:
        clock = ctx.use(CLOCK)
        log.append(f"load {clock.now}")
        return lambda: log.append(f"unload {clock.now}")

    return Plugin("consumer", apply, inject=(CLOCK,))


def clock_provider(now: int, log: list[str] | None = None) -> Plugin:
    def apply(ctx: Context, config: object) -> Disposer:
        ctx.provide(CLOCK, Clock(now))
        return lambda: None if log is None else log.append(f"provider {now} cleanup")

    return Plugin(f"clock-{now}", apply)


async def test_a_plugin_waits_for_the_services_it_injects(ctx: Context) -> None:
    log: list[str] = []

    fiber = await ctx.plugin(consumer(log))

    assert fiber.state is FiberState.PENDING
    assert fiber.missing == (CLOCK,)
    assert ctx.fiber.missing == ()
    assert log == []


async def test_a_pending_plugin_loads_when_its_service_is_provided(ctx: Context) -> None:
    log: list[str] = []
    fiber = await ctx.plugin(consumer(log))

    ctx.provide(CLOCK, Clock(1))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert fiber.missing == ()
    assert log == ["load 1"]


async def test_a_plugin_loads_at_once_when_its_services_exist(ctx: Context) -> None:
    log: list[str] = []
    ctx.provide(CLOCK, Clock(1))

    fiber = await ctx.plugin(consumer(log))

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1"]


async def test_a_plugin_unloads_when_its_service_is_withdrawn(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.provide(CLOCK, Clock(1))
    fiber = await ctx.plugin(consumer(log))

    await handle.dispose()

    assert fiber.state is FiberState.PENDING
    assert log == ["load 1", "unload 1"]


async def test_a_plugin_reloads_when_its_service_returns(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.provide(CLOCK, Clock(1))
    fiber = await ctx.plugin(consumer(log))
    await handle.dispose()

    ctx.provide(CLOCK, Clock(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1", "unload 1", "load 2"]


async def test_replacing_a_provider_reloads_its_dependents(ctx: Context) -> None:
    log: list[str] = []
    first = await ctx.plugin(clock_provider(1))
    fiber = await ctx.plugin(consumer(log))
    await ctx.settled()

    await first.dispose()
    await ctx.plugin(clock_provider(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1", "unload 1", "load 2"]


async def test_a_dependent_loads_only_after_its_provider_is_active(ctx: Context) -> None:
    provided = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        provided.set()
        await release.wait()
        log.append("provider ready")

    dependent = ctx.plugin(consumer(log))
    ctx.plugin(Plugin("provider", provider))
    await asyncio.wait_for(provided.wait(), timeout=1)
    await asyncio.sleep(0)
    assert dependent.state is FiberState.PENDING

    release.set()
    await ctx.settled()

    assert dependent.state is FiberState.ACTIVE
    assert log == ["provider ready", "load 1"]


async def test_dependents_unload_before_their_provider_cleans_up(ctx: Context) -> None:
    log: list[str] = []

    def provider(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("provider resource released"))
        inner.provide(CLOCK, Clock(1))
        inner.effect(lambda: lambda: log.append("provider late effect undone"))
        return lambda: log.append("provider disposer")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))
    dependent = await ctx.plugin(consumer(log))
    await ctx.settled()

    await provider_fiber.dispose()

    assert log == [
        "load 1",
        "unload 1",
        "provider disposer",
        "provider late effect undone",
        "provider resource released",
    ]
    assert dependent.state is FiberState.PENDING


async def test_a_chain_of_dependents_unloads_from_the_far_end(ctx: Context) -> None:
    log: list[str] = []

    def first(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, Clock(1))
        return lambda: log.append("first")

    def second(inner: Context, config: object) -> Disposer:
        inner.provide(STORE, {"now": inner.use(CLOCK).now})
        return lambda: log.append("second")

    def third(inner: Context, config: object) -> Disposer:
        inner.provide(INDEX, sorted(inner.use(STORE)))
        return lambda: log.append("third")

    ctx.plugin(Plugin("third", third, inject=(STORE,)))
    ctx.plugin(Plugin("second", second, inject=(CLOCK,)))
    head = ctx.plugin(Plugin("first", first))
    await ctx.settled()
    assert ctx.use(INDEX) == ["now"]

    await head.dispose()

    assert log == ["third", "second", "first"]
    assert [fiber.state for fiber in ctx.fibers()] == [FiberState.PENDING, FiberState.PENDING]


async def test_disposing_the_root_unloads_dependents_first() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: lambda: log.append("root resource released"))
    ctx.provide(CLOCK, Clock(1))
    await ctx.plugin(consumer(log))

    await ctx.fiber.dispose()

    assert log == ["load 1", "unload 1", "root resource released"]


async def test_using_an_undeclared_service_is_an_error(ctx: Context) -> None:
    ctx.provide(CLOCK, Clock(1))

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    with pytest.raises(ServiceNotInjectedError, match=r"'sneaky'.*'clock'.*inject"):
        await ctx.plugin(Plugin("sneaky", apply))


async def test_a_child_must_inject_a_service_its_parent_provides(ctx: Context) -> None:
    def child(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    def parent(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        inner.plugin(Plugin("careless", child))
        inner.plugin(Plugin("careful", child, inject=(CLOCK,)))

    await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    careless_fiber, careful_fiber = ctx.fibers()[1:]
    assert careless_fiber.state is FiberState.FAILED
    assert isinstance(careless_fiber.error, ServiceNotInjectedError)
    assert careful_fiber.state is FiberState.ACTIVE


async def test_get_reads_an_optional_service_without_declaring_it(ctx: Context) -> None:
    clock = Clock(1)
    ctx.provide(CLOCK, clock)
    seen: list[Clock | None] = []

    def apply(inner: Context, config: object) -> None:
        seen.append(inner.get(CLOCK))

    await ctx.plugin(Plugin("optional", apply))

    assert seen == [clock]


async def test_a_child_may_use_what_its_parent_injects(ctx: Context) -> None:
    ctx.provide(CLOCK, Clock(1))
    seen: list[int] = []

    def child(inner: Context, config: object) -> None:
        seen.append(inner.use(CLOCK).now)

    async def parent(inner: Context, config: object) -> None:
        await inner.plugin(Plugin("child", child))

    await ctx.plugin(Plugin("parent", parent, inject=(CLOCK,)))

    assert seen == [1]


async def test_a_failed_plugin_is_retried_when_its_service_returns(ctx: Context) -> None:
    attempts: list[int] = []

    def apply(inner: Context, config: object) -> None:
        attempts.append(inner.use(CLOCK).now)
        if len(attempts) == 1:
            raise RuntimeError("first attempt fails")

    handle = ctx.provide(CLOCK, Clock(1))
    fiber = ctx.plugin(Plugin("flaky", apply, inject=(CLOCK,)))
    with pytest.raises(RuntimeError, match="first attempt fails"):
        await fiber
    assert fiber.state is FiberState.FAILED

    await handle.dispose()
    assert fiber.state is FiberState.PENDING
    assert fiber.error is None

    ctx.provide(CLOCK, Clock(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert attempts == [1, 2]


async def test_a_failed_plugin_is_not_retried_while_its_services_are_unchanged(
    ctx: Context,
) -> None:
    attempts: list[int] = []

    def apply(inner: Context, config: object) -> None:
        attempts.append(inner.use(CLOCK).now)
        raise RuntimeError("always fails")

    ctx.provide(CLOCK, Clock(1))
    fiber = ctx.plugin(Plugin("broken", apply, inject=(CLOCK, STORE)))
    store = ctx.provide(STORE, {})
    await ctx.settled()
    assert fiber.state is FiberState.FAILED

    ctx.provide(INDEX, [])
    await ctx.settled()
    assert attempts == [1]

    await store.dispose()
    ctx.provide(STORE, {})
    await ctx.settled()
    assert attempts == [1, 1]


async def test_inject_is_read_from_a_module_like_plugin(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    fiber = await ctx.plugin(SimpleNamespace(name="module", apply=apply, inject=[CLOCK]))
    assert fiber.state is FiberState.PENDING

    ctx.provide(CLOCK, Clock(1))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE


@pytest.mark.parametrize("inject", ["clock", [CLOCK, "store"], {CLOCK}])
def test_inject_must_be_a_list_of_service_keys(inject: object) -> None:
    def apply(inner: Context, config: object) -> None:
        return None

    with pytest.raises(InvalidPluginError, match="list of service keys"):
        resolve_plugin(SimpleNamespace(name="module", apply=apply, inject=inject))


async def test_a_provider_below_its_dependent_does_not_deadlock(ctx: Context) -> None:
    """An unloading plugin must not wait for itself through a service of its child."""
    gate = asyncio.Event()
    unloading = asyncio.Event()
    child_contexts: list[Context] = []

    def child(inner: Context, config: object) -> None:
        child_contexts.append(inner)

    async def parent(inner: Context, config: object) -> None:
        await inner.plugin(Plugin("child", child))

        async def slow() -> None:
            unloading.set()
            await gate.wait()

        inner.effect(lambda: slow)

    handle = ctx.provide(CLOCK, Clock(1))
    parent_fiber = await ctx.plugin(Plugin("parent", parent, inject=(CLOCK,)))
    child_fiber = child_contexts[0].fiber

    withdrawal = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(unloading.wait(), timeout=1)
    assert parent_fiber.state is FiberState.UNLOADING
    child_contexts[0].provide(CLOCK, Clock(2))

    gate.set()
    await asyncio.wait_for(withdrawal, timeout=1)
    await ctx.settled()

    assert child_fiber.state is FiberState.DISPOSED
    assert parent_fiber.state is FiberState.PENDING
    assert ctx.get(CLOCK) is None
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_dependencies.py -q --timeout=10`

Expected: `20 failed, 1 passed in 0.84s`，第一个错误是 `TypeError: Plugin.__init__() got an unexpected keyword argument 'inject'`

**Step 3: 写实现**

修改 `src/tack/kernel/fiber.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/fiber.py
+++ b/src/tack/kernel/fiber.py
@@ -51,11 +51,17 @@ class FiberState(enum.Enum):
 
 @dataclass(slots=True, eq=False)
 class Provision:
-    """A service value and the fiber that provides it."""
+    """A service value and the fiber that provides it.
+
+    ``serial`` is unique to this act of providing. A dependent records the
+    serials it loaded against, so it reloads whenever a service is provided
+    anew, even by the same fiber.
+    """
 
     key: ServiceKey[Any]
     value: Any
     fiber: Fiber
+    serial: int
 
 
 class KernelState:
@@ -91,6 +97,64 @@ class KernelState:
             return provision
         return None
 
+    def available(self, key: ServiceKey[Any]) -> Provision | None:
+        """Return the provision of ``key`` if its provider is active."""
+        provision = self.services.get(key)
+        if provision is None or provision.fiber.state is not FiberState.ACTIVE:
+            return None
+        return provision
+
+    def declared(self, key: ServiceKey[Any], viewer: Fiber) -> bool:
+        """Return whether ``viewer`` is entitled to rely on ``key``.
+
+        A plugin may rely on a service that it provides itself, and on one that
+        it or an enclosing plugin injects: the kernel unloads the plugin before
+        an injected service goes away. Code outside every plugin may rely on
+        any service.
+        """
+        provision = self.services.get(key)
+        provider = None if provision is None else provision.fiber
+        in_own_plugin = True
+        inside_a_plugin = False
+        for fiber in viewer.lineage():
+            if in_own_plugin and fiber is provider:
+                return True
+            if fiber.plugin is not None:
+                if key in fiber.plugin.inject:
+                    return True
+                in_own_plugin = False
+                inside_a_plugin = True
+        return not inside_a_plugin
+
+    def notify(self, key: ServiceKey[Any]) -> None:
+        """Re-evaluate every fiber that injects ``key``."""
+        for fiber in tuple(self.fibers.values()):
+            if fiber.injects(key):
+                fiber._refresh()
+
+    def announce(self, provider: Fiber) -> None:
+        """Re-evaluate every fiber that injects a service of ``provider``."""
+        for provision in tuple(self.services.values()):
+            if provision.fiber is provider:
+                self.notify(provision.key)
+
+    async def dependents_settled(self, key: ServiceKey[Any], provider: Fiber) -> None:
+        """Wait until the fibers that inject ``key`` have finished changing state.
+
+        The provider and the fibers above it are not waited for. One of them
+        may be the very fiber whose unload is waiting here.
+        """
+        above = set(provider.lineage())
+        for fiber in tuple(self.fibers.values()):
+            if fiber.injects(key) and fiber not in above:
+                await fiber.settled()
+
+    async def all_dependents_settled(self, provider: Fiber) -> None:
+        """Wait for the dependents of every service that ``provider`` provides."""
+        for provision in tuple(self.services.values()):
+            if provision.fiber is provider:
+                await self.dependents_settled(provision.key, provider)
+
 
 class EffectHandle:
     """A registered effect. Disposing the handle undoes the effect once."""
@@ -220,6 +284,17 @@ class Fiber:
             yield fiber
             fiber = fiber.parent
 
+    def injects(self, key: ServiceKey[Any]) -> bool:
+        """Return whether this fiber's plugin requires ``key``."""
+        return self.plugin is not None and key in self.plugin.inject
+
+    @property
+    def missing(self) -> tuple[ServiceKey[Any], ...]:
+        """The injected services that are not available right now."""
+        inject = () if self.plugin is None else self.plugin.inject
+        available = self._kernel.available
+        return tuple(key for key in inject if available(key) is None)
+
     def __await__(self) -> Generator[Any, None, Fiber]:
         return self._ready().__await__()
 
@@ -267,13 +342,18 @@ class Fiber:
         if self._disposed:
             return
         self._disposed = True
-        self._kernel.fibers.pop(self.uid, None)
         self._refresh()
 
     def _compute_epoch(self) -> Epoch | None:
         if self._disposed or self._broken:
             return None
-        return ()
+        serials: list[int] = []
+        for key in () if self.plugin is None else self.plugin.inject:
+            provision = self._kernel.available(key)
+            if provision is None:
+                return None
+            serials.append(provision.serial)
+        return tuple(serials)
 
     def _refresh(self) -> None:
         wanted = self._compute_epoch()
@@ -310,7 +390,7 @@ class Fiber:
         finally:
             driver = self._driver
             self._driver = None
-            if driver is not None:
+            if driver is not None:  # pragma: no branch - always set while driving
                 self._kernel.drivers.discard(driver)
 
     async def _load(self) -> None:
@@ -343,6 +423,7 @@ class Fiber:
 
     async def _unload(self) -> None:
         self._set_state(FiberState.UNLOADING)
+        await self._kernel.all_dependents_settled(self)
         await self._undo_effects()
         self._loaded = None
 
@@ -357,13 +438,14 @@ class Fiber:
         effects = self._effects
         if effects and effects[-1] is handle:
             effects.pop()
-        elif handle in effects:
+        elif handle in effects:  # pragma: no branch - a handle is forgotten once
             effects.remove(handle)
 
     def _rest(self) -> None:
         if self._loaded is not None:
             self._set_state(FiberState.ACTIVE)
         elif self._disposed:
+            self._kernel.fibers.pop(self.uid, None)
             self._set_state(FiberState.DISPOSED)
         elif self.error is not None:
             self._set_state(FiberState.FAILED)
@@ -371,4 +453,9 @@ class Fiber:
             self._set_state(FiberState.PENDING)
 
     def _set_state(self, state: FiberState) -> None:
+        previous = self.state
+        if previous is state:
+            return
         self.state = state
+        if FiberState.ACTIVE in (previous, state):
+            self._kernel.announce(self)
```

修改 `src/tack/kernel/plugin.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/plugin.py
+++ b/src/tack/kernel/plugin.py
@@ -2,13 +2,14 @@
 
 from __future__ import annotations
 
-from collections.abc import Awaitable, Callable
-from dataclasses import dataclass
+from collections.abc import Awaitable, Callable, Sequence
+from dataclasses import dataclass, field
 from typing import TYPE_CHECKING, Any, TypeAlias, cast
 
 from pydantic import BaseModel
 
 from tack.kernel.errors import InvalidPluginError
+from tack.kernel.services import ServiceKey
 
 if TYPE_CHECKING:
     from tack.kernel.context import Context
@@ -27,21 +28,25 @@ class Plugin:
         apply: Called with the plugin's own context and its config each time
             the plugin loads. It may be a coroutine function. It may return a
             disposer, which runs first when the plugin unloads.
+        inject: The services the plugin requires. The plugin loads only while
+            all of them are available, unloads when one is withdrawn, and
+            loads again when it returns.
         config: A pydantic model that the raw config is validated against, or
             ``None`` to hand the raw config to ``apply`` unchanged.
     """
 
     name: str
     apply: Apply
-    config: type[BaseModel] | None = None
+    inject: tuple[ServiceKey[Any], ...] = field(default=(), kw_only=True)
+    config: type[BaseModel] | None = field(default=None, kw_only=True)
 
 
 def resolve_plugin(candidate: object) -> Plugin:
     """Turn any accepted plugin shape into a :class:`Plugin`.
 
     Three shapes are accepted: a :class:`Plugin`; a module or object with an
-    ``apply`` callable and optional ``name`` and ``Config`` attributes; and a
-    bare callable, which is its own ``apply``.
+    ``apply`` callable and optional ``name``, ``inject`` and ``Config``
+    attributes; and a bare callable, which is its own ``apply``.
 
     Raises:
         InvalidPluginError: ``candidate`` has none of these shapes.
@@ -61,4 +66,14 @@ def resolve_plugin(candidate: object) -> Plugin:
     config = getattr(candidate, "Config", None)
     if config is not None and not (isinstance(config, type) and issubclass(config, BaseModel)):
         raise InvalidPluginError(f"the Config of plugin {name!r} must be a pydantic model")
-    return Plugin(name, cast("Apply", apply), config)
+    declared: object = getattr(candidate, "inject", ())
+    if not isinstance(declared, list | tuple):
+        raise InvalidPluginError(f"the inject of plugin {name!r} must be a list of service keys")
+    inject: list[ServiceKey[Any]] = []
+    for key in cast("Sequence[object]", declared):
+        if not isinstance(key, ServiceKey):
+            raise InvalidPluginError(
+                f"the inject of plugin {name!r} must be a list of service keys"
+            )
+        inject.append(cast("ServiceKey[Any]", key))
+    return Plugin(name, cast("Apply", apply), inject=tuple(inject), config=config)
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -8,7 +8,11 @@ import asyncio
 from collections.abc import Callable
 from typing import Any, overload
 
-from tack.kernel.errors import ServiceConflictError, ServiceUnavailableError
+from tack.kernel.errors import (
+    ServiceConflictError,
+    ServiceNotInjectedError,
+    ServiceUnavailableError,
+)
 from tack.kernel.events import (
     BailEvent,
     BailListener,
@@ -92,30 +96,40 @@ class Context:
 
         The service is withdrawn when the owning fiber unloads or the returned
         handle is disposed. Other fibers see it once the owning fiber is active.
+        Plugins that inject the service load when it appears. When it is
+        withdrawn they unload first, and the withdrawal waits for them.
 
         Raises:
             ServiceConflictError: Another provider already holds the name.
             InactiveEffectError: The owning fiber is not loaded.
         """
-        services = self._kernel.services
+        kernel = self._kernel
         fiber = self.fiber
 
-        def withdraw() -> None:
-            del services[key]
+        async def withdraw() -> None:
+            del kernel.services[key]
+            kernel.notify(key)
+            await kernel.dependents_settled(key, fiber)
 
         def setup() -> Disposer:
-            existing = services.get(key)
+            existing = kernel.services.get(key)
             if existing is not None:
                 raise ServiceConflictError(
                     f"service {key.name!r} is already provided by {existing.fiber.name!r}"
                 )
-            services[key] = Provision(key, value, fiber)
+            kernel.services[key] = Provision(key, value, fiber, kernel.next_uid())
+            kernel.notify(key)
             return withdraw
 
         return self.effect(setup)
 
     def get(self, key: ServiceKey[ServiceT]) -> ServiceT | None:
-        """Return the service named by ``key``, or ``None`` if it is not available."""
+        """Return the service named by ``key``, or ``None`` if it is not available.
+
+        Use this for a service the caller can do without. Nothing unloads the
+        caller when such a service goes away, so ask again each time instead of
+        keeping the result.
+        """
         provision = self._kernel.lookup(key, self.fiber)
         return None if provision is None else provision.value
 
@@ -123,9 +137,16 @@ class Context:
         """Return the service named by ``key``.
 
         Raises:
+            ServiceNotInjectedError: The caller is inside a plugin, and neither
+                that plugin nor an enclosing one injects or provides the service.
             ServiceUnavailableError: No active plugin provides the service.
         """
-        provision = self._kernel.lookup(key, self.fiber)
+        kernel = self._kernel
+        if not kernel.declared(key, self.fiber):
+            raise ServiceNotInjectedError(
+                f"{self.fiber.name!r} uses service {key.name!r} without declaring it in inject"
+            )
+        provision = kernel.lookup(key, self.fiber)
         if provision is None:
             raise ServiceUnavailableError(f"service {key.name!r} is not available")
         return provision.value
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_dependencies.py -q`

Expected: `21 passed in 0.65s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`121 passed in 1.73s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`18 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): drive plugin lifecycles from the services they inject"
```

---

### Task 9: 作用域

实现按 Agent 隔离监听器的机制。之后每个 Agent 都有自己的作用域。

**要点**

- 作用域是一个没有插件的 Fiber，加上上下文上的一个标记。
- 监听器记住注册它的上下文的标记。分发时用分发方上下文的标记来筛选。
- 不带作用域的分发会送到所有监听器，包括各个作用域里的。
- 作用域不嵌套。在作用域里再建作用域，新的标记取代旧的。

**Files:**

- Modify: `src/tack/kernel/events.py`
- Modify: `src/tack/kernel/context.py`
- Test（新建）: `tests/kernel/test_scopes.py`

**Step 1: 写测试**

新建 `tests/kernel/test_scopes.py`：

```python
import pytest

from tack.kernel import (
    BailEvent,
    Context,
    EmitEvent,
    FiberState,
    InactiveEffectError,
    Next,
    ParallelEvent,
    Plugin,
    SerialEvent,
    ServiceKey,
    WaterfallEvent,
)

PING = EmitEvent[str]("test/ping")
ASK = BailEvent[str, str]("test/ask")
FLUSH = ParallelEvent[str]("test/flush")
DECIDE = SerialEvent[str, str]("test/decide")
RENDER = WaterfallEvent[str, str]("test/render")
NOTES = ServiceKey[list[str]]("notes")


class Agent:
    """Stands in for whatever object identifies a scope."""


async def test_a_scope_is_active_as_soon_as_it_is_created(ctx: Context) -> None:
    agent = Agent()

    scoped = ctx.scope(agent)

    assert scoped.scope_key is agent
    assert scoped.fiber.state is FiberState.ACTIVE
    assert scoped.fiber.name == "scope"
    assert scoped.fiber.parent is ctx.fiber
    assert ctx.scope_key is None
    assert ctx.fibers() == ()


async def test_a_scoped_dispatch_reaches_its_scope_and_global_listeners(ctx: Context) -> None:
    log: list[str] = []
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    ctx.on(PING, lambda payload: log.append("global"))
    first.on(PING, lambda payload: log.append("first"))
    second.on(PING, lambda payload: log.append("second"))

    first.emit(PING, "x")

    assert log == ["global", "first"]


async def test_an_unscoped_dispatch_reaches_every_listener(ctx: Context) -> None:
    log: list[str] = []
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    ctx.on(PING, lambda payload: log.append("global"))
    first.on(PING, lambda payload: log.append("first"))
    second.on(PING, lambda payload: log.append("second"))

    ctx.emit(PING, "x")

    assert log == ["global", "first", "second"]


async def test_every_dispatch_mode_respects_the_scope(ctx: Context) -> None:
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    flushed: list[str] = []

    async def render(payload: str, next: Next[str, str]) -> str:
        return f"second({await next(payload)})"

    async def final(payload: str) -> str:
        return payload

    second.on(ASK, lambda payload: "second")
    second.on(DECIDE, lambda payload: "second")
    second.on(FLUSH, flushed.append)
    second.on(RENDER, render)

    assert first.bail(ASK, "x") is None
    assert await first.serial(DECIDE, "x") is None
    await first.parallel(FLUSH, "x")
    assert flushed == []
    assert await first.waterfall(RENDER, "x", final) == "x"

    assert second.bail(ASK, "x") == "second"
    assert await second.serial(DECIDE, "x") == "second"
    await second.parallel(FLUSH, "x")
    assert flushed == ["x"]
    assert await second.waterfall(RENDER, "x", final) == "second(x)"


async def test_a_plugin_loaded_in_a_scope_stays_in_that_scope(ctx: Context) -> None:
    log: list[str] = []
    agent = Agent()
    first = ctx.scope(agent)
    second = ctx.scope(Agent())

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)

    fiber = await first.plugin(Plugin("listener", apply))

    second.emit(PING, "second")
    first.emit(PING, "first")

    assert fiber.ctx.scope_key is agent
    assert log == ["first"]
    assert ctx.fibers() == (fiber,)


async def test_disposing_a_scope_undoes_what_was_registered_through_it(ctx: Context) -> None:
    log: list[str] = []
    scoped = ctx.scope(Agent())
    scoped.on(PING, log.append)
    scoped.effect(lambda: lambda: log.append("effect undone"))
    scoped.provide(NOTES, ["a note"])
    fiber = await scoped.plugin(Plugin("inside", lambda inner, config: None))
    assert ctx.use(NOTES) == ["a note"]

    await scoped.fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["effect undone"]
    assert scoped.fiber.state is FiberState.DISPOSED
    assert fiber.state is FiberState.DISPOSED
    assert ctx.get(NOTES) is None
    assert ctx.fiber.state is FiberState.ACTIVE


async def test_a_scope_is_disposed_with_the_fiber_that_created_it(ctx: Context) -> None:
    log: list[str] = []
    scopes: list[Context] = []

    def apply(inner: Context, config: object) -> None:
        scoped = inner.scope(Agent())
        scoped.on(PING, log.append)
        scopes.append(scoped)

    fiber = await ctx.plugin(Plugin("owner", apply))
    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == []
    assert scopes[0].fiber.state is FiberState.DISPOSED


async def test_a_nested_scope_replaces_the_key(ctx: Context) -> None:
    log: list[str] = []
    outer = ctx.scope(Agent())
    inner = outer.scope(Agent())
    outer.on(PING, lambda payload: log.append("outer"))
    inner.on(PING, lambda payload: log.append("inner"))

    inner.emit(PING, "x")

    assert log == ["inner"]
    assert inner.fiber.parent is outer.fiber


async def test_a_scope_inside_a_plugin_may_use_the_plugins_own_service(ctx: Context) -> None:
    seen: list[list[str]] = []

    def apply(inner: Context, config: object) -> None:
        inner.provide(NOTES, ["own"])
        seen.append(inner.scope(Agent()).use(NOTES))

    await ctx.plugin(Plugin("owner", apply))

    assert seen == [["own"]]


async def test_scopes_cannot_be_created_on_a_disposed_fiber() -> None:
    ctx = Context()
    await ctx.fiber.dispose()

    with pytest.raises(InactiveEffectError):
        ctx.scope(Agent())
```

**Step 2: 运行测试，确认失败**

Run: `uv run pytest tests/kernel/test_scopes.py -q --timeout=10`

Expected: `10 failed in 0.71s`，第一个错误是 `AttributeError: 'Context' object has no attribute 'scope'`

**Step 3: 写实现**

修改 `src/tack/kernel/events.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/events.py
+++ b/src/tack/kernel/events.py
@@ -96,9 +96,10 @@ _PLAIN_LISTENER_MODES = frozenset({"emit", "bail"})
 
 @dataclass(slots=True, eq=False)
 class Hook:
-    """One registered listener."""
+    """One registered listener, and the scope it was registered in, if any."""
 
     callback: Callable[..., Any]
+    scope_key: object | None = None
 
 
 class HookTable:
@@ -149,9 +150,15 @@ class HookTable:
         if not hooks:
             del self._hooks[event.name]
 
-    def select(self, event: Event, kind: type[Event]) -> tuple[Hook, ...]:
+    def select(
+        self, event: Event, kind: type[Event], scope_key: object | None = None
+    ) -> tuple[Hook, ...]:
         """Return the listeners one dispatch of ``event`` should reach.
 
+        A dispatch made outside every scope reaches all listeners. A dispatch
+        made in a scope reaches the listeners of that scope and those
+        registered outside every scope.
+
         The result is a snapshot: listeners added or removed while the dispatch
         runs do not change who it reaches.
 
@@ -161,7 +168,12 @@ class HookTable:
         if not isinstance(event, kind):
             raise EventModeError(f"{event!r} is dispatched with {event.mode}(), not {kind.mode}()")
         self.declare(event)
-        return tuple(self._hooks.get(event.name, ()))
+        hooks = self._hooks.get(event.name, ())
+        if scope_key is None:
+            return tuple(hooks)
+        return tuple(
+            hook for hook in hooks if hook.scope_key is None or hook.scope_key is scope_key
+        )
 
 
 def run_emit(hooks: Sequence[Hook], event: Event, payload: object) -> None:
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -45,24 +45,54 @@ class Context:
 
     Everything registered through a context belongs to the fiber that owns the
     context and is undone when that fiber unloads. Calling ``Context()``
-    creates the root context of a new application.
+    creates the root context of a new application. The kernel is driven by
+    asyncio and must be used from within a running event loop.
+
+    Attributes:
+        fiber: The fiber that owns what is registered through this context.
+        scope_key: The object identifying the scope this context belongs to,
+            or ``None`` outside every scope.
     """
 
-    __slots__ = ("_kernel", "fiber")
+    __slots__ = ("_kernel", "fiber", "scope_key")
 
     def __init__(self) -> None:
         self._kernel = KernelState()
         self.fiber = Fiber(self._kernel, None)
         self.fiber.ctx = self
+        self.scope_key: object | None = None
 
     @classmethod
-    def _owned_by(cls, kernel: KernelState, fiber: Fiber) -> Context:
+    def _owned_by(cls, kernel: KernelState, fiber: Fiber, scope_key: object | None) -> Context:
         ctx = cls.__new__(cls)
         ctx._kernel = kernel
         ctx.fiber = fiber
+        ctx.scope_key = scope_key
         fiber.ctx = ctx
         return ctx
 
+    def scope(self, key: object) -> Context:
+        """Create a scope identified by ``key`` and return its context.
+
+        Listeners registered through the returned context, and by plugins
+        loaded through it, hear only the events dispatched in the same scope
+        and those dispatched outside every scope. Events dispatched through it
+        reach those listeners and the ones registered outside every scope. A
+        scope created inside another scope replaces the key.
+
+        The scope is disposed with the owning fiber, or earlier by disposing
+        the ``fiber`` of the returned context.
+
+        Raises:
+            InactiveEffectError: The owning fiber is not loaded.
+        """
+        parent = self.fiber
+        parent._require_loaded()
+        fiber = Fiber(self._kernel, parent)
+        scoped = Context._owned_by(self._kernel, fiber, key)
+        fiber._handle = parent.effect(lambda: fiber._dispose_owned)
+        return scoped
+
     def plugin(self, plugin: object, config: object = None) -> Fiber:
         """Load ``plugin`` as a child of the owning fiber.
 
@@ -85,7 +115,7 @@ class Context:
         parent = self.fiber
         parent._require_loaded()
         fiber = Fiber(self._kernel, parent, spec, config)
-        Context._owned_by(self._kernel, fiber)
+        Context._owned_by(self._kernel, fiber, self.scope_key)
         fiber._handle = parent.effect(lambda: fiber._dispose_owned)
         self._kernel.fibers[fiber.uid] = fiber
         fiber._refresh()
@@ -230,7 +260,7 @@ class Context:
             InactiveEffectError: The owning fiber is not loaded.
         """
         hooks = self._kernel.hooks
-        hook = Hook(listener)
+        hook = Hook(listener, self.scope_key)
 
         def setup() -> Disposer:
             hooks.add(event, hook, prepend=prepend)
@@ -238,26 +268,29 @@ class Context:
 
         return self.effect(setup)
 
+    def _select(self, event: Event, kind: type[Event]) -> tuple[Hook, ...]:
+        return self._kernel.hooks.select(event, kind, self.scope_key)
+
     def emit(self, event: EmitEvent[PayloadT], payload: PayloadT) -> None:
         """Notify every listener of ``event``.
 
         A listener that raises is logged and does not stop the others.
         """
-        run_emit(self._kernel.hooks.select(event, EmitEvent), event, payload)
+        run_emit(self._select(event, EmitEvent), event, payload)
 
     def bail(self, event: BailEvent[PayloadT, ResultT], payload: PayloadT) -> ResultT | None:
         """Ask each listener in turn and return the first answer that is not ``None``.
 
         A listener that raises stops the dispatch and the error reaches the caller.
         """
-        return run_bail(self._kernel.hooks.select(event, BailEvent), payload)
+        return run_bail(self._select(event, BailEvent), payload)
 
     async def parallel(self, event: ParallelEvent[PayloadT], payload: PayloadT) -> None:
         """Run every listener of ``event`` concurrently and wait for all of them.
 
         A listener that raises is logged and does not stop the others.
         """
-        await run_parallel(self._kernel.hooks.select(event, ParallelEvent), event, payload)
+        await run_parallel(self._select(event, ParallelEvent), event, payload)
 
     async def serial(
         self, event: SerialEvent[PayloadT, ResultT], payload: PayloadT
@@ -266,7 +299,7 @@ class Context:
 
         A listener that raises stops the dispatch and the error reaches the caller.
         """
-        return await run_serial(self._kernel.hooks.select(event, SerialEvent), payload)
+        return await run_serial(self._select(event, SerialEvent), payload)
 
     async def waterfall(
         self,
@@ -281,5 +314,4 @@ class Context:
         different payload on, call ``next`` again to retry, or return without
         calling it to short-circuit. Errors reach the caller.
         """
-        hooks = self._kernel.hooks.select(event, WaterfallEvent)
-        return await run_waterfall(hooks, payload, final)
+        return await run_waterfall(self._select(event, WaterfallEvent), payload, final)
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_scopes.py -q`

Expected: `10 passed in 0.53s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`131 passed in 1.98s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`19 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): add scopes for per-agent registrations"
```

---

### Task 10: 生命周期的属性测试

用随机生成的插件图检查依赖驱动的生命周期。这个任务只加测试。

**要点**

- 每个场景声明几个服务和几个插件。每个服务有一个固定的拥有者：根上下文、某个插件，或者没有。
- 场景对真实的内核执行一串随机步骤：加载、销毁、提供、撤回。
- 每次内核静止下来，测试核对四件事：插件为 `ACTIVE` 当且仅当它的服务都可用；它拿到的是当前的服务实例；加载和卸载次数配平；提供方清理时没有依赖方还处在已加载状态。
- 期望的活跃集合按最小不动点计算。互相依赖成环的插件永远不会加载。

**Files:**

- Test（新建）: `tests/kernel/test_lifecycle_properties.py`

**Step 1: 写测试**

新建 `tests/kernel/test_lifecycle_properties.py`：

```python
"""Property test: random plugin graphs under random loads, disposals and withdrawals.

Each generated scenario declares a few services and a few plugins. Every
service has one owner: the root context, one of the plugins, or nobody. Every
plugin injects some of the services. A scenario then plays a random sequence of
steps against a real kernel and checks, each time the kernel comes to rest,
that the kernel is exactly where the declarations say it should be.
"""

import asyncio
from collections import Counter
from dataclasses import dataclass

from hypothesis import given, settings
from hypothesis import strategies as st

from tack.kernel import Context, Disposer, EffectHandle, Fiber, FiberState, Plugin, ServiceKey

ROOT = -1
NOBODY = -2


class Token:
    """Stands in for one provided service instance."""


@dataclass(frozen=True)
class Scenario:
    owners: tuple[int, ...]
    """The owner of each service: a plugin index, ``ROOT`` or ``NOBODY``."""
    injects: tuple[frozenset[int], ...]
    """The services each plugin injects."""
    steps: tuple[tuple[str, int], ...]


PLUGIN_ACTIONS = ("add", "dispose", "dispose_later")
# Loads and provisions are listed more often than their opposites, so that most
# scenarios spend their steps on a populated graph rather than an empty one.
ACTIONS = (
    *("add",) * 5,
    *("provide",) * 3,
    *("withdraw",) * 2,
    *("dispose",) * 2,
    "dispose_later",
    "yield",
    "settle",
)


@st.composite
def scenarios(draw: st.DrawFn) -> Scenario:
    plugin_count = draw(st.integers(min_value=2, max_value=4))
    service_count = draw(st.integers(min_value=1, max_value=3))
    owner = st.sampled_from([ROOT, ROOT, NOBODY, *range(plugin_count), *range(plugin_count)])
    owners = tuple(draw(owner) for _ in range(service_count))
    services = st.integers(min_value=0, max_value=service_count - 1)
    injects = tuple(frozenset(draw(st.sets(services, max_size=2))) for _ in range(plugin_count))
    raw_steps = draw(
        st.lists(
            st.tuples(st.sampled_from(ACTIONS), st.integers(min_value=0, max_value=11)),
            min_size=4,
            max_size=40,
        )
    )
    steps = tuple(
        (action, number % (plugin_count if action in PLUGIN_ACTIONS else service_count))
        for action, number in raw_steps
    )
    return Scenario(owners, injects, steps)


class Run:
    """Plays one scenario against a kernel and tracks what should be true."""

    def __init__(self, scenario: Scenario) -> None:
        self.scenario = scenario
        self.ctx = Context()
        self.keys = [ServiceKey[Token](f"service-{s}") for s in range(len(scenario.owners))]
        self.fibers: dict[int, Fiber] = {}
        self.retired: list[Fiber] = []
        self.handles: dict[int, EffectHandle] = {}
        self.tokens: dict[int, Token] = {}
        self.seen: dict[int, dict[int, Token]] = {}
        self.loaded: set[int] = set()
        self.loads: Counter[int] = Counter()
        self.unloads: Counter[int] = Counter()
        self.pending: list[asyncio.Future[None]] = []
        self.violations: list[str] = []

    def dependents_of(self, service: int) -> list[int]:
        return [p for p, injects in enumerate(self.scenario.injects) if service in injects]

    def plugin(self, index: int) -> Plugin:
        provides = [s for s, owner in enumerate(self.scenario.owners) if owner == index]
        injects = sorted(self.scenario.injects[index])

        def apply(ctx: Context, config: object) -> Disposer:
            self.seen[index] = {s: ctx.use(self.keys[s]) for s in injects}
            for service in provides:
                token = Token()
                ctx.provide(self.keys[service], token)
                self.tokens[service] = token
            self.loaded.add(index)
            self.loads[index] += 1

            def undo() -> None:
                for service in provides:
                    for dependent in self.dependents_of(service):
                        if dependent != index and dependent in self.loaded:
                            self.violations.append(
                                f"plugin {index} cleaned up while its dependent "
                                f"{dependent} was still loaded"
                            )
                    del self.tokens[service]
                self.loaded.discard(index)
                self.unloads[index] += 1

            return undo

        return Plugin(f"plugin-{index}", apply, inject=tuple(self.keys[s] for s in injects))

    async def step(self, action: str, target: int) -> None:
        if action == "add":
            previous = [fiber for fiber in self.retired if fiber.name == f"plugin-{target}"]
            if target not in self.fibers and all(
                fiber.state is FiberState.DISPOSED for fiber in previous
            ):
                self.fibers[target] = self.ctx.plugin(self.plugin(target))
        elif action in ("dispose", "dispose_later"):
            fiber = self.fibers.pop(target, None)
            if fiber is not None:
                self.retired.append(fiber)
                disposal = asyncio.ensure_future(fiber.dispose())
                if action == "dispose":
                    await disposal
                else:
                    self.pending.append(disposal)
        elif action == "provide":
            if self.scenario.owners[target] == ROOT and target not in self.handles:
                token = Token()
                self.handles[target] = self.ctx.provide(self.keys[target], token)
                self.tokens[target] = token
        elif action == "withdraw":
            handle = self.handles.pop(target, None)
            if handle is not None:
                await handle.dispose()
                del self.tokens[target]
                for dependent in self.dependents_of(target):
                    if dependent in self.loaded:
                        self.violations.append(
                            f"service {target} was withdrawn while its dependent "
                            f"{dependent} was still loaded"
                        )
        elif action == "yield":
            await asyncio.sleep(0)
        else:
            await self.settle()
            self.check()

    async def settle(self) -> None:
        while self.pending:
            pending, self.pending = self.pending, []
            await asyncio.gather(*pending)
        await self.ctx.settled()

    def expected_active(self) -> set[int]:
        """The least fixed point: who can be active given who is live and what root provides."""
        available = set(self.handles)
        active: set[int] = set()
        progressed = True
        while progressed:
            progressed = False
            for index in set(self.fibers) - active:
                if self.scenario.injects[index] <= available:
                    active.add(index)
                    available |= {
                        s for s, owner in enumerate(self.scenario.owners) if owner == index
                    }
                    progressed = True
        return active

    def check(self) -> None:
        active = self.expected_active()
        for index, fiber in self.fibers.items():
            expected = FiberState.ACTIVE if index in active else FiberState.PENDING
            assert fiber.state is expected, (index, fiber.state, expected)
            assert self.loads[index] - self.unloads[index] == (1 if index in active else 0)
            if index in active:
                for service in self.scenario.injects[index]:
                    assert self.seen[index][service] is self.tokens[service], (index, service)
        assert self.loaded == active
        for service, key in enumerate(self.keys):
            assert self.ctx.get(key) is self.tokens.get(service), service
        assert self.violations == []

    async def finish(self) -> None:
        """Withdraw one root service, then dispose the root, checking at each rest.

        The other root services stay provided, so that disposing the root also
        has dependents to unload.
        """
        await self.settle()
        self.check()
        if self.handles:
            await self.step("withdraw", min(self.handles))
            await self.settle()
            self.check()
        fibers = [*self.fibers.values(), *self.retired]
        await self.ctx.fiber.dispose()
        assert all(fiber.state is FiberState.DISPOSED for fiber in fibers)
        assert self.ctx.fibers() == ()
        assert self.loaded == set()
        assert self.loads == self.unloads
        assert all(self.ctx.get(key) is None for key in self.keys)
        assert self.violations == []


async def play(scenario: Scenario) -> None:
    run = Run(scenario)
    for action, target in scenario.steps:
        await run.step(action, target)
    await run.finish()


@settings(max_examples=300, deadline=None)
@given(scenarios())
def test_plugins_are_active_exactly_when_their_services_are(scenario: Scenario) -> None:
    asyncio.run(play(scenario))
```

**Step 2: 运行测试**

这个任务没有新的实现，测试写完就应当通过。它有没有检出能力，由下面的变异检查确认。

Run: `uv run pytest tests/kernel/test_lifecycle_properties.py -q --timeout=10`

Expected: `1 passed in 2.64s`

**Step 3: 写实现**

这个任务不改动实现。


**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_lifecycle_properties.py -q`

Expected: `1 passed in 2.54s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`132 passed in 5.10s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`20 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**变异检查：确认这个测试能发现错误**

临时删掉 `src/tack/kernel/fiber.py` 中 `_unload` 里的这一行：

```python
        await self._kernel.all_dependents_settled(self)
```

Run: `uv run pytest tests/kernel/test_lifecycle_properties.py -q`

Expected: 失败，断言信息里有 `cleaned up while its dependent`。在 8 个不同的随机种子下，这个改动每次都被发现。

Run: `git checkout -- src/tack/kernel/fiber.py`

确认文件已恢复后再提交。

**Step 5: 提交**

```bash
git add -A
git commit -m "test(kernel): check lifecycle invariants on random plugin graphs"
```

---

### Task 11: 加固：取消、重入与撤回时的竞态

任务 3 到 9 的内核在正常路径上是对的，但在三类情况下会出错：等待方或内核任务被取消，生命周期代码反过来等待自己所在的 Fiber，以及服务撤回和提供方清理同时进行。这个任务把这三类情况全部补上，并顺带加了状态事件、插件的类型约束和监听器检查。

**要点**

- 返回可等待对象的 disposer 改为在自己的任务里运行。调用方被取消时清理继续进行，父 Fiber 也不会抢在子 Fiber 之前完成卸载。
- 内核给生命周期代码打标记。标记放在上下文变量里，所以这段代码启动的任务会继承它。标记在代码结束的那一刻失效。
- 内核的任务一律显式设置标记。驱动任务不带标记。否则在 disposer 里启动的驱动会继承 disposer 的标记，卸载循环会空转而不让出事件循环。
- disposer 可以返回一个早先创建的任务。这种任务有自己的上下文，继承不到标记，所以内核另外按任务身份查找标记。被返回的任务一结束，标记立即失效。
- `waits_for(target, caller)` 计算 `target` 的卸载是否要等 `caller`。它沿两种关系展开：`target` 下面的 Fiber，以及注入了这些 Fiber 所持有服务的插件。
- 撤回服务时名字保留到依赖方卸载完。这样同一个服务不会有两代同时存在，提供方的卸载也总能找到还在卸载的依赖方。
- `settled()` 发现还有工作要做而驱动已经不在时，会重新启动驱动。被取消的驱动因此由下一个等待方接续。
- `Plugin` 改为按配置类型参数化。配置模型和 `apply` 的参数类型对不上时，pyright 会报错。
- 属性测试加入了慢的加载与清理，以及不等待完成的撤回。

**Files:**

- Modify: `src/tack/kernel/errors.py`
- Modify: `src/tack/kernel/events.py`
- Modify: `src/tack/kernel/fiber.py`
- Modify: `src/tack/kernel/plugin.py`
- Modify: `src/tack/kernel/context.py`
- Modify: `src/tack/kernel/__init__.py`
- Test（新建）: `tests/kernel/test_cancellation.py`
- Test（新建）: `tests/kernel/test_eager_tasks.py`
- Test（新建）: `tests/kernel/test_fiber_states.py`
- Test（新建）: `tests/kernel/test_reentrancy.py`
- Test（新建）: `tests/kernel/test_withdrawal.py`
- Test（修改）: `tests/kernel/test_dependencies.py`
- Test（修改）: `tests/kernel/test_events_sync.py`
- Test（修改）: `tests/kernel/test_lifecycle_properties.py`
- Test（修改）: `tests/kernel/test_plugins.py`
- Test（修改）: `tests/kernel/test_scopes.py`

**Step 1: 写测试**

新建 `tests/kernel/test_cancellation.py`：

```python
"""Cancelling a caller, or code that cancels itself, must not lose a cleanup."""

import asyncio
import logging

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EmitEvent,
    Fiber,
    FiberState,
    KernelError,
    ParallelEvent,
    Plugin,
    ServiceKey,
)

PING = EmitEvent[str]("test/ping")
FLUSH = ParallelEvent[str]("test/flush")
CLOCK = ServiceKey[int]("clock")


def nothing(ctx: Context, config: object) -> None:
    return None


async def a_few_turns() -> None:
    """Let every task that is ready to run take several steps."""
    for _ in range(5):
        await asyncio.sleep(0)


def driver_of(fiber: Fiber) -> asyncio.Task[object]:
    """Find the background task that is loading or unloading ``fiber``."""
    name = f"tack-fiber-{fiber.uid}"
    return next(task for task in asyncio.all_tasks() if task.get_name() == name)


async def test_cancelling_the_caller_does_not_abandon_a_cleanup() -> None:
    ctx = Context()
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def undo() -> None:
        started.set()
        await release.wait()
        log.append("cleaned")

    handle = ctx.effect(lambda: undo)
    caller = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert not handle.disposed

    release.set()
    await ctx.fiber.dispose()

    assert log == ["cleaned"]
    assert handle.disposed


async def test_a_parent_waits_for_a_child_whose_disposal_was_cancelled(ctx: Context) -> None:
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    def child(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            started.set()
            await release.wait()
            log.append("child cleaned")

        return undo

    def parent(inner: Context, config: object) -> None:
        inner.effect(lambda: lambda: log.append("parent resource released"))
        inner.plugin(Plugin("child", child))

    parent_fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()
    child_fiber = ctx.fibers()[1]

    caller = asyncio.ensure_future(child_fiber.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert child_fiber.state is FiberState.UNLOADING

    disposal = asyncio.ensure_future(parent_fiber.dispose())
    await a_few_turns()
    assert log == []

    release.set()
    await disposal

    assert log == ["child cleaned", "parent resource released"]
    assert child_fiber.state is FiberState.DISPOSED


async def test_a_cancellation_raised_by_apply_is_a_failed_load(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    async def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        raise asyncio.CancelledError

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("cancelled", apply))
        with pytest.raises(KernelError, match="cancelled while loading"):
            await fiber

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, KernelError)
    ctx.emit(PING, "after")
    assert log == []


@pytest.mark.parametrize("kind", ["plain function", "coroutine function"])
async def test_a_cancellation_raised_by_a_disposer_does_not_stop_the_unload(
    kind: str, caplog: pytest.LogCaptureFixture
) -> None:
    ctx = Context()
    log: list[str] = []

    def cancelled() -> None:
        raise asyncio.CancelledError

    async def cancelled_later() -> None:
        await asyncio.sleep(0)
        raise asyncio.CancelledError

    ctx.effect(lambda: lambda: log.append("older"))
    ctx.effect(lambda: cancelled if kind == "plain function" else cancelled_later)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["older"]
    assert ctx.fiber.state is FiberState.DISPOSED
    assert "cancelled" in caplog.text


async def test_an_unload_whose_driver_was_cancelled_is_resumed_by_dispose() -> None:
    ctx = Context()
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def slow() -> None:
        started.set()
        await release.wait()
        log.append("slow")

    ctx.effect(lambda: lambda: log.append("older"))
    ctx.effect(lambda: slow)
    disposal = asyncio.ensure_future(ctx.fiber.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)

    driver_of(ctx.fiber).cancel()
    await a_few_turns()
    assert not disposal.done()

    release.set()
    await asyncio.wait_for(disposal, timeout=1)

    assert log == ["slow", "older"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_load_whose_driver_was_cancelled_is_rolled_back(ctx: Context) -> None:
    entered = asyncio.Event()
    log: list[str] = []

    async def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        inner.effect(lambda: lambda: log.append("rolled back"))
        entered.set()
        await asyncio.Event().wait()

    fiber = ctx.plugin(Plugin("slow", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)

    driver_of(fiber).cancel()
    await a_few_turns()

    assert fiber.state is FiberState.FAILED
    assert log == ["rolled back"]
    ctx.emit(PING, "after")
    assert log == ["rolled back"]
    with pytest.raises(KernelError, match="cancelled while loading"):
        await fiber


async def test_a_driver_cancelled_before_it_ran_is_replaced_for_the_next_waiter(
    ctx: Context,
) -> None:
    fiber = ctx.plugin(Plugin("unlucky", nothing))
    driver_of(fiber).cancel()

    assert fiber.state is FiberState.PENDING
    await asyncio.wait_for(fiber.settled(), timeout=1)

    # Whoever waits next starts the load again.
    assert fiber.state is FiberState.ACTIVE


async def test_a_disposer_may_return_a_future() -> None:
    ctx = Context()
    log: list[str] = []

    async def cleanup() -> None:
        await asyncio.sleep(0)
        log.append("cleaned")

    def undo() -> asyncio.Future[None]:
        return asyncio.ensure_future(cleanup())

    ctx.effect(lambda: undo)
    await ctx.fiber.dispose()

    assert log == ["cleaned"]


async def test_parallel_isolates_a_listener_that_is_cancelled(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    finished: list[str] = []

    async def cancelled(payload: str) -> None:
        future: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        future.cancel()
        await future

    async def slow(payload: str) -> None:
        await a_few_turns()
        finished.append(payload)

    ctx.on(FLUSH, cancelled)
    ctx.on(FLUSH, slow)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await ctx.parallel(FLUSH, "x")

    assert finished == ["x"]
    assert "was cancelled" in caplog.text


async def test_cancelling_a_parallel_dispatch_cancels_its_listeners(ctx: Context) -> None:
    started = asyncio.Event()
    cancelled: list[str] = []

    async def listener(payload: str) -> None:
        started.set()
        try:
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            cancelled.append(payload)
            raise

    ctx.on(FLUSH, listener)
    dispatch = asyncio.ensure_future(ctx.parallel(FLUSH, "x"))
    await asyncio.wait_for(started.wait(), timeout=1)

    dispatch.cancel()
    with pytest.raises(asyncio.CancelledError):
        await dispatch

    assert cancelled == ["x"]


async def test_a_withdrawal_outlasts_a_dependent_whose_driver_was_cancelled(ctx: Context) -> None:
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)
        inner.effect(lambda: lambda: log.append("older"))

        async def newer() -> None:
            started.set()
            await release.wait()
            log.append("newer")

        inner.effect(lambda: newer)

    handle = ctx.provide(CLOCK, 1)
    fiber = await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    withdrawal = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    driver_of(fiber).cancel()
    await a_few_turns()
    try:
        assert not withdrawal.done()
    finally:
        release.set()
    await asyncio.wait_for(withdrawal, timeout=1)

    assert log == ["newer", "older"]
    assert fiber.state is FiberState.PENDING


async def test_a_rollback_survives_a_cleanup_that_cancels_itself(ctx: Context) -> None:
    started = asyncio.Event()
    log: list[str] = []

    async def cancelled_cleanup() -> None:
        raise asyncio.CancelledError

    async def apply(inner: Context, config: object) -> None:
        inner.effect(lambda: lambda: log.append("older"))
        inner.effect(lambda: cancelled_cleanup)
        started.set()
        await asyncio.Event().wait()

    fiber = ctx.plugin(Plugin("slow", apply))
    await asyncio.wait_for(started.wait(), timeout=1)

    driver_of(fiber).cancel()
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert log == ["older"]
    assert fiber.state is FiberState.FAILED


async def test_a_cleanup_cancelled_before_it_ran_is_run_for_the_next_waiter(ctx: Context) -> None:
    release = asyncio.Event()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

        async def undo() -> None:
            await release.wait()
            log.append("dependent cleaned")

        inner.effect(lambda: undo)

    handle = ctx.provide(CLOCK, 1)
    await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    def cancel_the_release_before_it_starts() -> None:
        name = f"tack-effect-{ctx.fiber.uid}"
        next(task for task in asyncio.all_tasks() if task.get_name() == name).cancel()

    caller = asyncio.ensure_future(handle.dispose())
    asyncio.get_running_loop().call_soon(cancel_the_release_before_it_starts)
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert not handle.disposed

    release.set()
    await asyncio.wait_for(handle.dispose(), timeout=1)

    assert handle.disposed
    assert log == ["dependent cleaned"]
    ctx.provide(CLOCK, 2)


async def test_a_cleanup_cancelled_after_it_started_is_reported(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()
    started = asyncio.Event()

    async def undo() -> None:
        started.set()
        await asyncio.Event().wait()

    handle = ctx.effect(lambda: undo)
    waiter = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)

    name = f"tack-effect-{ctx.fiber.uid}"
    next(task for task in asyncio.all_tasks() if task.get_name() == name).cancel()
    with (
        caplog.at_level(logging.ERROR, logger="tack.kernel"),
        pytest.raises(asyncio.CancelledError),
    ):
        await waiter

    assert "was cancelled before it finished" in caplog.text
    assert handle.disposed
    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)
```

新建 `tests/kernel/test_eager_tasks.py`：

```python
"""The kernel must behave the same when the event loop starts tasks eagerly."""

import asyncio
import sys
from collections.abc import AsyncIterator, Awaitable

import pytest

from tack.kernel import Context, Disposer, EffectHandle, FiberState, Plugin

pytestmark = pytest.mark.skipif(
    sys.version_info < (3, 12), reason="eager task factories arrived in Python 3.12"
)


@pytest.fixture
async def eager_tasks() -> AsyncIterator[None]:
    """Make the running loop start every new task before create_task returns."""
    loop = asyncio.get_running_loop()
    loop.set_task_factory(getattr(asyncio, "eager_task_factory"))  # noqa: B009
    yield
    loop.set_task_factory(None)


@pytest.mark.usefixtures("eager_tasks")
async def test_cleanup_stays_in_reverse_order_under_an_eager_task_factory() -> None:
    ctx = Context()
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("older"))

        async def undo() -> None:
            log.append("newer starts")
            await inner.fiber.dispose()
            log.append("newer ends")

        return undo

    fiber = await ctx.plugin(Plugin("demo", apply))
    await fiber.dispose()

    assert log == ["newer starts", "newer ends", "older"]
    assert fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


@pytest.mark.usefixtures("eager_tasks")
async def test_a_plugin_loads_in_the_background_under_an_eager_task_factory() -> None:
    ctx = Context()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        log.append("applied")

    fiber = ctx.plugin(Plugin("demo", apply))
    assert log == []

    await fiber

    assert log == ["applied"]
    await ctx.fiber.dispose()


@pytest.mark.usefixtures("eager_tasks")
async def test_a_disposer_that_disposes_its_own_handle_in_an_eager_task_is_still_waited_for() -> (
    None
):
    ctx = Context()
    gate = asyncio.Event()
    log: list[str] = []
    handles: list[EffectHandle] = []
    spawned: list[asyncio.Future[None]] = []

    async def cleanup() -> None:
        await gate.wait()
        log.append("cleaned up")

    def undo() -> Awaitable[None]:
        spawned.append(asyncio.ensure_future(handles[0].dispose()))
        return cleanup()

    handles.append(ctx.effect(lambda: undo))
    waiter = asyncio.ensure_future(handles[0].dispose())
    await asyncio.sleep(0)
    try:
        assert not waiter.done()
        assert not handles[0].disposed
    finally:
        gate.set()
    await asyncio.wait_for(waiter, timeout=1)

    assert log == ["cleaned up"]
    assert handles[0].disposed
    await ctx.fiber.dispose()
```

新建 `tests/kernel/test_fiber_states.py`：

```python
"""The states a fiber passes through, as reported by FIBER_STATE_CHANGED."""

import asyncio
import logging

import pytest

from tack.kernel import (
    FIBER_STATE_CHANGED,
    Context,
    Disposer,
    EmitEvent,
    FiberState,
    FiberStateChange,
    Plugin,
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")
PING = EmitEvent[str]("test/ping")

PENDING = FiberState.PENDING
LOADING = FiberState.LOADING
ACTIVE = FiberState.ACTIVE
FAILED = FiberState.FAILED
UNLOADING = FiberState.UNLOADING
DISPOSED = FiberState.DISPOSED


def states_of(ctx: Context, name: str) -> list[FiberState]:
    """Record every state the fiber called ``name`` enters from now on."""
    seen: list[FiberState] = []

    def listener(change: FiberStateChange) -> None:
        if change.fiber.name == name:
            seen.append(change.current)

    ctx.on(FIBER_STATE_CHANGED, listener)
    return seen


def nothing(ctx: Context, config: object) -> None:
    return None


async def test_a_plugin_reports_each_state_it_passes_through(ctx: Context) -> None:
    seen = states_of(ctx, "demo")

    fiber = await ctx.plugin(Plugin("demo", nothing))
    await fiber.dispose()

    assert seen == [LOADING, ACTIVE, UNLOADING, DISPOSED]


async def test_a_change_carries_the_fiber_and_both_states(ctx: Context) -> None:
    changes: list[FiberStateChange] = []
    ctx.on(FIBER_STATE_CHANGED, changes.append)

    fiber = await ctx.plugin(Plugin("demo", nothing))

    assert [(change.fiber, change.previous, change.current) for change in changes] == [
        (fiber, PENDING, LOADING),
        (fiber, LOADING, ACTIVE),
    ]


async def test_a_failed_load_is_unwound_before_it_is_reported_failed(ctx: Context) -> None:
    seen = states_of(ctx, "broken")

    def apply(inner: Context, config: object) -> None:
        raise RuntimeError("cannot start")

    fiber = ctx.plugin(Plugin("broken", apply))
    await fiber.settled()

    assert seen == [LOADING, UNLOADING, FAILED]


async def test_a_dependent_returns_to_pending_and_loads_again(ctx: Context) -> None:
    seen = states_of(ctx, "dependent")
    handle = ctx.provide(CLOCK, 1)
    await ctx.plugin(Plugin("dependent", nothing, inject=(CLOCK,)))

    await handle.dispose()
    ctx.provide(CLOCK, 2)
    await ctx.settled()

    assert seen == [LOADING, ACTIVE, UNLOADING, PENDING, LOADING, ACTIVE]


async def test_a_plugin_disposed_while_loading_never_becomes_active(ctx: Context) -> None:
    seen = states_of(ctx, "demo")
    entered = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def apply(inner: Context, config: object) -> Disposer:
        entered.set()
        await release.wait()
        return lambda: log.append("undone")

    fiber = ctx.plugin(Plugin("demo", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)
    disposal = asyncio.ensure_future(fiber.dispose())
    await asyncio.sleep(0)

    release.set()
    await disposal

    assert log == ["undone"]
    assert seen == [LOADING, UNLOADING, DISPOSED]


async def test_scopes_and_the_root_report_their_disposal() -> None:
    ctx = Context()
    changes: list[tuple[str, FiberState]] = []
    ctx.on(FIBER_STATE_CHANGED, lambda change: changes.append((change.fiber.name, change.current)))
    scoped = ctx.scope(object())

    await scoped.fiber.dispose()
    await ctx.fiber.dispose()

    # The listener belongs to the root, so it is gone before the root is disposed.
    assert changes == [("scope", UNLOADING), ("scope", DISPOSED), ("root", UNLOADING)]


async def test_a_failing_state_listener_does_not_disturb_the_lifecycle(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    def failing(change: FiberStateChange) -> None:
        raise RuntimeError("observer broke")

    ctx.on(FIBER_STATE_CHANGED, failing)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = await ctx.plugin(Plugin("demo", nothing))

    assert fiber.state is ACTIVE
    assert "observer broke" in caplog.text


async def test_an_observer_that_raises_cancelled_error_cannot_corrupt_a_load(
    ctx: Context,
) -> None:
    log: list[str] = []

    def observe(change: FiberStateChange) -> None:
        if change.fiber.name == "victim" and change.current is LOADING:
            inner = change.fiber.ctx
            inner.effect(lambda: lambda: log.append("cleaned"))
            inner.on(PING, log.append)
            raise asyncio.CancelledError

    ctx.on(FIBER_STATE_CHANGED, observe)

    fiber = await ctx.plugin(Plugin("victim", nothing))
    assert fiber.state is ACTIVE

    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["cleaned"]
    assert fiber.state is DISPOSED
```

新建 `tests/kernel/test_reentrancy.py`：

```python
"""Lifecycle code that waits for its own fiber must not wait for itself."""

import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    FiberState,
    LifecycleError,
    Plugin,
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")


async def test_a_plugin_can_dispose_itself_from_its_own_disposer(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await inner.fiber.dispose()
            log.append("undone")

        return undo

    fiber = await ctx.plugin(Plugin("self", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert fiber.state is FiberState.DISPOSED


async def test_a_plugin_can_dispose_itself_while_another_caller_disposes_it(
    ctx: Context,
) -> None:
    entered = asyncio.Event()
    release = asyncio.Event()

    async def apply(inner: Context, config: object) -> None:
        entered.set()
        await release.wait()
        await inner.fiber.dispose()

    fiber = ctx.plugin(Plugin("self", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)
    outside = asyncio.ensure_future(fiber.dispose())
    await asyncio.sleep(0)

    release.set()
    await asyncio.wait_for(outside, timeout=1)

    assert fiber.state is FiberState.DISPOSED


async def test_the_root_can_be_disposed_from_one_of_its_own_disposers() -> None:
    ctx = Context()
    log: list[str] = []

    async def undo() -> None:
        await ctx.fiber.dispose()
        log.append("undone")

    ctx.effect(lambda: undo)

    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_child_can_dispose_its_parent_from_its_own_disposer(ctx: Context) -> None:
    log: list[str] = []

    def child(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            parent_fiber = inner.fiber.parent
            assert parent_fiber is not None
            await parent_fiber.dispose()
            log.append("child undone")

        return undo

    def parent(inner: Context, config: object) -> None:
        inner.plugin(Plugin("child", child))

    fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert log == ["child undone"]
    assert ctx.fibers() == ()


async def test_a_handle_can_be_disposed_from_inside_its_own_disposer(ctx: Context) -> None:
    handles: list[EffectHandle] = []
    log: list[str] = []

    async def undo() -> None:
        await handles[0].dispose()
        log.append("undone")

    handles.append(ctx.effect(lambda: undo))

    await asyncio.wait_for(handles[0].dispose(), timeout=1)

    assert log == ["undone"]
    assert handles[0].disposed


async def test_awaiting_a_fiber_from_its_own_apply_does_not_wait(ctx: Context) -> None:
    states: list[FiberState] = []

    async def apply(inner: Context, config: object) -> None:
        await inner.fiber
        await inner.fiber.settled()
        states.append(inner.fiber.state)

    fiber = ctx.plugin(Plugin("self", apply))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert states == [FiberState.LOADING]
    assert fiber.state is FiberState.ACTIVE


async def test_the_application_wide_wait_is_refused_inside_a_plugin() -> None:
    # The root is disposed by hand: were the two plugins to wait for each
    # other, disposing them could never finish either.
    ctx = Context()

    async def child(inner: Context, config: object) -> None:
        await inner.settled()

    async def parent(inner: Context, config: object) -> None:
        await inner.plugin(Plugin("child", child))

    fiber = ctx.plugin(Plugin("parent", parent))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, LifecycleError)
    with pytest.raises(LifecycleError, match="settled"):
        await fiber
    await ctx.fiber.dispose()


async def test_the_application_wide_wait_is_refused_inside_a_disposer(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()

    def apply(inner: Context, config: object) -> Disposer:
        return inner.settled

    fiber = await ctx.plugin(Plugin("waiter", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert fiber.state is FiberState.DISPOSED
    assert "LifecycleError" in caplog.text
    await ctx.fiber.dispose()


async def test_a_disposer_may_dispose_its_fiber_through_a_task() -> None:
    ctx = Context()

    def apply(inner: Context, config: object) -> Disposer:
        def undo() -> asyncio.Future[None]:
            return asyncio.ensure_future(inner.fiber.dispose())

        return undo

    fiber = await ctx.plugin(Plugin("self", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


async def test_the_application_wide_wait_is_refused_in_a_task_spawned_by_apply() -> None:
    ctx = Context()

    async def apply(inner: Context, config: object) -> None:
        await asyncio.gather(inner.settled())

    fiber = ctx.plugin(Plugin("waiter", apply))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, LifecycleError)
    await ctx.fiber.dispose()


async def test_a_task_that_outlives_apply_may_wait_for_the_application(ctx: Context) -> None:
    go = asyncio.Event()
    outcomes: list[str] = []
    tasks: list[asyncio.Future[None]] = []

    async def background(inner: Context) -> None:
        await go.wait()
        await inner.settled()
        outcomes.append("waited")

    def apply(inner: Context, config: object) -> None:
        tasks.append(asyncio.ensure_future(background(inner)))

    await ctx.plugin(Plugin("spawner", apply))

    go.set()
    await asyncio.wait_for(tasks[0], timeout=1)

    assert outcomes == ["waited"]


async def test_a_dependent_may_dispose_its_provider_from_its_own_disposer() -> None:
    ctx = Context()
    log: list[str] = []

    def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, 1)

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    def dependent(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await provider_fiber.dispose()
            log.append("dependent undone")

        return undo

    dependent_fiber = await ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))

    await asyncio.wait_for(provider_fiber.dispose(), timeout=1)

    assert log == ["dependent undone"]
    assert provider_fiber.state is FiberState.DISPOSED
    assert dependent_fiber.state is FiberState.PENDING
    await ctx.fiber.dispose()


async def test_disposing_a_dependent_that_disposes_its_provider_does_not_hang() -> None:
    ctx = Context()

    def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, 1)

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    def dependent(inner: Context, config: object) -> Disposer:
        return provider_fiber.dispose

    dependent_fiber = await ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))

    await asyncio.wait_for(dependent_fiber.dispose(), timeout=1)
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert dependent_fiber.state is FiberState.DISPOSED
    assert provider_fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


async def a_few_turns() -> None:
    for _ in range(5):
        await asyncio.sleep(0)


async def test_a_disposer_that_disposes_its_fiber_and_carries_on_does_not_stall_the_loop() -> None:
    ctx = Context()
    log: list[str] = []

    async def undo() -> None:
        await ctx.fiber.dispose()
        await asyncio.sleep(0)
        log.append("undone")

    handle = ctx.effect(lambda: undo)

    await asyncio.wait_for(handle.dispose(), timeout=1)
    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_provider_waits_for_a_dependent_that_disposed_it_while_loading() -> None:
    ctx = Context()
    entered = asyncio.Event()
    gate = asyncio.Event()
    log: list[str] = []

    def provider(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, 1)
        return lambda: log.append("provider cleaned up")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    async def dependent(inner: Context, config: object) -> Disposer:
        inner.use(CLOCK)
        await provider_fiber.dispose()
        entered.set()
        await gate.wait()
        return lambda: log.append("dependent cleaned up")

    ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))
    await asyncio.wait_for(entered.wait(), timeout=1)
    await a_few_turns()
    try:
        assert log == []
    finally:
        gate.set()
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert log == ["dependent cleaned up", "provider cleaned up"]
    await ctx.fiber.dispose()


async def test_a_task_started_by_a_disposer_waits_normally_once_the_disposer_has_returned() -> None:
    ctx = Context()
    release = asyncio.Event()
    log: list[str] = []
    tasks: list[asyncio.Future[None]] = []

    async def older() -> None:
        await release.wait()
        log.append("older")

    async def after() -> None:
        await ctx.fiber.dispose()
        log.append(f"after returned while {ctx.fiber.state.value}")

    async def undo() -> None:
        tasks.append(asyncio.ensure_future(after()))

    ctx.effect(lambda: older)
    handle = ctx.effect(lambda: undo)

    await handle.dispose()
    await a_few_turns()
    try:
        assert log == []
    finally:
        release.set()
    await asyncio.wait_for(tasks[0], timeout=1)

    assert log == ["older", "after returned while disposed"]


async def test_a_dependent_may_withdraw_the_service_it_injects_from_its_own_disposer() -> None:
    ctx = Context()
    log: list[str] = []
    handle = ctx.provide(CLOCK, 1)

    def apply(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await handle.dispose()
            log.append("undone")

        return undo

    fiber = await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    await asyncio.wait_for(fiber.dispose(), timeout=1)
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert log == ["undone"]
    assert fiber.state is FiberState.DISPOSED
    assert ctx.get(CLOCK) is None
    await asyncio.wait_for(handle.dispose(), timeout=1)
    assert handle.disposed
    await ctx.fiber.dispose()


async def test_a_task_started_in_apply_and_returned_as_cleanup_may_dispose_its_fiber() -> None:
    ctx = Context()
    go = asyncio.Event()
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        async def cleanup() -> None:
            await go.wait()
            await inner.fiber.dispose()
            log.append("cleaned up")

        task = asyncio.ensure_future(cleanup())

        def undo() -> asyncio.Future[None]:
            go.set()
            return task

        return undo

    fiber = await ctx.plugin(Plugin("self", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert log == ["cleaned up"]
    assert fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


async def test_a_task_started_by_a_finished_cleanup_task_is_ordinary_code(ctx: Context) -> None:
    outcomes: list[str] = []
    tasks: list[asyncio.Future[None]] = []

    async def after() -> None:
        await ctx.settled()
        outcomes.append("waited")

    async def cleanup() -> None:
        await asyncio.sleep(0)
        tasks.append(asyncio.ensure_future(after()))

    def undo() -> asyncio.Future[None]:
        return asyncio.ensure_future(cleanup())

    handle = ctx.effect(lambda: undo)

    await asyncio.wait_for(handle.dispose(), timeout=1)
    await asyncio.wait_for(tasks[0], timeout=1)

    assert outcomes == ["waited"]
```

新建 `tests/kernel/test_withdrawal.py`：

```python
"""A service that is being withdrawn while its dependents are still cleaning up."""

import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")
STORE = ServiceKey[int]("store")


def nothing(ctx: Context, config: object) -> None:
    return None


async def a_few_turns() -> None:
    for _ in range(5):
        await asyncio.sleep(0)


class SlowDependent:
    """A plugin that injects the clock and whose cleanup waits to be released."""

    def __init__(self, log: list[str]) -> None:
        self.log = log
        self.cleaning = asyncio.Event()
        self.release = asyncio.Event()
        self.plugin = Plugin("dependent", self.apply, inject=(CLOCK,))

    def apply(self, ctx: Context, config: object) -> Disposer:
        now = ctx.use(CLOCK)
        self.log.append(f"dependent loads against {now}")

        async def undo() -> None:
            self.log.append("dependent cleanup starts")
            self.cleaning.set()
            await self.release.wait()
            self.log.append("dependent cleanup ends")

        return undo


async def test_a_provider_waits_for_dependents_of_a_service_it_already_withdrew(
    ctx: Context,
) -> None:
    log: list[str] = []
    handles: list[EffectHandle] = []
    dependent = SlowDependent(log)

    def provider(inner: Context, config: object) -> Disposer:
        handles.append(inner.provide(CLOCK, 1))
        return lambda: log.append("provider resource closed")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))
    await ctx.plugin(dependent.plugin)

    withdrawal = asyncio.ensure_future(handles[0].dispose())
    await asyncio.wait_for(dependent.cleaning.wait(), timeout=1)
    disposal = asyncio.ensure_future(provider_fiber.dispose())
    await a_few_turns()
    try:
        assert log == ["dependent loads against 1", "dependent cleanup starts"]
    finally:
        dependent.release.set()
    await asyncio.wait_for(asyncio.gather(withdrawal, disposal), timeout=1)

    assert log == [
        "dependent loads against 1",
        "dependent cleanup starts",
        "dependent cleanup ends",
        "provider resource closed",
    ]


async def test_a_name_stays_taken_until_its_dependents_have_unloaded(ctx: Context) -> None:
    log: list[str] = []
    dependent = SlowDependent(log)
    handle = ctx.provide(CLOCK, 1)
    fiber = await ctx.plugin(dependent.plugin)

    withdrawal = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(dependent.cleaning.wait(), timeout=1)

    try:
        assert ctx.get(CLOCK) is None
        with pytest.raises(ServiceConflictError, match="still being withdrawn by 'root'"):
            ctx.provide(CLOCK, 2)
    finally:
        dependent.release.set()
    await asyncio.wait_for(withdrawal, timeout=1)
    assert fiber.state is FiberState.PENDING

    ctx.provide(CLOCK, 2)
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == [
        "dependent loads against 1",
        "dependent cleanup starts",
        "dependent cleanup ends",
        "dependent loads against 2",
    ]


async def test_a_withdrawal_cannot_be_overtaken_by_a_dependent_providing_the_name(
    ctx: Context,
) -> None:
    """Two plugins that would wait for each other if the name could change hands early."""
    old = ctx.provide(CLOCK, 1)

    def provide_store(inner: Context, config: object) -> None:
        inner.provide(STORE, inner.use(CLOCK))

    first = await ctx.plugin(Plugin("first", provide_store, inject=(CLOCK,)))
    second = await ctx.plugin(Plugin("second", nothing, inject=(STORE,)))

    withdrawal = asyncio.ensure_future(old.dispose())
    await asyncio.sleep(0)
    with pytest.raises(ServiceConflictError, match="still being withdrawn"):
        second.ctx.provide(CLOCK, 2)

    await asyncio.wait_for(withdrawal, timeout=1)

    assert first.state is FiberState.PENDING
    assert second.state is FiberState.PENDING
    assert ctx.get(CLOCK) is None
```

修改 `tests/kernel/test_dependencies.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/tests/kernel/test_dependencies.py
+++ b/tests/kernel/test_dependencies.py
@@ -25,7 +25,7 @@ STORE = ServiceKey[dict[str, int]]("store")
 INDEX = ServiceKey[list[str]]("index")
 
 
-def consumer(log: list[str]) -> Plugin:
+def consumer(log: list[str]) -> Plugin[object]:
     """A plugin that needs the clock and records each load and unload."""
 
     def apply(ctx: Context, config: object) -> Disposer:
@@ -36,7 +36,7 @@ def consumer(log: list[str]) -> Plugin:
     return Plugin("consumer", apply, inject=(CLOCK,))
 
 
-def clock_provider(now: int, log: list[str] | None = None) -> Plugin:
+def clock_provider(now: int, log: list[str] | None = None) -> Plugin[object]:
     def apply(ctx: Context, config: object) -> Disposer:
         ctx.provide(CLOCK, Clock(now))
         return lambda: None if log is None else log.append(f"provider {now} cleanup")
@@ -323,45 +323,21 @@ async def test_inject_is_read_from_a_module_like_plugin(ctx: Context) -> None:
 
 
 @pytest.mark.parametrize("inject", ["clock", [CLOCK, "store"], {CLOCK}])
-def test_inject_must_be_a_list_of_service_keys(inject: object) -> None:
+def test_inject_must_be_a_list_or_tuple_of_service_keys(inject: object) -> None:
     def apply(inner: Context, config: object) -> None:
         return None
 
-    with pytest.raises(InvalidPluginError, match="list of service keys"):
+    with pytest.raises(InvalidPluginError, match="list or tuple of service keys"):
         resolve_plugin(SimpleNamespace(name="module", apply=apply, inject=inject))
 
+    with pytest.raises(InvalidPluginError, match="list or tuple of service keys"):
+        Plugin("direct", apply, inject=inject)  # pyright: ignore[reportArgumentType]
 
-async def test_a_provider_below_its_dependent_does_not_deadlock(ctx: Context) -> None:
-    """An unloading plugin must not wait for itself through a service of its child."""
-    gate = asyncio.Event()
-    unloading = asyncio.Event()
-    child_contexts: list[Context] = []
 
-    def child(inner: Context, config: object) -> None:
-        child_contexts.append(inner)
-
-    async def parent(inner: Context, config: object) -> None:
-        await inner.plugin(Plugin("child", child))
-
-        async def slow() -> None:
-            unloading.set()
-            await gate.wait()
-
-        inner.effect(lambda: slow)
-
-    handle = ctx.provide(CLOCK, Clock(1))
-    parent_fiber = await ctx.plugin(Plugin("parent", parent, inject=(CLOCK,)))
-    child_fiber = child_contexts[0].fiber
-
-    withdrawal = asyncio.ensure_future(handle.dispose())
-    await asyncio.wait_for(unloading.wait(), timeout=1)
-    assert parent_fiber.state is FiberState.UNLOADING
-    child_contexts[0].provide(CLOCK, Clock(2))
+def test_inject_given_as_a_list_is_kept_as_a_tuple() -> None:
+    def apply(inner: Context, config: object) -> None:
+        return None
 
-    gate.set()
-    await asyncio.wait_for(withdrawal, timeout=1)
-    await ctx.settled()
+    plugin = Plugin("direct", apply, inject=[CLOCK])  # pyright: ignore[reportArgumentType]
 
-    assert child_fiber.state is FiberState.DISPOSED
-    assert parent_fiber.state is FiberState.PENDING
-    assert ctx.get(CLOCK) is None
+    assert plugin.inject == (CLOCK,)
```

修改 `tests/kernel/test_events_sync.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/tests/kernel/test_events_sync.py
+++ b/tests/kernel/test_events_sync.py
@@ -1,3 +1,4 @@
+import asyncio
 import logging
 
 import pytest
@@ -150,24 +151,64 @@ async def test_one_name_cannot_be_declared_as_two_kinds(ctx: Context) -> None:
         ctx.bail(BailEvent[str, int]("test/twice"), "x")
 
 
-async def test_emit_events_reject_coroutine_functions_as_listeners(ctx: Context) -> None:
-    async def listener(payload: str) -> None:
-        return None
+async def a_coroutine(payload: str) -> int:
+    return len(payload)
+
+
+class AsyncCallable:
+    async def __call__(self, payload: str) -> int:
+        return len(payload)
 
+
+@pytest.mark.parametrize("listener", [a_coroutine, AsyncCallable()], ids=["function", "object"])
+async def test_events_that_do_not_await_reject_async_listeners(
+    ctx: Context, listener: object
+) -> None:
     with pytest.raises(EventModeError, match="plain functions"):
         ctx.on(PING, listener)  # pyright: ignore[reportArgumentType, reportCallIssue]
 
+    with pytest.raises(EventModeError, match="plain functions"):
+        ctx.on(ASK, listener)  # pyright: ignore[reportArgumentType, reportCallIssue]
+
 
-async def test_emit_closes_a_coroutine_returned_by_a_listener(
+async def test_emit_closes_a_coroutine_returned_by_a_plain_listener(
     ctx: Context, caplog: pytest.LogCaptureFixture
 ) -> None:
-    class AsyncCallable:
-        async def __call__(self, payload: str) -> None:
-            return None
+    def sneaky(payload: str) -> None:
+        return a_coroutine(payload)  # pyright: ignore[reportReturnType]
 
-    ctx.on(PING, AsyncCallable())  # pyright: ignore[reportArgumentType, reportCallIssue]
+    ctx.on(PING, sneaky)
 
     with caplog.at_level(logging.ERROR, logger="tack.kernel"):
         ctx.emit(PING, "x")
 
     assert "returned a coroutine" in caplog.text
+
+
+async def test_bail_refuses_a_coroutine_as_an_answer(ctx: Context) -> None:
+    def sneaky(payload: str) -> int | None:
+        return a_coroutine(payload)  # pyright: ignore[reportReturnType]
+
+    ctx.on(ASK, sneaky)
+    ctx.on(ASK, lambda payload: 7)
+
+    with pytest.raises(EventModeError, match="returned a coroutine"):
+        ctx.bail(ASK, "four")
+
+
+async def test_emit_isolates_a_listener_that_raises_cancelled_error(
+    ctx: Context, caplog: pytest.LogCaptureFixture
+) -> None:
+    log: list[str] = []
+
+    def cancelled(payload: str) -> None:
+        raise asyncio.CancelledError
+
+    ctx.on(PING, cancelled)
+    ctx.on(PING, log.append)
+
+    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
+        ctx.emit(PING, "x")
+
+    assert log == ["x"]
+    assert "test/ping" in caplog.text
```

用下面的内容替换 `tests/kernel/test_lifecycle_properties.py` 的全部内容：

```python
"""Property test: random plugin graphs under random loads, disposals and withdrawals.

Each generated scenario declares a few services and a few plugins. Every
service has one owner: the root context, one of the plugins, or nobody. Every
plugin injects some of the services, and may be slow to load or to clean up. A
scenario then plays a random sequence of steps against a real kernel and
checks, each time the kernel comes to rest, that the kernel is exactly where
the declarations say it should be.
"""

import asyncio
from collections import Counter
from dataclasses import dataclass

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    Fiber,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
)

ROOT = -1
NOBODY = -2


class Token:
    """Stands in for one provided service instance."""


@dataclass(frozen=True)
class Scenario:
    owners: tuple[int, ...]
    """The owner of each service: a plugin index, ``ROOT`` or ``NOBODY``."""
    injects: tuple[frozenset[int], ...]
    """The services each plugin injects."""
    slow: tuple[tuple[bool, bool], ...]
    """Whether each plugin yields to the event loop while loading and while cleaning up."""
    steps: tuple[tuple[str, int], ...]


PLUGIN_ACTIONS = ("add", "dispose", "dispose_later")
# Loads and provisions are listed more often than their opposites, so that most
# scenarios spend their steps on a populated graph rather than an empty one.
ACTIONS = (
    *("add",) * 5,
    *("provide",) * 3,
    "withdraw",
    "withdraw_later",
    "dispose",
    "dispose_later",
    *("yield",) * 3,
    "settle",
)


async def linger() -> None:
    """Stay unfinished for a few turns of the event loop."""
    for _ in range(4):
        await asyncio.sleep(0)


@st.composite
def scenarios(draw: st.DrawFn) -> Scenario:
    plugin_count = draw(st.integers(min_value=2, max_value=4))
    service_count = draw(st.integers(min_value=1, max_value=3))
    owner = st.sampled_from([ROOT, ROOT, NOBODY, *range(plugin_count), *range(plugin_count)])
    owners = tuple(draw(owner) for _ in range(service_count))
    services = st.integers(min_value=0, max_value=service_count - 1)
    injects = tuple(frozenset(draw(st.sets(services, max_size=2))) for _ in range(plugin_count))
    slow = tuple(draw(st.tuples(st.booleans(), st.booleans())) for _ in range(plugin_count))
    raw_steps = draw(
        st.lists(
            st.tuples(st.sampled_from(ACTIONS), st.integers(min_value=0, max_value=11)),
            min_size=4,
            max_size=40,
        )
    )
    steps = tuple(
        (action, number % (plugin_count if action in PLUGIN_ACTIONS else service_count))
        for action, number in raw_steps
    )
    return Scenario(owners, injects, slow, steps)


class Run:
    """Plays one scenario against a kernel and tracks what should be true."""

    def __init__(self, scenario: Scenario) -> None:
        self.scenario = scenario
        self.ctx = Context()
        self.keys = [ServiceKey[Token](f"service-{s}") for s in range(len(scenario.owners))]
        self.fibers: dict[int, Fiber] = {}
        self.retired: list[Fiber] = []
        self.handles: dict[int, EffectHandle] = {}
        self.withdrawals: dict[int, asyncio.Future[None]] = {}
        self.tokens: dict[int, Token] = {}
        self.seen: dict[int, dict[int, Token]] = {}
        self.loaded: set[int] = set()
        self.loads: Counter[int] = Counter()
        self.unloads: Counter[int] = Counter()
        self.pending: list[asyncio.Future[None]] = []
        self.violations: list[str] = []

    def dependents_of(self, service: int) -> list[int]:
        return [p for p, injects in enumerate(self.scenario.injects) if service in injects]

    def plugin(self, index: int) -> Plugin[object]:
        provides = [s for s, owner in enumerate(self.scenario.owners) if owner == index]
        injects = sorted(self.scenario.injects[index])
        slow_to_load, slow_to_clean_up = self.scenario.slow[index]

        async def apply(ctx: Context, config: object) -> Disposer:
            self.seen[index] = {s: ctx.use(self.keys[s]) for s in injects}
            self.loaded.add(index)
            self.loads[index] += 1
            if slow_to_load:
                await linger()
            for service in provides:
                token = Token()
                ctx.provide(self.keys[service], token)
                self.tokens[service] = token

            async def undo() -> None:
                # A plugin counts as loaded until its cleanup has finished, so
                # a provider that starts cleaning up during it is caught here.
                for service in provides:
                    for dependent in self.dependents_of(service):
                        if dependent != index and dependent in self.loaded:
                            self.violations.append(
                                f"plugin {index} cleaned up while its dependent "
                                f"{dependent} was still loaded"
                            )
                if slow_to_clean_up:
                    await linger()
                for service in provides:
                    del self.tokens[service]
                self.loaded.discard(index)
                self.unloads[index] += 1

            return undo

        return Plugin(f"plugin-{index}", apply, inject=tuple(self.keys[s] for s in injects))

    async def withdraw(self, service: int, handle: EffectHandle) -> None:
        await handle.dispose()
        del self.tokens[service]
        for dependent in self.dependents_of(service):
            if dependent in self.loaded:
                self.violations.append(
                    f"service {service} was withdrawn while its dependent "
                    f"{dependent} was still loaded"
                )

    def provide(self, service: int) -> None:
        if self.scenario.owners[service] != ROOT or service in self.handles:
            return
        withdrawal = self.withdrawals.get(service)
        if withdrawal is not None and not withdrawal.done():
            with pytest.raises(ServiceConflictError):
                self.ctx.provide(self.keys[service], Token())
            return
        token = Token()
        self.handles[service] = self.ctx.provide(self.keys[service], token)
        self.tokens[service] = token

    async def step(self, action: str, target: int) -> None:
        if action == "add":
            previous = [fiber for fiber in self.retired if fiber.name == f"plugin-{target}"]
            if target not in self.fibers and all(
                fiber.state is FiberState.DISPOSED for fiber in previous
            ):
                self.fibers[target] = self.ctx.plugin(self.plugin(target))
        elif action in ("dispose", "dispose_later"):
            fiber = self.fibers.pop(target, None)
            if fiber is not None:
                self.retired.append(fiber)
                disposal = asyncio.ensure_future(fiber.dispose())
                if action == "dispose":
                    await disposal
                else:
                    self.pending.append(disposal)
        elif action == "provide":
            self.provide(target)
        elif action in ("withdraw", "withdraw_later"):
            handle = self.handles.pop(target, None)
            if handle is not None:
                withdrawal = asyncio.ensure_future(self.withdraw(target, handle))
                self.withdrawals[target] = withdrawal
                if action == "withdraw":
                    await withdrawal
                else:
                    self.pending.append(withdrawal)
        elif action == "yield":
            await asyncio.sleep(0)
        else:
            await self.settle()
            self.check()

    async def settle(self) -> None:
        while self.pending:
            pending, self.pending = self.pending, []
            await asyncio.gather(*pending)
        await self.ctx.settled()

    def expected_active(self) -> set[int]:
        """The least fixed point: who can be active given who is live and what root provides."""
        available = set(self.handles)
        active: set[int] = set()
        progressed = True
        while progressed:
            progressed = False
            for index in set(self.fibers) - active:
                if self.scenario.injects[index] <= available:
                    active.add(index)
                    available |= {
                        s for s, owner in enumerate(self.scenario.owners) if owner == index
                    }
                    progressed = True
        return active

    def check(self) -> None:
        active = self.expected_active()
        for index, fiber in self.fibers.items():
            expected = FiberState.ACTIVE if index in active else FiberState.PENDING
            assert fiber.state is expected, (index, fiber.state, expected)
            assert self.loads[index] - self.unloads[index] == (1 if index in active else 0)
            if index in active:
                for service in self.scenario.injects[index]:
                    assert self.seen[index][service] is self.tokens[service], (index, service)
        assert self.loaded == active
        for service, key in enumerate(self.keys):
            assert self.ctx.get(key) is self.tokens.get(service), service
        assert self.violations == []

    async def finish(self) -> None:
        """Withdraw one root service, then dispose the root, checking at each rest.

        The other root services stay provided, so that disposing the root also
        has dependents to unload.
        """
        await self.settle()
        self.check()
        if self.handles:
            await self.step("withdraw", min(self.handles))
            await self.settle()
            self.check()
        fibers = [*self.fibers.values(), *self.retired]
        await self.ctx.fiber.dispose()
        assert all(fiber.state is FiberState.DISPOSED for fiber in fibers)
        assert self.ctx.fibers() == ()
        assert self.loaded == set()
        assert self.loads == self.unloads
        assert all(self.ctx.get(key) is None for key in self.keys)
        assert self.violations == []


async def play(scenario: Scenario) -> None:
    run = Run(scenario)
    for action, target in scenario.steps:
        await run.step(action, target)
    await run.finish()


@pytest.mark.timeout(120)
@settings(max_examples=300, deadline=None)
@given(scenarios())
def test_plugins_are_active_exactly_when_their_services_are(scenario: Scenario) -> None:
    asyncio.run(play(scenario))
```

修改 `tests/kernel/test_plugins.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/tests/kernel/test_plugins.py
+++ b/tests/kernel/test_plugins.py
@@ -3,7 +3,7 @@ import logging
 from types import SimpleNamespace
 
 import pytest
-from pydantic import BaseModel, ValidationError
+from pydantic import BaseModel, ValidationError, field_validator
 
 from tack.kernel import (
     Context,
@@ -253,29 +253,6 @@ async def test_settled_waits_for_every_plugin(ctx: Context) -> None:
     assert ctx.fibers() == (first, second)
 
 
-async def test_a_plugin_disposed_while_loading_never_becomes_active(ctx: Context) -> None:
-    entered = asyncio.Event()
-    release = asyncio.Event()
-    log: list[str] = []
-
-    async def apply(inner: Context, config: object) -> Disposer:
-        entered.set()
-        await release.wait()
-        return lambda: log.append("undone")
-
-    fiber = ctx.plugin(Plugin("demo", apply))
-    await asyncio.wait_for(entered.wait(), timeout=1)
-    disposal = asyncio.ensure_future(fiber.dispose())
-    await asyncio.sleep(0)
-    assert fiber.state is FiberState.LOADING
-
-    release.set()
-    await disposal
-
-    assert log == ["undone"]
-    assert fiber.state is FiberState.DISPOSED
-
-
 async def test_a_plugin_can_dispose_itself_while_loading(ctx: Context) -> None:
     async def apply(inner: Context, config: object) -> None:
         await inner.fiber.dispose()
@@ -332,3 +309,45 @@ def test_resolve_plugin_reads_the_module_name_when_no_name_is_given() -> None:
 def test_resolve_plugin_rejects_other_objects(candidate: object) -> None:
     with pytest.raises(InvalidPluginError):
         resolve_plugin(candidate)
+
+
+def test_a_plugin_checks_what_it_is_given() -> None:
+    with pytest.raises(InvalidPluginError, match="needs a name"):
+        Plugin("", nothing)
+
+    with pytest.raises(InvalidPluginError, match="must be callable"):
+        Plugin("demo", "not callable")  # pyright: ignore[reportArgumentType]
+
+    with pytest.raises(InvalidPluginError, match="pydantic model"):
+        Plugin("demo", nothing, config=dict)
+
+
+def test_pyright_rejects_a_config_model_that_apply_does_not_accept() -> None:
+    """A static check: the ignore below fails the type check once it is unnecessary."""
+
+    class Other(BaseModel):
+        size: int = 0
+
+    def apply(inner: Context, config: Greeting) -> None:
+        return None
+
+    plugin = Plugin("mismatched", apply, config=Other)  # pyright: ignore[reportArgumentType]
+
+    assert plugin.config is Other
+
+
+async def test_a_config_validator_that_raises_anything_fails_the_fiber(ctx: Context) -> None:
+    class Fussy(BaseModel):
+        number: int
+
+        @field_validator("number", mode="before")
+        @classmethod
+        def reject(cls, value: object) -> object:
+            raise TypeError(f"cannot read {value!r}")
+
+    fiber = ctx.plugin(Plugin("fussy", nothing, config=Fussy), {"number": "many"})
+
+    assert fiber.state is FiberState.FAILED
+    assert isinstance(fiber.error, TypeError)
+    with pytest.raises(TypeError, match="cannot read 'many'"):
+        await fiber
```

修改 `tests/kernel/test_scopes.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/tests/kernel/test_scopes.py
+++ b/tests/kernel/test_scopes.py
@@ -26,6 +26,10 @@ class Agent:
     """Stands in for whatever object identifies a scope."""
 
 
+def nothing(ctx: Context, config: object) -> None:
+    return None
+
+
 async def test_a_scope_is_active_as_soon_as_it_is_created(ctx: Context) -> None:
     agent = Agent()
 
@@ -119,7 +123,7 @@ async def test_disposing_a_scope_undoes_what_was_registered_through_it(ctx: Cont
     scoped.on(PING, log.append)
     scoped.effect(lambda: lambda: log.append("effect undone"))
     scoped.provide(NOTES, ["a note"])
-    fiber = await scoped.plugin(Plugin("inside", lambda inner, config: None))
+    fiber = await scoped.plugin(Plugin("inside", nothing))
     assert ctx.use(NOTES) == ["a note"]
 
     await scoped.fiber.dispose()
```

**Step 2: 运行测试，确认失败**

在未加固的内核上，`test_cancellation.py` 和 `test_withdrawal.py` 里有用例会一直等待，所以这里逐个文件运行并带超时。`test_scopes.py` 只是把匿名函数换成了具名函数，改动前后都通过。`test_eager_tasks.py` 在 Python 3.11 上整个跳过。

逐个文件运行：`uv run pytest <文件> -q --timeout=10`

Expected:

| 文件 | 结果 | 第一个错误 |
|---|---|---|
| `test_cancellation.py` | 有用例在 10 秒内没有结束 | — |
| `test_dependencies.py` | 1 error in 0.61s | TypeError: type 'Plugin' is not subscriptable |
| `test_eager_tasks.py` | 3 skipped in 0.45s | — |
| `test_events_sync.py` | 3 failed, 15 passed, 1 error in 0.70s | RuntimeWarning: coroutine 'a_coroutine' was never awaited |
| `test_fiber_states.py` | 1 error in 0.60s | ImportError: cannot import name 'FIBER_STATE_CHANGED' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py) |
| `test_lifecycle_properties.py` | 1 error in 0.68s | TypeError: type 'Plugin' is not subscriptable |
| `test_plugins.py` | 2 failed, 26 passed in 0.81s | Failed: DID NOT RAISE InvalidPluginError |
| `test_reentrancy.py` | 1 error in 0.60s | ImportError: cannot import name 'LifecycleError' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py) |
| `test_scopes.py` | 10 passed in 0.49s | — |
| `test_withdrawal.py` | 45 秒内没有结果，进程被强制结束 | — |

**Step 3: 写实现**

`fiber.py` 和 `plugin.py` 的改动接近重写，下面给出完整的新内容。其余文件给出 diff。

修改 `src/tack/kernel/errors.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/errors.py
+++ b/src/tack/kernel/errors.py
@@ -9,6 +9,10 @@ class InactiveEffectError(KernelError):
     """An effect was registered on a plugin instance that is not loaded."""
 
 
+class LifecycleError(KernelError):
+    """Code that runs while a plugin loads or unloads asked for something impossible there."""
+
+
 class ServiceConflictError(KernelError):
     """A service was provided while another provider already holds its name."""
 
```

修改 `src/tack/kernel/events.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/events.py
+++ b/src/tack/kernel/events.py
@@ -94,6 +94,13 @@ WaterfallListener: TypeAlias = Callable[[PayloadT, Next[PayloadT, ResultT]], Awa
 _PLAIN_LISTENER_MODES = frozenset({"emit", "bail"})
 
 
+def _is_async(callback: Callable[..., Any]) -> bool:
+    """Return whether calling ``callback`` produces a coroutine."""
+    return inspect.iscoroutinefunction(callback) or inspect.iscoroutinefunction(
+        type(callback).__call__
+    )
+
+
 @dataclass(slots=True, eq=False)
 class Hook:
     """One registered listener, and the scope it was registered in, if any."""
@@ -133,7 +140,7 @@ class HookTable:
                 its listeners.
         """
         self.declare(event)
-        if event.mode in _PLAIN_LISTENER_MODES and inspect.iscoroutinefunction(hook.callback):
+        if event.mode in _PLAIN_LISTENER_MODES and _is_async(hook.callback):
             raise EventModeError(
                 f"{event!r} takes plain functions as listeners, not coroutine functions"
             )
@@ -177,11 +184,15 @@ class HookTable:
 
 
 def run_emit(hooks: Sequence[Hook], event: Event, payload: object) -> None:
-    """Call every listener, logging the ones that fail."""
+    """Call every listener, logging the ones that fail.
+
+    A listener that raises ``CancelledError`` is logged like any other. Nothing
+    is awaited here, so that error cannot be a cancellation of the caller.
+    """
     for hook in hooks:
         try:
             result = hook.callback(payload)
-        except Exception:
+        except (Exception, asyncio.CancelledError):
             logger.exception("a listener of %r failed", event.name)
             continue
         if inspect.iscoroutine(result):
@@ -193,10 +204,20 @@ def run_emit(hooks: Sequence[Hook], event: Event, payload: object) -> None:
             )
 
 
-def run_bail(hooks: Sequence[Hook], payload: object) -> Any:
-    """Return the first answer that is not ``None``."""
+def run_bail(hooks: Sequence[Hook], event: Event, payload: object) -> Any:
+    """Return the first answer that is not ``None``.
+
+    Raises:
+        EventModeError: A listener answered with a coroutine.
+    """
     for hook in hooks:
         result = hook.callback(payload)
+        if inspect.iscoroutine(result):
+            result.close()
+            raise EventModeError(
+                f"a listener of {event.name!r} returned a coroutine; "
+                f"{event.mode} listeners must be plain functions"
+            )
         if result is not None:
             return result
     return None
@@ -213,7 +234,9 @@ async def run_parallel(hooks: Sequence[Hook], event: Event, payload: object) ->
         except Exception:
             logger.exception("a listener of %r failed", event.name)
 
-    await asyncio.gather(*(call(hook) for hook in hooks))
+    outcomes = await asyncio.gather(*(call(hook) for hook in hooks), return_exceptions=True)
+    if any(isinstance(outcome, asyncio.CancelledError) for outcome in outcomes):
+        logger.error("a listener of %r was cancelled", event.name)
 
 
 async def run_serial(hooks: Sequence[Hook], payload: object) -> Any:
```

用下面的内容替换 `src/tack/kernel/fiber.py` 的全部内容：

```python
"""Fibers and the effects they own.

A fiber is the kernel's unit of ownership. Everything registered through a
context is recorded as an effect on the fiber that owns the context, and is
undone when that fiber unloads.

Loads, unloads and asynchronous cleanups run in tasks that belong to the
kernel. Cancelling a caller that waits for one of them never interrupts it. If
one of those tasks is itself cancelled, as ``asyncio.run`` does on exit to
whatever is still running, the fiber keeps a truthful state, and the next
caller that waits for it resumes the interrupted work. One thing cannot be
resumed: a cleanup that had already started when its task was cancelled. It is
reported in the log and counts as done.
"""

# Fibers, effect handles and the shared kernel state reach into one another.
# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
import contextvars
import enum
import inspect
import itertools
import logging
import weakref
from collections.abc import Awaitable, Callable, Coroutine, Generator, Iterator
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeAlias, cast

from tack.kernel.errors import InactiveEffectError, InvalidPluginError, KernelError
from tack.kernel.events import EmitEvent, HookTable, run_emit
from tack.kernel.services import ServiceKey

if TYPE_CHECKING:
    from tack.kernel.context import Context
    from tack.kernel.plugin import Plugin

logger = logging.getLogger("tack.kernel")

Disposer: TypeAlias = Callable[[], Awaitable[None] | None]
"""Undoes one effect. It may be a plain function or a coroutine function."""

Epoch: TypeAlias = tuple[int, ...]
"""Identifies the set of providers a fiber is loaded against."""


def _being_cancelled() -> bool:
    """Return whether the current task itself has been asked to cancel.

    A ``CancelledError`` also surfaces when the awaited work was cancelled.
    This tells the two apart.
    """
    task = asyncio.current_task()
    return task is not None and task.cancelling() > 0


def _spawn(
    coroutine: Coroutine[Any, Any, None], name: str, mark: Lifecycle | None = None
) -> asyncio.Task[None]:
    """Start a task that belongs to the kernel.

    The task runs under ``mark``, or under no lifecycle mark when none is
    given. It must not inherit the mark of whatever code happened to start it:
    a driver started from inside a disposer is not part of that disposer.

    The task is created directly instead of through the loop's task factory,
    so it never starts to run before this function returns. The kernel records
    each task right after creating it and relies on that order.
    """
    context = contextvars.copy_context()
    context.run(_lifecycle.set, mark)
    return asyncio.Task(coroutine, loop=asyncio.get_running_loop(), name=name, context=context)


class FiberState(enum.Enum):
    """Where a fiber is in its lifecycle."""

    PENDING = "pending"
    LOADING = "loading"
    ACTIVE = "active"
    FAILED = "failed"
    UNLOADING = "unloading"
    DISPOSED = "disposed"


@dataclass(frozen=True, slots=True)
class FiberStateChange:
    """The payload of :data:`FIBER_STATE_CHANGED`."""

    fiber: Fiber
    previous: FiberState
    current: FiberState


FIBER_STATE_CHANGED: EmitEvent[FiberStateChange] = EmitEvent("kernel/fiber-state-changed")
"""Emitted, outside every scope, each time a fiber enters a new state."""


@dataclass(slots=True, eq=False)
class Lifecycle:
    """Marks code as the ``apply`` or a disposer of ``fiber``.

    The mark is kept in a context variable, so a task started by that code
    inherits it. ``running`` turns false once the ``apply`` or the disposer has
    finished: a task that outlives it is ordinary code again.

    The marked code may hand the kernel a future or a task of its own to wait
    for. ``awaited`` is that future. The mark stops counting the moment it is
    done, without waiting for the kernel to notice.
    """

    fiber: Fiber
    running: bool = True
    awaited: asyncio.Future[Any] | None = None

    @property
    def live(self) -> bool:
        """Whether the code this mark describes is still running."""
        return self.running and not (self.awaited is not None and self.awaited.done())

    def adopt(self, awaitable: object) -> None:
        """Extend the mark to a future or task that the marked code returned.

        A task that already existed runs in a context of its own, so it cannot
        inherit the mark. It is looked up by identity instead.
        """
        if isinstance(awaitable, asyncio.Future):
            self.awaited = cast("asyncio.Future[Any]", awaitable)
        if isinstance(awaitable, asyncio.Task):
            _adopted[cast("asyncio.Task[Any]", awaitable)] = self


_lifecycle: contextvars.ContextVar[Lifecycle | None] = contextvars.ContextVar(
    "tack-lifecycle", default=None
)

_adopted: weakref.WeakKeyDictionary[asyncio.Task[Any], Lifecycle] = weakref.WeakKeyDictionary()


def _current_mark() -> Lifecycle | None:
    """Return the live lifecycle mark of the calling code, if it has one."""
    task = asyncio.current_task()
    adopted = None if task is None else _adopted.get(task)
    if adopted is not None and adopted.live:
        return adopted
    mark = _lifecycle.get()
    return mark if mark is not None and mark.live else None


@dataclass(slots=True, eq=False)
class Provision:
    """A service value and the fiber that provides it.

    ``serial`` is unique to this act of providing. A dependent records the
    serials it loaded against, so it reloads whenever a service is provided
    anew, even by the same fiber.

    ``withdrawn`` is set when the provider starts to withdraw the service. The
    service is unavailable from then on, but the provision keeps its name
    until every dependent has finished unloading.
    """

    key: ServiceKey[Any]
    value: Any
    fiber: Fiber
    serial: int
    withdrawn: bool = False


class KernelState:
    """State shared by every fiber and context of one application."""

    __slots__ = ("_uids", "drivers", "fibers", "hooks", "services")

    def __init__(self) -> None:
        self._uids = itertools.count()
        self.drivers: set[asyncio.Task[None]] = set()
        self.fibers: dict[int, Fiber] = {}
        self.hooks = HookTable()
        self.services: dict[ServiceKey[Any], Provision] = {}

    def next_uid(self) -> int:
        """Return a fiber identifier that has not been used before."""
        return next(self._uids)

    def lifecycle_fiber(self) -> Fiber | None:
        """Return the fiber whose ``apply`` or disposer the caller is part of, if any."""
        mark = _current_mark()
        if mark is None or mark.fiber._kernel is not self:
            return None
        return mark.fiber

    def waits_for(self, target: Fiber, caller: Fiber) -> bool:
        """Return whether ``target`` cannot settle before ``caller`` has.

        To unload, a fiber waits for the fibers below it, and for the fibers
        that inject a service held by it or by a fiber below it. Each of those
        waits in the same way for others.
        """
        caller_and_above = set(caller.lineage())
        seen = {target}
        waited_for = [target]
        while waited_for:
            member = waited_for.pop()
            if member in caller_and_above:
                return True
            for provision in self.services.values():
                if all(holder is not member for holder in provision.fiber.lineage()):
                    continue
                for fiber in self.fibers.values():
                    if fiber not in seen and fiber.injects(provision.key):
                        seen.add(fiber)
                        waited_for.append(fiber)
        return False

    def would_wait_for_itself(self, target: Fiber) -> bool:
        """Return whether the caller is code that ``target`` waits for.

        Such code must not wait for ``target`` in turn: neither would finish.
        """
        caller = self.lifecycle_fiber()
        return caller is not None and self.waits_for(target, caller)

    def withdrawal_waits_for(self, key: ServiceKey[Any], caller: Fiber) -> bool:
        """Return whether withdrawing ``key`` cannot finish before ``caller`` has settled."""
        return any(
            fiber.injects(key) and self.waits_for(fiber, caller) for fiber in self.fibers.values()
        )

    def lookup(self, key: ServiceKey[Any], viewer: Fiber) -> Provision | None:
        """Return the provision of ``key`` if ``viewer`` can see it.

        A service is visible to everyone while its provider is active. The
        provider itself and the fibers below it also see the service while the
        provider is loading or unloading. Nobody sees a service that is being
        withdrawn.
        """
        provision = self.services.get(key)
        if provision is None or provision.withdrawn:
            return None
        provider = provision.fiber
        if provider.state is FiberState.ACTIVE:
            return provision
        if any(fiber is provider for fiber in viewer.lineage()):
            return provision
        return None

    def available(self, key: ServiceKey[Any]) -> Provision | None:
        """Return the provision of ``key`` if dependents may load against it."""
        provision = self.services.get(key)
        if provision is None or provision.withdrawn:
            return None
        if provision.fiber.state is not FiberState.ACTIVE:
            return None
        return provision

    def declared(self, key: ServiceKey[Any], viewer: Fiber) -> bool:
        """Return whether ``viewer`` is entitled to rely on ``key``.

        A plugin may rely on a service that it provides itself, and on one that
        it or an enclosing plugin injects: the kernel unloads the plugin before
        an injected service goes away. Code outside every plugin may rely on
        any service.
        """
        provision = self.services.get(key)
        provider = None if provision is None else provision.fiber
        in_own_plugin = True
        inside_a_plugin = False
        for fiber in viewer.lineage():
            if in_own_plugin and fiber is provider:
                return True
            if fiber.plugin is not None:
                if key in fiber.plugin.inject:
                    return True
                in_own_plugin = False
                inside_a_plugin = True
        return not inside_a_plugin

    def notify(self, key: ServiceKey[Any]) -> None:
        """Re-evaluate every fiber that injects ``key``."""
        for fiber in tuple(self.fibers.values()):
            if fiber.injects(key):
                fiber._refresh()

    def announce(self, provider: Fiber) -> None:
        """Re-evaluate every fiber that injects a service of ``provider``."""
        for provision in tuple(self.services.values()):
            if provision.fiber is provider:
                self.notify(provision.key)

    def has_unsettled_dependents(self, key: ServiceKey[Any]) -> bool:
        """Return whether a fiber that injects ``key`` still has loading or unloading to do."""
        return any(fiber.injects(key) and not fiber._at_rest() for fiber in self.fibers.values())

    async def dependents_settled(self, key: ServiceKey[Any]) -> None:
        """Wait until the fibers that inject ``key`` have finished changing state.

        Call this only while ``key`` is unavailable. A dependent then has
        nothing to load against, so waiting for it to settle is waiting for it
        to unload.
        """
        for fiber in tuple(self.fibers.values()):
            if fiber.injects(key):
                await fiber.settled()

    async def all_dependents_settled(self, provider: Fiber) -> None:
        """Wait for the dependents of every service that ``provider`` holds.

        This includes services the provider has begun to withdraw and whose
        dependents are still unloading.
        """
        for provision in tuple(self.services.values()):
            if provision.fiber is provider:
                await self.dependents_settled(provision.key)


class EffectHandle:
    """A registered effect. Disposing the handle undoes the effect once.

    A disposer that returns an awaitable is driven by a task of its own, so
    that cancelling whoever asked for the disposal does not interrupt the
    cleanup.
    """

    __slots__ = (
        "_awaitable",
        "_begun",
        "_disposer",
        "_done",
        "_entered",
        "_fiber",
        "_mark",
        "_pending",
        "_waits_for",
    )

    def __init__(
        self,
        fiber: Fiber,
        disposer: Disposer | None,
        waits_for: Callable[[Fiber], bool] | None = None,
    ) -> None:
        self._fiber = fiber
        self._disposer = disposer
        self._waits_for = waits_for
        self._mark = Lifecycle(fiber)
        self._awaitable: Awaitable[None] | None = None
        self._pending: asyncio.Task[None] | None = None
        self._begun = False
        self._entered = False
        self._done = False

    @property
    def disposed(self) -> bool:
        """Whether the effect has been undone."""
        return self._done

    async def dispose(self) -> None:
        """Undo the effect and wait until that is finished.

        Only the first call runs the disposer. A caller that waits for it
        receives the error it raises, if any. A call made after it has finished
        returns at once.

        An ``apply`` or a disposer that this cleanup itself waits for cannot
        wait for it in turn. Called from such code, this starts the cleanup and
        returns. That is the case for the disposer itself, and for the code of
        a plugin whose unloading the cleanup waits for: withdrawing a service
        waits for the plugins that inject it.
        """
        self._begin()
        if not self._caller_is_waited_for():
            await self._join()

    def _caller_is_waited_for(self) -> bool:
        mark = _current_mark()
        if mark is None:
            return False
        if mark is self._mark:
            return True
        return self._waits_for is not None and self._waits_for(mark.fiber)

    async def _join(self) -> None:
        """Wait until the effect is undone, whoever is asking."""
        while not self._done:
            pending = self._pending
            if pending is None:  # pragma: no cover - needs an eagerly started, unmarked task
                # Asked while the disposer is still being called.
                return
            if pending.done():
                if self._entered:
                    self._finish()
                    return
                # Cancelled before its first step: the cleanup has yet to run.
                pending = self._start()
            await asyncio.shield(pending)

    def _begin(self) -> None:
        """Start undoing the effect without waiting for it."""
        if self._begun:
            return
        self._begun = True
        disposer, self._disposer = self._disposer, None
        token = _lifecycle.set(self._mark)
        try:
            result = None if disposer is None else disposer()
        except BaseException:
            self._finish()
            raise
        finally:
            _lifecycle.reset(token)
        if inspect.isawaitable(result):
            self._mark.adopt(result)
            self._awaitable = result
            self._start()
        else:
            self._finish()

    def _start(self) -> asyncio.Task[None]:
        awaitable = self._awaitable
        assert awaitable is not None
        task = _spawn(self._run(awaitable), f"tack-effect-{self._fiber.uid}", self._mark)
        self._pending = task
        task.add_done_callback(self._settle)
        return task

    async def _run(self, awaitable: Awaitable[None]) -> None:
        self._entered = True
        try:
            await awaitable
        finally:
            # A task the disposer started must be ordinary code from here on,
            # even if it runs before this task is seen to have finished.
            self._mark.running = False

    def _settle(self, task: asyncio.Task[None]) -> None:
        if not self._entered:
            return
        if task.cancelled():
            logger.error("a cleanup of %r was cancelled before it finished", self._fiber.name)
        self._finish()

    def _finish(self) -> None:
        if self._done:
            return
        self._done = True
        self._awaitable = None
        self._mark.running = False
        self._fiber._forget(self)


class Fiber:
    """One instance of a plugin, and the owner of everything it registered.

    A fiber loads by calling its plugin's ``apply`` and unloads by undoing the
    effects registered since, in reverse order and one at a time. Loading and
    unloading run in the background; ``await fiber`` waits for them and raises
    the error of a failed load.

    The root fiber and scope fibers have no plugin. They are active from the
    moment they are created until they are disposed.

    Attributes:
        uid: A number unique within the application.
        parent: The fiber whose context created this one, or ``None`` for the root.
        plugin: The plugin this fiber is an instance of, if any.
        config: The validated config handed to ``apply``.
        error: Why the most recent load failed, if it did.
        state: Where the fiber is in its lifecycle.
        ctx: The context whose registrations this fiber owns.
    """

    __slots__ = (
        "_broken",
        "_disposed",
        "_driver",
        "_effects",
        "_failed",
        "_handle",
        "_kernel",
        "_loaded",
        "_wanted",
        "config",
        "ctx",
        "error",
        "parent",
        "plugin",
        "state",
        "uid",
    )

    ctx: Context

    def __init__(
        self,
        kernel: KernelState,
        parent: Fiber | None,
        plugin: Plugin[Any] | None = None,
        config: object = None,
    ) -> None:
        self.uid = kernel.next_uid()
        self.parent = parent
        self.plugin = plugin
        self.config: Any = config
        self.error: Exception | None = None
        self.state = FiberState.PENDING
        self._kernel = kernel
        self._effects: list[EffectHandle] = []
        self._wanted: Epoch | None = None
        self._loaded: Epoch | None = None
        self._failed: Epoch | None = None
        self._driver: asyncio.Task[None] | None = None
        self._handle: EffectHandle | None = None
        self._disposed = False
        self._broken = False
        if plugin is None:
            self._wanted = self._loaded = ()
            self.state = FiberState.ACTIVE
        elif plugin.config is not None:
            try:
                self.config = plugin.config.model_validate({} if config is None else config)
            except Exception as error:
                # Usually a ValidationError, but a validator may raise anything.
                logger.error("plugin %r has an invalid config: %s", plugin.name, error)
                self.error = error
                self._broken = True

    @property
    def name(self) -> str:
        """A short label for logs and error messages."""
        if self.plugin is not None:
            return self.plugin.name
        return "root" if self.parent is None else "scope"

    def __repr__(self) -> str:
        return f"<Fiber {self.name!r} #{self.uid} {self.state.value}>"

    def lineage(self) -> Iterator[Fiber]:
        """Yield this fiber, then its parent, and so on up to the root."""
        fiber: Fiber | None = self
        while fiber is not None:
            yield fiber
            fiber = fiber.parent

    def injects(self, key: ServiceKey[Any]) -> bool:
        """Return whether this fiber's plugin requires ``key``."""
        return self.plugin is not None and key in self.plugin.inject

    @property
    def missing(self) -> tuple[ServiceKey[Any], ...]:
        """The injected services that are not available right now."""
        inject = () if self.plugin is None else self.plugin.inject
        available = self._kernel.available
        return tuple(key for key in inject if available(key) is None)

    def __await__(self) -> Generator[Any, None, Fiber]:
        return self._ready().__await__()

    def effect(
        self,
        setup: Callable[[], Disposer | None],
        *,
        waits_for: Callable[[Fiber], bool] | None = None,
    ) -> EffectHandle:
        """Run ``setup`` now and keep the disposer it returns.

        Args:
            setup: Performs a registration and returns what undoes it.
            waits_for: For the kernel's own effects: tells whether undoing
                this one waits for a given fiber to settle.

        Raises:
            InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
        """
        self._require_loaded()
        handle = EffectHandle(self, setup(), waits_for)
        self._effects.append(handle)
        return handle

    async def dispose(self) -> None:
        """Unload the fiber for good and wait until its effects are undone.

        Cancelling the caller does not interrupt the unload.

        An ``apply`` or a disposer that the fiber's unload waits for cannot
        wait for that unload in turn. Called from such code, this requests the
        disposal and returns. That is the case for the fiber's own code, for
        the code of fibers below it, and for the code of plugins that inject a
        service the fiber or a fiber below it provides.
        """
        self._retire()
        handle = self._handle
        if handle is not None:
            await handle.dispose()
        elif not self._kernel.would_wait_for_itself(self):
            await self._until_disposed()

    async def settled(self) -> None:
        """Wait until this fiber has no loading or unloading left to do.

        Cancelling the caller does not interrupt the load or unload. Called
        from code that the fiber itself waits for (see :meth:`dispose`), this
        returns at once.
        """
        if self._kernel.would_wait_for_itself(self):
            return
        while True:
            driver = self._live_driver()
            if driver is None:
                if not self._needs_work():
                    return
                # A driver was cancelled part-way. Carry on from where it stopped.
                self._kick()
                continue
            try:
                await asyncio.shield(driver)
            except asyncio.CancelledError:
                if _being_cancelled():
                    raise

    async def _ready(self) -> Fiber:
        await self.settled()
        if self.state is FiberState.FAILED and self.error is not None:
            raise self.error
        return self

    def _require_loaded(self) -> None:
        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
            raise InactiveEffectError(
                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
            )

    async def _until_disposed(self) -> None:
        self._retire()
        await self.settled()

    def _retire(self) -> None:
        self._disposed = True
        self._refresh()

    def _compute_epoch(self) -> Epoch | None:
        if self._disposed or self._broken:
            return None
        serials: list[int] = []
        for key in () if self.plugin is None else self.plugin.inject:
            provision = self._kernel.available(key)
            if provision is None:
                return None
            serials.append(provision.serial)
        return tuple(serials)

    def _refresh(self) -> None:
        wanted = self._compute_epoch()
        if wanted != self._wanted:
            self._wanted = wanted
            self._failed = None
            self.error = None
        self._kick()

    def _needs_work(self) -> bool:
        if self._loaded is not None:
            return self._loaded != self._wanted
        return self._wanted is not None and self._wanted != self._failed

    def _at_rest(self) -> bool:
        return self._live_driver() is None and not self._needs_work()

    def _live_driver(self) -> asyncio.Task[None] | None:
        driver = self._driver
        if driver is not None and driver.done():
            # Cancelled before it could run, so it never cleared itself.
            self._driver = None
            self._kernel.drivers.discard(driver)
            return None
        return driver

    def _kick(self) -> None:
        if self._live_driver() is not None:
            return
        if not self._needs_work():
            self._rest()
            return
        driver = _spawn(self._drive(), f"tack-fiber-{self.uid}")
        self._driver = driver
        self._kernel.drivers.add(driver)

    async def _drive(self) -> None:
        try:
            while self._needs_work():
                if self._loaded is not None:
                    await self._unload()
                else:
                    await self._load()
            self._rest()
        finally:
            driver = self._driver
            self._driver = None
            if driver is not None:  # pragma: no branch - always set while driving
                self._kernel.drivers.discard(driver)

    async def _load(self) -> None:
        plugin = self.plugin
        epoch = self._wanted
        assert plugin is not None
        assert epoch is not None
        self._loaded = epoch
        try:
            self._set_state(FiberState.LOADING)
            result = await self._apply(plugin)
            if result is not None:
                returned: object = result
                if not callable(returned):
                    raise InvalidPluginError(
                        f"plugin {self.name!r} returned {returned!r}; "
                        "apply must return a disposer or None"
                    )
                self._effects.append(EffectHandle(self, result))
        except (Exception, asyncio.CancelledError) as error:
            interrupted = isinstance(error, asyncio.CancelledError) and _being_cancelled()
            if interrupted:
                # This request has been received. Take it off the count, so
                # that the rollback can tell a second request from it.
                task = asyncio.current_task()
                assert task is not None
                task.uncancel()
            logger.error("plugin %r failed to load", self.name, exc_info=error)
            if isinstance(error, Exception):
                self.error = error
            else:
                self.error = KernelError(f"plugin {self.name!r} was cancelled while loading")
            self._failed = epoch
            await self._unload()
            if interrupted:
                self._rest()
                raise
            return
        if self._wanted == epoch:
            self._set_state(FiberState.ACTIVE)

    async def _apply(self, plugin: Plugin[Any]) -> Disposer | None:
        mark = Lifecycle(self)
        token = _lifecycle.set(mark)
        try:
            result = plugin.apply(self.ctx, self.config)
            if inspect.isawaitable(result):
                mark.adopt(result)
                result = await result
            return result
        finally:
            mark.running = False
            _lifecycle.reset(token)

    async def _unload(self) -> None:
        self._set_state(FiberState.UNLOADING)
        await self._kernel.all_dependents_settled(self)
        await self._undo_effects()
        self._loaded = None

    async def _undo_effects(self) -> None:
        while self._effects:
            handle = self._effects[-1]
            try:
                handle._begin()
                await handle._join()
            except asyncio.CancelledError:
                if _being_cancelled():
                    raise
                if handle._pending is None:
                    # A task reports its own cancellation when it settles.
                    logger.error("a cleanup of %r was cancelled before it finished", self.name)
            except Exception:
                logger.exception("an effect of %r failed to clean up", self.name)

    def _forget(self, handle: EffectHandle) -> None:
        effects = self._effects
        if effects and effects[-1] is handle:
            effects.pop()
        elif handle in effects:  # pragma: no branch - a handle is forgotten once
            effects.remove(handle)

    def _rest(self) -> None:
        if self._loaded is not None:
            self._set_state(FiberState.ACTIVE)
        elif self._disposed:
            self._kernel.fibers.pop(self.uid, None)
            self._set_state(FiberState.DISPOSED)
        elif self.error is not None:
            self._set_state(FiberState.FAILED)
        else:
            self._set_state(FiberState.PENDING)

    def _set_state(self, state: FiberState) -> None:
        previous = self.state
        if previous is state:
            return
        self.state = state
        if FiberState.ACTIVE in (previous, state):
            self._kernel.announce(self)
        listeners = self._kernel.hooks.select(FIBER_STATE_CHANGED, EmitEvent)
        run_emit(listeners, FIBER_STATE_CHANGED, FiberStateChange(self, previous, state))
```

用下面的内容替换 `src/tack/kernel/plugin.py` 的全部内容：

```python
"""Plugins: the units the kernel loads and unloads."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Generic, TypeAlias, TypeVar, cast

from pydantic import BaseModel

from tack.kernel.errors import InvalidPluginError
from tack.kernel.fiber import Disposer
from tack.kernel.services import ServiceKey

if TYPE_CHECKING:
    from tack.kernel.context import Context

ConfigT = TypeVar("ConfigT")

ApplyResult: TypeAlias = Disposer | None
"""What ``apply`` returns: a disposer that runs first on unload, or nothing."""

Apply: TypeAlias = Callable[["Context", ConfigT], ApplyResult | Awaitable[ApplyResult]]
"""The signature of ``apply``. It may be a plain function or a coroutine function."""


@dataclass(frozen=True, slots=True)
class Plugin(Generic[ConfigT]):
    """A unit of functionality that the kernel can load and unload.

    The type parameter is the type of the config that ``apply`` accepts. When a
    config model is given, type checkers require it to be that type.

    Attributes:
        name: A label for logs and error messages.
        apply: Called with the plugin's own context and its config each time
            the plugin loads. It may be a coroutine function. It may return a
            disposer, which runs first when the plugin unloads.
        inject: The services the plugin requires. The plugin loads only while
            all of them are available, unloads when one is withdrawn, and
            loads again when it returns.
        config: A pydantic model that the raw config is validated against, or
            ``None`` to hand the raw config to ``apply`` unchanged.

    Raises:
        InvalidPluginError: A field has the wrong shape.
    """

    name: str
    apply: Apply[ConfigT]
    inject: tuple[ServiceKey[Any], ...] = field(default=(), kw_only=True)
    config: type[ConfigT] | None = field(default=None, kw_only=True)

    def __post_init__(self) -> None:
        # The annotations describe what a type checker enforces. Callers that
        # are not type-checked, such as resolve_plugin, are checked here.
        name = cast("object", self.name)
        if not isinstance(name, str) or not name:
            raise InvalidPluginError(f"a plugin needs a name, not {name!r}")
        if not callable(cast("object", self.apply)):
            raise InvalidPluginError(f"the apply of plugin {name!r} must be callable")
        config = cast("object", self.config)
        if config is not None and not (isinstance(config, type) and issubclass(config, BaseModel)):
            raise InvalidPluginError(f"the config of plugin {name!r} must be a pydantic model")
        inject = cast("object", self.inject)
        keys = cast("Sequence[object]", inject) if isinstance(inject, list | tuple) else None
        if keys is None or not all(isinstance(key, ServiceKey) for key in keys):
            raise InvalidPluginError(
                f"the inject of plugin {name!r} must be a list or tuple of service keys"
            )
        object.__setattr__(self, "inject", tuple(keys))


def resolve_plugin(candidate: object) -> Plugin[Any]:
    """Turn any accepted plugin shape into a :class:`Plugin`.

    Three shapes are accepted: a :class:`Plugin`; a module or object with an
    ``apply`` callable and optional ``name``, ``inject`` and ``Config``
    attributes; and a bare callable, which is its own ``apply``.

    Raises:
        InvalidPluginError: ``candidate`` has none of these shapes.
    """
    if isinstance(candidate, Plugin):
        return cast("Plugin[Any]", candidate)
    apply = getattr(candidate, "apply", candidate)
    if not callable(apply):
        raise InvalidPluginError(
            f"{candidate!r} is not a plugin: expected a callable or an object with apply()"
        )
    name = getattr(candidate, "name", None) or getattr(candidate, "__name__", None)
    if name is None:
        name = type(candidate).__name__
    return Plugin(
        name,
        cast("Apply[Any]", apply),
        inject=getattr(candidate, "inject", ()),
        config=getattr(candidate, "Config", None),
    )
```

修改 `src/tack/kernel/context.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/context.py
+++ b/src/tack/kernel/context.py
@@ -5,10 +5,11 @@
 from __future__ import annotations
 
 import asyncio
-from collections.abc import Callable
+from collections.abc import Awaitable, Callable
 from typing import Any, overload
 
 from tack.kernel.errors import (
+    LifecycleError,
     ServiceConflictError,
     ServiceNotInjectedError,
     ServiceUnavailableError,
@@ -90,9 +91,17 @@ class Context:
         parent._require_loaded()
         fiber = Fiber(self._kernel, parent)
         scoped = Context._owned_by(self._kernel, fiber, key)
-        fiber._handle = parent.effect(lambda: fiber._dispose_owned)
+        self._own(fiber)
         return scoped
 
+    def _own(self, child: Fiber) -> None:
+        """Make the owning fiber dispose ``child`` when it unloads."""
+        kernel = self._kernel
+        child._handle = self.fiber.effect(
+            lambda: child._until_disposed,
+            waits_for=lambda caller: kernel.waits_for(child, caller),
+        )
+
     def plugin(self, plugin: object, config: object = None) -> Fiber:
         """Load ``plugin`` as a child of the owning fiber.
 
@@ -116,7 +125,7 @@ class Context:
         parent._require_loaded()
         fiber = Fiber(self._kernel, parent, spec, config)
         Context._owned_by(self._kernel, fiber, self.scope_key)
-        fiber._handle = parent.effect(lambda: fiber._dispose_owned)
+        self._own(fiber)
         self._kernel.fibers[fiber.uid] = fiber
         fiber._refresh()
         return fiber
@@ -126,32 +135,51 @@ class Context:
 
         The service is withdrawn when the owning fiber unloads or the returned
         handle is disposed. Other fibers see it once the owning fiber is active.
-        Plugins that inject the service load when it appears. When it is
-        withdrawn they unload first, and the withdrawal waits for them.
+        Plugins that inject the service load when it appears.
+
+        Withdrawing makes the service unavailable at once and then waits for
+        the plugins that inject it to unload. The name stays taken until they
+        have, so a replacement can be provided only after the withdrawal has
+        finished.
 
         Raises:
-            ServiceConflictError: Another provider already holds the name.
+            ServiceConflictError: Another provider holds the name, or the name
+                is still being withdrawn.
             InactiveEffectError: The owning fiber is not loaded.
         """
         kernel = self._kernel
-        fiber = self.fiber
+        provision = Provision(key, value, self.fiber, kernel.next_uid())
 
-        async def withdraw() -> None:
-            del kernel.services[key]
+        def withdraw() -> Awaitable[None] | None:
+            provision.withdrawn = True
             kernel.notify(key)
-            await kernel.dependents_settled(key, fiber)
+            if not kernel.has_unsettled_dependents(key):
+                del kernel.services[key]
+                return None
+            return release_the_name()
+
+        async def release_the_name() -> None:
+            try:
+                await kernel.dependents_settled(key)
+            finally:
+                del kernel.services[key]
 
         def setup() -> Disposer:
             existing = kernel.services.get(key)
             if existing is not None:
+                problem = (
+                    "still being withdrawn by" if existing.withdrawn else "already provided by"
+                )
                 raise ServiceConflictError(
-                    f"service {key.name!r} is already provided by {existing.fiber.name!r}"
+                    f"service {key.name!r} is {problem} {existing.fiber.name!r}"
                 )
-            kernel.services[key] = Provision(key, value, fiber, kernel.next_uid())
+            kernel.services[key] = provision
             kernel.notify(key)
             return withdraw
 
-        return self.effect(setup)
+        return self.fiber.effect(
+            setup, waits_for=lambda caller: kernel.withdrawal_waits_for(key, caller)
+        )
 
     def get(self, key: ServiceKey[ServiceT]) -> ServiceT | None:
         """Return the service named by ``key``, or ``None`` if it is not available.
@@ -186,10 +214,27 @@ class Context:
         return tuple(self._kernel.fibers.values())
 
     async def settled(self) -> None:
-        """Wait until no plugin in the application is loading or unloading."""
-        drivers = self._kernel.drivers
-        current = asyncio.current_task()
-        while pending := [driver for driver in drivers if driver is not current]:
+        """Wait until no plugin in the application is loading or unloading.
+
+        Raises:
+            LifecycleError: Called from an ``apply`` or a disposer, or from a
+                task one of them started and is still running alongside. The
+                caller would be waiting for itself, directly or through
+                another plugin that waits for it.
+        """
+        kernel = self._kernel
+        if kernel.lifecycle_fiber() is not None:
+            raise LifecycleError(
+                "settled() cannot be awaited while a plugin loads or unloads: "
+                "the caller would be waiting for itself"
+            )
+        while True:
+            for fiber in tuple(kernel.fibers.values()):
+                # Carry on any work whose driver was cancelled part-way.
+                fiber._kick()
+            pending = [driver for driver in kernel.drivers if not driver.done()]
+            if not pending:
+                return
             await asyncio.wait(pending)
 
     def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
@@ -283,7 +328,7 @@ class Context:
 
         A listener that raises stops the dispatch and the error reaches the caller.
         """
-        return run_bail(self._select(event, BailEvent), payload)
+        return run_bail(self._select(event, BailEvent), event, payload)
 
     async def parallel(self, event: ParallelEvent[PayloadT], payload: PayloadT) -> None:
         """Run every listener of ``event`` concurrently and wait for all of them.
```

修改 `src/tack/kernel/__init__.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/__init__.py
+++ b/src/tack/kernel/__init__.py
@@ -7,6 +7,7 @@ from tack.kernel.errors import (
     InactiveEffectError,
     InvalidPluginError,
     KernelError,
+    LifecycleError,
     ServiceConflictError,
     ServiceNotInjectedError,
     ServiceUnavailableError,
@@ -25,11 +26,19 @@ from tack.kernel.events import (
     WaterfallEvent,
     WaterfallListener,
 )
-from tack.kernel.fiber import Disposer, EffectHandle, Fiber, FiberState
+from tack.kernel.fiber import (
+    FIBER_STATE_CHANGED,
+    Disposer,
+    EffectHandle,
+    Fiber,
+    FiberState,
+    FiberStateChange,
+)
 from tack.kernel.plugin import Apply, ApplyResult, Plugin, resolve_plugin
 from tack.kernel.services import ServiceKey
 
 __all__ = [
+    "FIBER_STATE_CHANGED",
     "Apply",
     "ApplyResult",
     "BailEvent",
@@ -44,9 +53,11 @@ __all__ = [
     "EventModeError",
     "Fiber",
     "FiberState",
+    "FiberStateChange",
     "InactiveEffectError",
     "InvalidPluginError",
     "KernelError",
+    "LifecycleError",
     "Next",
     "ParallelEvent",
     "ParallelListener",
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_cancellation.py tests/kernel/test_dependencies.py tests/kernel/test_eager_tasks.py tests/kernel/test_events_sync.py tests/kernel/test_fiber_states.py tests/kernel/test_lifecycle_properties.py tests/kernel/test_plugins.py tests/kernel/test_reentrancy.py tests/kernel/test_scopes.py tests/kernel/test_withdrawal.py -q`

Expected: `123 passed, 3 skipped in 3.23s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`182 passed, 3 skipped in 5.36s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`25 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "fix(kernel): harden disposal against cancellation, re-entrancy and withdrawal races"
```

---

### Task 12: 组合：分层的插件清单

实现从 YAML 清单组合应用：解析层，叠加补丁，填入环境变量，导入并挂载插件。

**要点**

- 层的 YAML 用 `yaml.safe_load` 读取，不会执行代码，也不会读别的文件。
- `flatten` 是纯函数。它返回新的条目列表，不修改输入。
- 修改不存在的条目、插入已存在的 id、`extends` 成环，都报 `CompositionError`，消息以层的来源开头。
- `mount` 分两步。第一步导入全部插件并填好配置，第二步才开始加载。条目的顺序不决定加载顺序。
- 配置模型的校验发生在第二步。校验失败的条目进入 `FAILED`，不影响其他条目，由 `problems()` 报告。
- 测试用的插件放在 `tests/kernel/fixtures/compose_demo/`。测试运行时把这个目录加入 `sys.path`。

**Files:**

- Modify: `pyproject.toml`
- Create: `src/tack/kernel/compose.py`
- Modify: `src/tack/kernel/__init__.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/__init__.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/clock.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/explodes.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/extras.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/keys.py`
- Test（新建）: `tests/kernel/fixtures/compose_demo/reporter.py`
- Test（新建）: `tests/kernel/test_compose_layers.py`
- Test（新建）: `tests/kernel/test_compose_mount.py`

**Step 1: 写测试**

新建 `tests/kernel/fixtures/compose_demo/__init__.py`：

```python
"""Plugins that the composition tests load by import path."""
```

新建 `tests/kernel/fixtures/compose_demo/clock.py`：

```python
"""A plugin written as a module: it provides the clock."""

from pydantic import BaseModel

from compose_demo.keys import CLOCK
from tack.kernel import Context

name = "clock"


class Config(BaseModel):
    now: int = 0


def apply(ctx: Context, config: Config) -> None:
    ctx.provide(CLOCK, config.now)
```

新建 `tests/kernel/fixtures/compose_demo/explodes.py`：

```python
"""A module that cannot be imported."""

raise RuntimeError("boom at import")
```

新建 `tests/kernel/fixtures/compose_demo/extras.py`：

```python
"""Plugins and non-plugins addressed as attributes of a module."""

from pydantic import BaseModel, field_validator

from compose_demo.keys import CLOCK, STORE
from tack.kernel import Context, Plugin


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
```

新建 `tests/kernel/fixtures/compose_demo/keys.py`：

```python
"""Service keys shared by the demo plugins. This module is not a plugin."""

from tack.kernel import ServiceKey

CLOCK = ServiceKey[int]("clock")
STORE = ServiceKey[dict[str, int]]("store")
REPORT = ServiceKey[dict[str, object]]("report")
```

新建 `tests/kernel/fixtures/compose_demo/reporter.py`：

```python
"""A plugin written as a module: it needs the clock and reports what it was given."""

from pydantic import BaseModel

from compose_demo.keys import CLOCK, REPORT
from tack.kernel import Context

name = "reporter"
inject = [CLOCK]


class Config(BaseModel):
    label: str = "report"
    port: int = 0


def apply(ctx: Context, config: Config) -> None:
    ctx.provide(REPORT, {"label": config.label, "port": config.port, "now": ctx.use(CLOCK)})
```

新建 `tests/kernel/test_compose_layers.py`：

```python
"""Layers: parsing them, stacking them, and filling in environment variables."""

import pytest

from tack.kernel import (
    Change,
    CompositionError,
    Entry,
    Insert,
    Layer,
    flatten,
    interpolate,
    parse_layer,
)

BASE = """
- insert:
    - id: session
      plugin: demo.session
    - id: llm
      plugin: demo.llm
      config:
        model: small
        temperature: 0
"""


def layers(**named: str) -> dict[str, Layer]:
    return {name: parse_layer(text, source=name) for name, text in named.items()}


def test_a_layer_may_be_a_bare_list_of_operations() -> None:
    layer = parse_layer(BASE, source="base")

    assert layer.extends is None
    assert layer.source == "base"
    assert [entry.id for entry in flatten([layer])] == ["session", "llm"]


def test_an_empty_file_is_an_empty_layer() -> None:
    assert flatten([parse_layer("")]) == []


def test_a_layer_can_be_built_without_yaml() -> None:
    layer = Layer(
        patch=(
            Insert(insert=(Entry(id="llm", plugin="demo.llm"),)),
            Change(id="llm", disabled=True),
        )
    )

    assert flatten([layer]) == [Entry(id="llm", plugin="demo.llm", disabled=True)]
    assert layer.source == "<layer>"


def test_a_layer_may_extend_another() -> None:
    known = layers(base=BASE)
    cli = parse_layer(
        """
        extends: base
        patch:
          - insert:
              - id: cli
                plugin: demo.cli
        """,
        source="cli",
    )

    entries = flatten([cli], known.__getitem__)

    assert [entry.id for entry in entries] == ["session", "llm", "cli"]
    assert entries[2].plugin == "demo.cli"
    assert entries[2].config is None
    assert entries[2].disabled is False


def test_a_change_replaces_the_whole_config() -> None:
    patch = parse_layer("- {id: llm, config: {model: large}}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].config == {"model": "large"}


def test_a_change_leaves_the_fields_it_does_not_mention() -> None:
    patch = parse_layer("- {id: llm, disabled: true}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].disabled is True
    assert entries[1].plugin == "demo.llm"
    assert entries[1].config == {"model": "small", "temperature": 0}


def test_a_change_can_swap_the_plugin_and_clear_the_config() -> None:
    patch = parse_layer("- {id: llm, plugin: demo.other_llm, config: null}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].plugin == "demo.other_llm"
    assert entries[1].config is None


def test_layers_are_applied_in_order() -> None:
    first = parse_layer("- {id: llm, disabled: true}", source="user")
    second = parse_layer("- {id: llm, disabled: false}", source="command line")

    entries = flatten([parse_layer(BASE), first, second])

    assert entries[1].disabled is False


def test_an_operation_can_change_an_entry_inserted_earlier_in_the_same_layer() -> None:
    layer = parse_layer(
        """
        - insert:
            - {id: cli, plugin: demo.cli}
        - {id: cli, config: {theme: dark}}
        """
    )

    entries = flatten([layer])

    assert entries[0].config == {"theme": "dark"}


def test_changing_an_unknown_entry_is_an_error() -> None:
    patch = parse_layer("- {id: lmm, disabled: true}", source="user patch")

    with pytest.raises(CompositionError, match="user patch: there is no entry 'lmm' to change"):
        flatten([parse_layer(BASE), patch])


def test_inserting_an_existing_id_is_an_error() -> None:
    patch = parse_layer("- insert: [{id: llm, plugin: demo.again}]", source="user patch")

    with pytest.raises(CompositionError, match="user patch: entry 'llm' already exists"):
        flatten([parse_layer(BASE), patch])


def test_only_the_first_layer_may_extend() -> None:
    known = layers(base=BASE)
    patch = parse_layer("extends: base", source="user patch")

    with pytest.raises(CompositionError, match="user patch: only the first layer may extend"):
        flatten([parse_layer(BASE), patch], known.__getitem__)


def test_a_layer_cannot_extend_itself_through_others() -> None:
    known = layers(one="extends: two", two="extends: one")

    with pytest.raises(CompositionError, match="two -> one -> two"):
        flatten([known["one"]], known.__getitem__)


def test_the_source_of_a_layer_is_not_mistaken_for_a_layer_name() -> None:
    base = parse_layer("- insert: [{id: x, plugin: demo.x}]", source="actual-base.yml")
    child = parse_layer("extends: base", source="base")

    entries = flatten([child], {"base": base}.__getitem__)

    assert [entry.id for entry in entries] == ["x"]


def test_extending_an_unknown_layer_is_an_error() -> None:
    known = layers(cli="extends: base")

    with pytest.raises(CompositionError, match="cli: there is no layer 'base' to extend"):
        flatten([known["cli"]], known.__getitem__)

    with pytest.raises(CompositionError, match="cli: there is no layer 'base' to extend"):
        flatten([known["cli"]])


@pytest.mark.parametrize(
    ("text", "complaint"),
    [
        ("- insert: [", "not valid YAML"),
        ("just a string", "list of operations"),
        ("- {id: llm, colour: blue}", "colour"),
        ("- {insert: [], id: llm}", "id"),
        ("- insert: [{id: llm}]", "plugin"),
        ("- {disabled: true}", "id"),
        ("- {id: llm, plugin: null}", "leave it out"),
        ("- {id: llm, disabled: null}", "leave it out"),
        ("extends: base\nextra: 1", "extra"),
    ],
)
def test_a_malformed_layer_is_rejected_with_its_source(text: str, complaint: str) -> None:
    with pytest.raises(CompositionError, match=complaint) as failure:
        parse_layer(text, source="broken.yml")

    assert str(failure.value).startswith("broken.yml: ")


def test_interpolate_fills_in_variables_everywhere() -> None:
    config = {
        "url": "http://${HOST}:${PORT}/v1",
        "models": ["${MODEL}", "fixed"],
        "nested": {"key_env": "${KEY_NAME}"},
        "retries": 3,
        "enabled": True,
        "nothing": None,
    }
    env = {"HOST": "localhost", "PORT": "11434", "MODEL": "qwen", "KEY_NAME": "MY_KEY"}

    assert interpolate(config, env) == {
        "url": "http://localhost:11434/v1",
        "models": ["qwen", "fixed"],
        "nested": {"key_env": "MY_KEY"},
        "retries": 3,
        "enabled": True,
        "nothing": None,
    }


def test_interpolate_uses_a_default_when_the_variable_is_missing() -> None:
    assert interpolate("${MODEL:-small}", {}) == "small"
    assert interpolate("${MODEL:-small}", {"MODEL": "large"}) == "large"
    assert interpolate("${MODEL:-}", {}) == ""


def test_interpolate_keeps_an_escaped_dollar_sign() -> None:
    assert interpolate("costs $$5, see $${HOME}", {"HOME": "/root"}) == "costs $5, see ${HOME}"


def test_interpolate_names_the_missing_variable() -> None:
    with pytest.raises(CompositionError, match="environment variable 'MODEL' is not set"):
        interpolate({"model": "${MODEL}"}, {})


@pytest.mark.parametrize(
    "text",
    [
        "${}",
        "${1X}",
        "${ NAME }",
        "${NAME",
        "${NAME:-{}}",
        "${NAME:-$$5}",
        "${NAME:-${OTHER}}",
    ],
)
def test_interpolate_rejects_a_reference_it_cannot_read(text: str) -> None:
    with pytest.raises(CompositionError, match="reference"):
        interpolate(text, {"NAME": "value", "OTHER": "other"})


def test_interpolate_leaves_a_lone_dollar_sign_alone() -> None:
    assert interpolate("costs $5 or 5$", {}) == "costs $5 or 5$"
```

新建 `tests/kernel/test_compose_mount.py`：

```python
"""Mounting a list of entries: importing the plugins and loading them."""

import sys
from pathlib import Path

import pytest

from tack.kernel import (
    CompositionError,
    Context,
    Entry,
    FiberState,
    ServiceKey,
    import_plugin,
    mount,
)

REPORT = ServiceKey[dict[str, object]]("report")

CLOCK = Entry(id="clock", plugin="compose_demo.clock", config={"now": 7})
REPORTER = Entry(id="reporter", plugin="compose_demo.reporter", config={"label": "demo"})
BROKEN = Entry(id="broken", plugin="compose_demo.extras:broken")


@pytest.fixture(autouse=True)
def demo_plugins_can_be_imported(monkeypatch: pytest.MonkeyPatch) -> None:
    fixtures = str(Path(__file__).parent / "fixtures")
    monkeypatch.setattr(sys, "path", [fixtures, *sys.path])


async def test_mount_loads_the_entries_whatever_their_order(ctx: Context) -> None:
    composition = mount(ctx, [REPORTER, CLOCK])
    await ctx.settled()

    assert list(composition.fibers) == ["reporter", "clock"]
    assert composition.fibers["clock"].state is FiberState.ACTIVE
    assert composition.fibers["clock"].name == "clock"
    assert ctx.use(REPORT) == {"label": "demo", "port": 0, "now": 7}
    assert composition.problems() == []


async def test_a_disabled_entry_is_not_even_imported(ctx: Context) -> None:
    missing = Entry(id="off", plugin="compose_demo.does_not_exist", disabled=True)

    composition = mount(ctx, [CLOCK, missing])
    await ctx.settled()

    assert list(composition.fibers) == ["clock"]


async def test_config_strings_are_filled_in_from_the_environment(ctx: Context) -> None:
    reporter = Entry(
        id="reporter",
        plugin="compose_demo.reporter",
        config={"label": "${LABEL:-fallback}", "port": "${PORT}"},
    )

    mount(ctx, [CLOCK, reporter], env={"PORT": "8080"})
    await ctx.settled()

    assert ctx.use(REPORT) == {"label": "fallback", "port": 8080, "now": 7}


async def test_problems_name_the_services_an_entry_is_waiting_for(ctx: Context) -> None:
    composition = mount(ctx, [REPORTER])
    await ctx.settled()

    assert composition.problems() == [
        "reporter (compose_demo.reporter) is waiting for the service 'clock'"
    ]


async def test_problems_report_an_entry_that_failed_to_load(ctx: Context) -> None:
    composition = mount(ctx, [CLOCK, BROKEN])
    await ctx.settled()

    assert composition.problems() == [
        "broken (compose_demo.extras:broken) failed: RuntimeError: cannot start"
    ]


async def test_problems_report_an_invalid_config(ctx: Context) -> None:
    clock = Entry(id="clock", plugin="compose_demo.clock", config={"now": "soon"})

    composition = mount(ctx, [clock])
    await ctx.settled()

    (problem,) = composition.problems()
    assert problem.startswith("clock (compose_demo.clock) failed: ValidationError: ")
    assert "now" in problem


async def test_nothing_is_loaded_when_an_entry_cannot_be_imported(ctx: Context) -> None:
    missing = Entry(id="missing", plugin="compose_demo.does_not_exist")

    with pytest.raises(CompositionError, match="entry 'missing': cannot import"):
        mount(ctx, [CLOCK, missing])

    assert ctx.fibers() == ()


async def test_nothing_is_loaded_when_a_variable_is_missing(ctx: Context) -> None:
    reporter = Entry(id="reporter", plugin="compose_demo.reporter", config={"port": "${PORT}"})

    with pytest.raises(CompositionError, match="entry 'reporter': environment variable 'PORT'"):
        mount(ctx, [CLOCK, reporter], env={})

    assert ctx.fibers() == ()


async def test_two_entries_cannot_share_an_id(ctx: Context) -> None:
    with pytest.raises(CompositionError, match="entry 'clock' appears twice"):
        mount(ctx, [CLOCK, CLOCK])

    assert ctx.fibers() == ()


def test_import_plugin_accepts_a_module_or_an_attribute() -> None:
    assert import_plugin("compose_demo.clock").name == "clock"
    assert import_plugin("compose_demo.extras:broken").name == "broken"


@pytest.mark.parametrize(
    ("path", "complaint"),
    [
        ("compose_demo.does_not_exist", "cannot import"),
        ("compose_demo.extras:nowhere", "has no attribute 'nowhere'"),
        ("compose_demo.extras:not_a_plugin", "is not a plugin"),
        ("compose_demo.keys", "is not a plugin"),
        ("compose_demo.explodes", "cannot import 'compose_demo.explodes': RuntimeError"),
        ("", "is not an import path"),
        (":apply", "is not an import path"),
        (".relative", "is not an import path"),
        ("compose_demo.clock:", "is not an import path"),
    ],
)
def test_import_plugin_explains_what_it_could_not_use(path: str, complaint: str) -> None:
    with pytest.raises(CompositionError, match=complaint):
        import_plugin(path)


async def test_problems_report_an_entry_that_was_disposed(ctx: Context) -> None:
    composition = mount(ctx, [CLOCK])
    await ctx.settled()

    await composition.fibers["clock"].dispose()

    assert composition.problems() == ["clock (compose_demo.clock) is disposed"]


async def test_problems_list_every_missing_service(ctx: Context) -> None:
    composition = mount(ctx, [Entry(id="needy", plugin="compose_demo.extras:needy")])
    await ctx.settled()

    assert composition.problems() == [
        "needy (compose_demo.extras:needy) is waiting for the services 'clock', 'store'"
    ]


async def test_problems_report_a_disposed_entry_even_if_it_injects_services(ctx: Context) -> None:
    composition = mount(ctx, [Entry(id="needy", plugin="compose_demo.extras:needy")])
    await ctx.settled()

    await composition.fibers["needy"].dispose()

    assert composition.problems() == ["needy (compose_demo.extras:needy) is disposed"]


async def test_a_config_that_cannot_be_validated_does_not_stop_the_other_entries(
    ctx: Context,
) -> None:
    fussy = Entry(id="fussy", plugin="compose_demo.extras:fussy", config={"number": "many"})

    composition = mount(ctx, [CLOCK, fussy])
    await ctx.settled()

    assert composition.fibers["clock"].state is FiberState.ACTIVE
    assert composition.problems() == [
        "fussy (compose_demo.extras:fussy) failed: TypeError: cannot read 'many'"
    ]
```

**Step 2: 运行测试，确认失败**

逐个文件运行：`uv run pytest <文件> -q --timeout=10`

Expected:

| 文件 | 结果 | 第一个错误 |
|---|---|---|
| `test_compose_layers.py` | 1 error in 0.60s | ImportError: cannot import name 'Change' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py) |
| `test_compose_mount.py` | 1 error in 0.59s | ImportError: cannot import name 'Entry' from 'tack.kernel' (F:\agent harness\agent-harness\src\tack\kernel\__init__.py) |

**Step 3: 写实现**

修改 `pyproject.toml`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/pyproject.toml
+++ b/pyproject.toml
@@ -47,7 +47,7 @@ packages = ["src/tack"]
 [tool.ruff]
 line-length = 100
 target-version = "py311"
-src = ["src", "tests"]
+src = ["src", "tests", "tests/kernel/fixtures"]
 extend-exclude = ["docs"]
 
 [tool.ruff.lint]
```

新建 `src/tack/kernel/compose.py`：

```python
"""Compositions: an application described as a list of plugin entries.

An application is assembled from layers. A layer is a list of operations: an
operation inserts entries, or changes an entry that an earlier operation
inserted. The first layer may extend a named layer, which is then applied
before it. The layers after the first are patches on top.

In YAML a layer is either a bare list of operations or a mapping::

    extends: base
    patch:
      - insert:
          - id: cli
            plugin: tack.apps.cli
      - id: llm
        config:
          model: deepseek-flash
"""

from __future__ import annotations

import importlib
import os
import re
from collections.abc import Callable, Mapping, Sequence
from typing import Annotated, Any, cast

import yaml
from pydantic import (
    BaseModel,
    ConfigDict,
    Discriminator,
    PrivateAttr,
    Tag,
    ValidationError,
    model_validator,
)

from tack.kernel.context import Context
from tack.kernel.errors import CompositionError, InvalidPluginError
from tack.kernel.fiber import Fiber, FiberState
from tack.kernel.plugin import Plugin, resolve_plugin


class Entry(BaseModel):
    """One plugin of an application.

    Attributes:
        id: The name by which later operations address the entry.
        plugin: Where to import the plugin from: ``package.module`` for a
            module that is itself a plugin, or ``package.module:attribute``.
        config: The raw config handed to the plugin. Strings in it may refer
            to environment variables as ``${NAME}`` or ``${NAME:-default}``.
        disabled: Whether to leave the plugin out.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    plugin: str
    config: Any = None
    disabled: bool = False


class Insert(BaseModel):
    """An operation that adds entries."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    insert: tuple[Entry, ...]


class Change(BaseModel):
    """An operation that replaces fields of the entry called ``id``.

    Only the fields that are written out are replaced. ``config`` is replaced
    as a whole, never merged, and may be set to null to clear it.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    plugin: str | None = None
    config: Any = None
    disabled: bool | None = None

    @model_validator(mode="after")
    def _only_config_may_be_null(self) -> Change:
        for name in ("plugin", "disabled"):
            if name in self.model_fields_set and getattr(self, name) is None:
                raise ValueError(f"{name} cannot be null; leave it out to keep what the entry has")
        return self


def _kind_of(operation: object) -> str:
    if isinstance(operation, Mapping):
        return "insert" if "insert" in operation else "change"
    return "insert" if isinstance(operation, Insert) else "change"


Operation = Annotated[
    Annotated[Insert, Tag("insert")] | Annotated[Change, Tag("change")],
    Discriminator(_kind_of),
]


class Layer(BaseModel):
    """A list of operations, optionally on top of a named layer.

    Attributes:
        extends: The name of the layer to apply first, if any.
        patch: The operations, applied in order.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    extends: str | None = None
    patch: tuple[Operation, ...] = ()

    _source: str = PrivateAttr(default="<layer>")

    @property
    def source(self) -> str:
        """Where the layer was read from. Error messages start with it."""
        return self._source


def parse_layer(text: str, *, source: str = "<layer>") -> Layer:
    """Read a layer from YAML.

    Args:
        text: The YAML document.
        source: A name for the document, such as its path, for error messages.

    Raises:
        CompositionError: The document is not YAML or does not describe a layer.
    """
    try:
        data: object = yaml.safe_load(text)
    except yaml.YAMLError as error:
        raise CompositionError(f"{source}: not valid YAML: {error}") from error
    if data is None:
        data = {}
    elif isinstance(data, list):
        data = {"patch": cast("list[object]", data)}
    elif not isinstance(data, dict):
        raise CompositionError(
            f"{source}: a layer is a list of operations, or a mapping with 'extends' and 'patch'"
        )
    try:
        layer = Layer.model_validate(data)
    except ValidationError as error:
        raise CompositionError(f"{source}: {error}") from error
    layer._source = source  # pyright: ignore[reportPrivateUsage]
    return layer


def flatten(layers: Sequence[Layer], lookup: Callable[[str], Layer] | None = None) -> list[Entry]:
    """Turn a stack of layers into one list of entries.

    The layers are applied in order. Only the first may extend another layer.

    Args:
        layers: The first layer, followed by any patches.
        lookup: Finds a layer by the name given in ``extends``. It raises
            ``KeyError`` for a name it does not know.

    Raises:
        CompositionError: An operation cannot be applied, a layer that is
            extended does not exist, or the layers extend one another in a
            circle.
    """
    entries: list[Entry] = []
    for position, layer in enumerate(layers):
        if position == 0:
            for ancestor in reversed(_ancestors(layer, lookup)):
                entries = _apply(entries, ancestor)
        elif layer.extends is not None:
            raise CompositionError(f"{layer.source}: only the first layer may extend another")
        entries = _apply(entries, layer)
    return entries


def _ancestors(layer: Layer, lookup: Callable[[str], Layer] | None) -> list[Layer]:
    """Return the layers that ``layer`` extends, nearest first."""
    ancestors: list[Layer] = []
    visited: list[str] = []
    current = layer
    while current.extends is not None:
        name = current.extends
        if name in visited:
            circle = " -> ".join([*visited[visited.index(name) :], name])
            raise CompositionError(
                f"{layer.source}: the layers extend one another in a circle: {circle}"
            )
        visited.append(name)
        try:
            if lookup is None:
                raise KeyError(name)
            current = lookup(name)
        except KeyError:
            raise CompositionError(
                f"{current.source}: there is no layer {name!r} to extend"
            ) from None
        ancestors.append(current)
    return ancestors


def _apply(entries: list[Entry], layer: Layer) -> list[Entry]:
    result = list(entries)
    positions = {entry.id: position for position, entry in enumerate(result)}
    for operation in layer.patch:
        if isinstance(operation, Insert):
            for entry in operation.insert:
                if entry.id in positions:
                    raise CompositionError(f"{layer.source}: entry {entry.id!r} already exists")
                positions[entry.id] = len(result)
                result.append(entry)
            continue
        position = positions.get(operation.id)
        if position is None:
            raise CompositionError(f"{layer.source}: there is no entry {operation.id!r} to change")
        changes = {name: getattr(operation, name) for name in operation.model_fields_set}
        del changes["id"]
        result[position] = result[position].model_copy(update=changes)
    return result


_REFERENCE = re.compile(
    r"\$\$"
    r"|\$\{(?P<name>[A-Za-z_][A-Za-z0-9_]*)(?::-(?P<default>[^{}$]*))?\}"
    r"|(?P<malformed>\$\{)"
)


def interpolate(value: Any, env: Mapping[str, str]) -> Any:
    """Return ``value`` with environment variables filled into its strings.

    ``${NAME}`` becomes the value of the variable, ``${NAME:-default}`` falls
    back to the default when the variable is not set, and ``$$`` is a literal
    dollar sign. A default is taken as written and cannot contain braces or
    dollar signs. Mappings and lists are searched all the way down. Everything
    that is not a string is returned unchanged.

    Raises:
        CompositionError: A variable is not set and has no default, or a
            ``${`` does not start a reference of one of these forms.
    """
    if isinstance(value, str):

        def fill(match: re.Match[str]) -> str:
            if match["malformed"] is not None:
                raise CompositionError(
                    f"cannot read the variable reference in {value!r}: write ${{NAME}} or "
                    "${NAME:-default}, without braces or dollar signs in the default, "
                    "and $$ for a dollar sign"
                )
            name = match["name"]
            if name is None:
                return "$"
            if name in env:
                return env[name]
            default = match["default"]
            if default is None:
                raise CompositionError(f"environment variable {name!r} is not set")
            return default

        return _REFERENCE.sub(fill, value)
    if isinstance(value, dict):
        items = cast("dict[Any, Any]", value)
        return {key: interpolate(item, env) for key, item in items.items()}
    if isinstance(value, list):
        return [interpolate(item, env) for item in cast("list[Any]", value)]
    return value


def import_plugin(path: str) -> Plugin[Any]:
    """Import the plugin that an entry names.

    Args:
        path: ``package.module`` for a module that is itself a plugin, or
            ``package.module:attribute`` for a plugin kept in a module.

    Raises:
        CompositionError: The module cannot be imported, the attribute does not
            exist, or what was found is not a plugin.
    """
    module_name, colon, attribute = path.partition(":")
    if not module_name or module_name.startswith(".") or (colon and not attribute):
        raise CompositionError(
            f"{path!r} is not an import path: write 'package.module' or 'package.module:attribute'"
        )
    try:
        target: object = importlib.import_module(module_name)
    except Exception as error:
        # Importing runs the module, which may raise anything.
        raise CompositionError(
            f"cannot import {module_name!r}: {type(error).__name__}: {error}"
        ) from error
    if attribute:
        if not hasattr(target, attribute):
            raise CompositionError(f"module {module_name!r} has no attribute {attribute!r}")
        target = getattr(target, attribute)
    try:
        return resolve_plugin(target)
    except InvalidPluginError as error:
        raise CompositionError(f"{path!r} is not a plugin: {error}") from error


class Composition:
    """The plugins that :func:`mount` loaded.

    Attributes:
        fibers: The fiber of each entry that was loaded, by entry id, in the
            order of the entries.
    """

    __slots__ = ("_entries", "fibers")

    def __init__(self, loaded: Sequence[tuple[Entry, Fiber]]) -> None:
        self._entries = {entry.id: entry for entry, _ in loaded}
        self.fibers = {entry.id: fiber for entry, fiber in loaded}

    def problems(self) -> list[str]:
        """Describe each loaded entry that is not active, in entry order.

        Call this once the application has settled. An entry is then inactive
        because it failed to load or because a service it injects is missing.
        """
        problems: list[str] = []
        for entry_id, fiber in self.fibers.items():
            label = f"{entry_id} ({self._entries[entry_id].plugin})"
            state = fiber.state
            missing = [repr(key.name) for key in fiber.missing]
            if state is FiberState.ACTIVE:
                continue
            if state is FiberState.FAILED:
                error = fiber.error
                problems.append(f"{label} failed: {type(error).__name__}: {error}")
            elif state is FiberState.PENDING and missing:
                noun = "service" if len(missing) == 1 else "services"
                problems.append(f"{label} is waiting for the {noun} {', '.join(missing)}")
            else:
                problems.append(f"{label} is {state.value}")
        return problems


def mount(
    ctx: Context, entries: Sequence[Entry], *, env: Mapping[str, str] | None = None
) -> Composition:
    """Load the enabled entries as plugins of ``ctx``.

    Every plugin is imported and every config filled in before anything is
    loaded, so a mistake in one entry leaves the context as it was. The order
    of the entries does not decide the order in which plugins become active:
    each loads as soon as the services it injects are available.

    Args:
        ctx: The context that will own the plugins.
        entries: The entries, as returned by :func:`flatten`.
        env: The variables for ``${NAME}`` references. Defaults to the
            environment of the process.

    Raises:
        CompositionError: Two entries share an id, a plugin cannot be imported,
            or a config refers to a variable that is not set.
        InactiveEffectError: The fiber that owns ``ctx`` is not loaded.
    """
    variables = os.environ if env is None else env
    seen: set[str] = set()
    prepared: list[tuple[Entry, Plugin[Any], Any]] = []
    for entry in entries:
        if entry.id in seen:
            raise CompositionError(f"entry {entry.id!r} appears twice")
        seen.add(entry.id)
        if entry.disabled:
            continue
        try:
            plugin = import_plugin(entry.plugin)
            config = interpolate(entry.config, variables)
        except CompositionError as error:
            raise CompositionError(f"entry {entry.id!r}: {error}") from error
        prepared.append((entry, plugin, config))
    return Composition([(entry, ctx.plugin(plugin, config)) for entry, plugin, config in prepared])
```

修改 `src/tack/kernel/__init__.py`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/src/tack/kernel/__init__.py
+++ b/src/tack/kernel/__init__.py
@@ -1,5 +1,17 @@
 """The plugin kernel: contexts, services, effects, events, and plugin instances."""
 
+from tack.kernel.compose import (
+    Change,
+    Composition,
+    Entry,
+    Insert,
+    Layer,
+    flatten,
+    import_plugin,
+    interpolate,
+    mount,
+    parse_layer,
+)
 from tack.kernel.context import Context
 from tack.kernel.errors import (
     CompositionError,
@@ -43,20 +55,25 @@ __all__ = [
     "ApplyResult",
     "BailEvent",
     "BailListener",
+    "Change",
+    "Composition",
     "CompositionError",
     "Context",
     "Disposer",
     "EffectHandle",
     "EmitEvent",
     "EmitListener",
+    "Entry",
     "Event",
     "EventModeError",
     "Fiber",
     "FiberState",
     "FiberStateChange",
     "InactiveEffectError",
+    "Insert",
     "InvalidPluginError",
     "KernelError",
+    "Layer",
     "LifecycleError",
     "Next",
     "ParallelEvent",
@@ -70,5 +87,10 @@ __all__ = [
     "ServiceUnavailableError",
     "WaterfallEvent",
     "WaterfallListener",
+    "flatten",
+    "import_plugin",
+    "interpolate",
+    "mount",
+    "parse_layer",
     "resolve_plugin",
 ]
```

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_compose_layers.py tests/kernel/test_compose_mount.py -q`

Expected: `59 passed in 0.56s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`241 passed, 3 skipped in 5.78s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`34 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**Step 5: 提交**

```bash
git add -A
git commit -m "feat(kernel): compose an application from layered plugin entries"
```

---

### Task 13: 持续集成、README 与示例

加上三平台的持续集成，并在 README 里用一个经过测试的例子说明内核。

**要点**

- 工作流的矩阵是 Ubuntu、Windows、macOS 乘以 Python 3.11、3.12，共六个作业。每个作业跑全部检查。
- `astral-sh/setup-uv` 没有 `v10` 这样的主版本标签，所以写完整版本号 `v10.2.0`。
- README 里的例子就是 `examples/kernel_demo.py` 的全文。测试会运行这个文件，并核对 README 里的文本与它一致。
- `test_public_api.py` 把内核导出的名字逐个列出。以后增删导出名必须同时改这个测试。

**Files:**

- Modify: `pyproject.toml`
- Create: `examples/kernel_demo.py`
- Create: `.github/workflows/ci.yml`
- Modify: `README.md`
- Test（新建）: `tests/kernel/test_public_api.py`
- Test（新建）: `tests/test_examples.py`

**Step 1: 写测试**

新建 `tests/kernel/test_public_api.py`：

```python
"""The names the kernel exports. Changing this list is changing the public API."""

import tack.kernel

EXPORTS = {
    # Contexts, fibers and effects
    "Context",
    "Disposer",
    "EffectHandle",
    "Fiber",
    "FiberState",
    "FiberStateChange",
    "FIBER_STATE_CHANGED",
    # Plugins
    "Apply",
    "ApplyResult",
    "Plugin",
    "resolve_plugin",
    # Services
    "ServiceKey",
    # Events
    "BailEvent",
    "BailListener",
    "EmitEvent",
    "EmitListener",
    "Event",
    "Next",
    "ParallelEvent",
    "ParallelListener",
    "SerialEvent",
    "SerialListener",
    "WaterfallEvent",
    "WaterfallListener",
    # Compositions
    "Change",
    "Composition",
    "Entry",
    "Insert",
    "Layer",
    "flatten",
    "import_plugin",
    "interpolate",
    "mount",
    "parse_layer",
    # Errors
    "CompositionError",
    "EventModeError",
    "InactiveEffectError",
    "InvalidPluginError",
    "KernelError",
    "LifecycleError",
    "ServiceConflictError",
    "ServiceNotInjectedError",
    "ServiceUnavailableError",
}


def test_the_kernel_exports_exactly_its_public_api() -> None:
    assert set(tack.kernel.__all__) == EXPORTS
    assert len(tack.kernel.__all__) == len(EXPORTS)


def test_every_exported_name_exists() -> None:
    missing = [name for name in tack.kernel.__all__ if not hasattr(tack.kernel, name)]

    assert missing == []
```

新建 `tests/test_examples.py`：

```python
"""The examples run, and the README shows them as they are."""

import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).parent.parent


def test_the_kernel_demo_prints_one_tick(capsys: pytest.CaptureFixture[str]) -> None:
    runpy.run_path(str(ROOT / "examples" / "kernel_demo.py"), run_name="__main__")

    assert capsys.readouterr().out == "tick 101\n"


def test_the_readme_shows_the_kernel_demo_unchanged() -> None:
    demo = (ROOT / "examples" / "kernel_demo.py").read_text(encoding="utf-8")
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    assert demo in readme
```

**Step 2: 运行测试，确认失败**

`test_public_api.py` 核对的名字在上一个任务结束时已经全部存在，所以它一写出来就通过。

逐个文件运行：`uv run pytest <文件> -q --timeout=10`

Expected:

| 文件 | 结果 | 第一个错误 |
|---|---|---|
| `test_public_api.py` | 2 passed in 0.46s | — |
| `test_examples.py` | 2 failed in 0.56s | FileNotFoundError: [Errno 2] No such file or directory: 'F:\\agent harness\\agent-harness\\examples\\kernel_demo.py' |

**Step 3: 写实现**

修改 `pyproject.toml`（可以存成补丁后用 `git apply` 应用）：

```diff
--- a/pyproject.toml
+++ b/pyproject.toml
@@ -78,9 +78,10 @@ convention = "google"
 
 [tool.ruff.lint.per-file-ignores]
 "tests/**" = ["D"]
+"examples/**" = ["D"]
 
 [tool.pyright]
-include = ["src", "tests"]
+include = ["src", "tests", "examples"]
 pythonVersion = "3.11"
 typeCheckingMode = "strict"
 reportUnnecessaryTypeIgnoreComment = true
```

新建 `examples/kernel_demo.py`：

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

新建 `.github/workflows/ci.yml`：

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  check:
    name: ${{ matrix.os }} / Python ${{ matrix.python }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
        python: ["3.11", "3.12"]
    steps:
      - uses: actions/checkout@v7

      - uses: astral-sh/setup-uv@v10.2.0
        with:
          python-version: ${{ matrix.python }}

      - name: Install
        run: uv sync --locked

      - name: Lint
        run: uv run ruff check .

      - name: Format
        run: uv run ruff format --check .

      - name: Type check
        run: uv run pyright

      - name: Test
        run: uv run coverage run -m pytest

      - name: Coverage
        run: uv run coverage report
```

修改 `README.md`（可以存成补丁后用 `git apply` 应用）：

````diff
--- a/README.md
+++ b/README.md
@@ -3,10 +3,62 @@
 Tack is a plugin-based agent harness written in Python. Every capability is a plugin:
 the model adapter, the tool registry, the session log, and the agent loop itself.
 
-**Status:** early development. Nothing is usable yet.
+**Status:** early development. The plugin kernel is in place. The agent loop, the tools
+and the command-line interface are being built on top of it, so there is nothing to
+install and run yet.
 
 Tack 是一个用 Python 写的插件化 Agent harness。模型适配、工具注册、会话日志和 Agent
-循环都是插件。项目处于早期开发阶段，目前还不能使用。
+循环都是插件。项目处于早期开发阶段：插件内核已经完成，Agent 循环、工具和命令行界面
+正在其上搭建，目前还没有可以安装运行的程序。
+
+## The kernel
+
+`tack.kernel` is a small asyncio plugin framework. It is the only part of Tack that
+exists today.
+
+- A **context** is a service repository. A plugin offers a service with `provide` and
+  reads one with `use`.
+- A **plugin** declares the services it needs in `inject`. It loads when all of them
+  are available, unloads when one is withdrawn, and loads again when it returns.
+- Every registration made through a context is an **effect** that is undone when the
+  plugin unloads, in reverse order.
+- **Events** are declared with one of five dispatch modes: `emit`, `parallel`,
+  `serial`, `bail` and `waterfall`.
+- An application is **composed** from YAML layers of plugin entries. A later layer
+  changes an entry by its id.
+
+```python
+import asyncio
+
+from tack.kernel import Context, EmitEvent, Plugin, ServiceKey
+
+CLOCK = ServiceKey[int]("clock")
+TICK = EmitEvent[int]("demo/tick")
+
+
+def announcer(ctx: Context, config: object) -> None:
+    now = ctx.use(CLOCK)
+    ctx.on(TICK, lambda step: print(f"tick {now + step}"))
+
+
+async def main() -> None:
+    root = Context()
+    root.plugin(Plugin("announcer", announcer, inject=(CLOCK,)))
+
+    root.emit(TICK, 1)  # nothing is printed: the plugin is waiting for the clock
+
+    clock = root.provide(CLOCK, 100)
+    await root.settled()
+    root.emit(TICK, 1)  # prints "tick 101"
+
+    await clock.dispose()  # the plugin unloads and its listener is removed
+    root.emit(TICK, 2)  # nothing is printed
+
+    await root.fiber.dispose()
+
+
+asyncio.run(main())
+```
 
 ## Development
 
````

**Step 4: 运行测试和全部检查，确认通过**

Run: `uv run pytest tests/kernel/test_public_api.py tests/test_examples.py -q`

Expected: `4 passed in 0.45s`

然后跑全部检查：

```bash
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Expected:

- 全套测试：`245 passed, 3 skipped in 6.21s`
- 覆盖率合计：`100%`
- ruff check：`All checks passed!`
- ruff format：`37 files already formatted`
- pyright：`0 errors, 0 warnings, 0 informations`

**推送后确认持续集成**

工作流要在推送之后才会运行。推送后到仓库的 Actions 页面确认六个作业全部通过。Python 3.12 的作业会多跑 `test_eager_tasks.py` 里的两个测试。

**Step 5: 提交**

```bash
git add -A
git commit -m "ci: check every push on three platforms; document the kernel"
```

---

## 完成之后

**推送与持续集成。** 这些提交都在本地的 `main` 分支上。用 GitHub Desktop 推送后，`.github/workflows/ci.yml` 会在 Ubuntu、Windows、macOS 三个平台和 Python 3.11、3.12 两个版本上各跑一遍全部检查。第 1 阶段的结束标志之一是这六个作业全部通过，这一项要在推送后确认。

**已知的两处取舍。**

1. `Plugin("x", apply, config=int)` 这样把非 pydantic 模型的类型传给 `config`，pyright 不会报错。`Plugin` 在构造时会检查并抛 `InvalidPluginError`，所以错误在导入或启动时就会暴露。要让类型检查器也拒绝它，需要给 `Plugin` 写两个重载的构造函数，目前没有做。
2. 内核自己的清理任务如果在开始执行之后被取消，这次清理无法续做。内核会记一条错误日志，并把这个 effect 视为已撤销。正常使用中内核的任务不会被取消，只有事件循环退出时根上下文还没有销毁才会出现这种情况。

**下一阶段。** 第 2 阶段是会话日志、模型适配、工具管线、Agent 循环和无界面模式。它的结束标志是 `tack run` 能用 DeepSeek Flash 完成一次带工具调用的任务。开始前另写一份实施计划。
