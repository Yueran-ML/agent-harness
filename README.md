# Columbina

Columbina is a plugin-based agent harness written in Python. Every capability is a plugin:
the model adapter, the tool registry, the session log, and the agent loop itself.

**Status:** early development. `columbina run` gives one task to an agent and prints the
answer. The agent talks to a model over the OpenAI Chat Completions protocol. It reads,
searches and changes the files of its working directory and runs commands, and what a
call may do is decided before it runs. An interactive interface comes next.

Columbina 是一个用 Python 写的插件化 Agent harness。模型适配、工具注册、会话日志和 Agent
循环都是插件。项目处于早期开发阶段。`columbina run` 已经可以把一个任务交给 Agent：它在工作目录里
读文件、搜索、改文件、执行命令，每次调用能做什么在执行之前判定。模型走 OpenAI Chat Completions
协议，默认接 DeepSeek。交互界面是接下来的工作。

## Running a task

Columbina needs Python 3.11 or newer and [uv](https://docs.astral.sh/uv/). It is not on PyPI
yet, so it runs from a clone of this repository.

```bash
uv sync
```

The default profile talks to DeepSeek. Put the API key into the environment variable
`DEEPSEEK_API_KEY`. Columbina reads it when it makes a request. The key is never part of a
config file, and it is cut out of every error message before the message is shown or
stored.

```console
$ uv run columbina run "What is the codename of release 0.1? It is written in notes.txt. Answer with the codename only."
Halyard
```

With `--json` the run is reported as lines of JSON, one for each thing that happens:

```console
$ uv run columbina run "What is the codename of release 0.1? ..." --session-id demo-1 --json
{"type": "session", "id": "demo-1", "cwd": "/work", "provider": "deepseek", "model": "deepseek-flash"}
{"type": "status", "turn": 1, "step": 1}
{"type": "thinking", "text": "Let me look at notes.txt."}
{"type": "tool_call", "id": "call_00_plQNpwHHD4aAtGs8H6551554", "name": "read", "arguments": "{\"path\": \"notes.txt\"}"}
{"type": "tool_result", "id": "call_00_plQNpwHHD4aAtGs8H6551554", "text": "     1\tRelease notes\n     2\t=============\n     3\t\n     4\tThe codename of release 0.1 is \"Halyard\".\n     5\tIt ships the kernel and the agent loop.", "error": null}
{"type": "status", "turn": 1, "step": 2}
{"type": "text", "text": "Halyard"}
{"type": "final", "reason": "completed", "detail": null, "text": "Halyard", "calls": 2, "usage": {"input_tokens": 689, "output_tokens": 49, "cache_read_tokens": 384, "cache_write_tokens": 0}}
```

These are the lines of a real run against `deepseek-flash`, with the working directory
shortened. The session that the run left behind is kept in
`tests/fixtures/recorded/deepseek-read`, and a test plays it again without the service.

| Option | What it does |
|---|---|
| `--cwd DIR` | The working directory of the agent. No file outside it is touched unless the session has full access. |
| `--model ID` | The model to use. The default is `deepseek-flash`, or `COLUMBINA_MODEL` if it is set. |
| `--session-id ID` | The session to start. If a session with this id exists, it is continued. |
| `--permission PRESET` | How much the agent may do: `read-only`, `workspace-write`, which is the default, or `full-access`. |
| `--allow RULE` | A rule that lets calls go ahead, such as `run(git status*)`. It may be given several times. |
| `--json`, `--no-json` | Report the run as lines of JSON, or print the answer alone. |
| `--profile NAME` | The profile to load: a name that comes with Columbina, or the path of a file. |
| `--patch FILE` | A file of changes to the profile. It may be given several times. |
| `--dump-config` | Print the plugin entries that would be loaded, and stop. |

The exit code is 0 if the turn was completed and 1 if it ended for another reason. It
is 2 if the command line, the profile or a plugin is at fault, in which case no request
is made. It is 130 after an interruption with Ctrl+C.

Sessions are stored under `~/.columbina/sessions`. The environment variable `COLUMBINA_HOME` puts
`~/.columbina` somewhere else. A session is one file of JSON lines to which events are only
ever appended.

## Changing files and running commands

The agent has six tools: `read`, `write`, `edit`, `glob`, `grep`, and a tool that runs
commands, which is called `powershell` on Windows and `bash` elsewhere.

```console
$ uv run columbina run 'check.py fails. Find the mistake in calc.py and fix it, and make sure that `python check.py` prints ok.' --allow 'run(python check.py)'
Fixed. In `calc.py`, `mean` was dividing the sum by `len(values) - 1` (that is, it computed a mean over one too few items, and blew up with `ZeroDivisionError` on a single value). It now divides by `len(values)`, and `python check.py` prints `ok`.
```

This is a real run against `deepseek-flash`, in a directory with a calculator, a check
that failed, and an `AGENTS.md` that says how the check is run. The model read both
files at once, changed one line and ran the check. That took four requests with 1158
input tokens and 350 output tokens, and another 7680 input tokens were answered from
the cache of the provider. The run is kept in `tests/fixtures/recorded/deepseek-fix`.

What a call may do is decided before it runs.

| `--permission` | Files | Commands |
|---|---|---|
| `read-only` | Nothing is written. | A rule or a yes is needed. |
| `workspace-write` | Files in the working directory are written, except in `.git` and `.columbina`. Nothing outside it is read or written. | A rule or a yes is needed. |
| `full-access` | Files anywhere are read and written. | They run. |

`columbina run` has nobody to ask. A command that needs a yes does not run there, and
the model is told why. A rule lets it run: `--allow "run(python check.py)"` allows this
one command and no other.

A rule is the name of a tool, or the name with a pattern: `grep`, `read(.env)`,
`write(docs/**)`. The names `read`, `write` and `run` stand for every tool that does
that, so a rule against reading a file also keeps a search from opening it. Rules that
are meant to stay go into `~/.columbina/permissions.yml`, under `deny`, `ask` and `allow`. A
project brings its own in `.columbina/permissions.yml`, but only rules that refuse or
ask: a repository that somebody cloned does not approve its own commands.

A rule about commands goes by the text of a command. One that allows counts only for
commands of a plain shape, without variables, substitutions or redirections. Such rules
are a convenience and not a boundary. Nothing confines a command yet, and whatever it
does, it does as the user. A sandbox in a container is planned.

A file is changed only after the model has read it, and only while it is still as the
model saw it.

`AGENTS.md` and `CLAUDE.md` are given to the model when a turn begins: first
`~/.columbina/AGENTS.md`, then the files from the root of the repository down to the working
directory. A link in the place of such a file is not followed.

## How it is built

- **The session log is the only source of truth.** Whatever the model is sent has been
  written to the log first, and every request is derived from the log. A log that
  stops in the middle of a turn is closed when the session is resumed, so that every
  tool call has a result.
- **Model access** has one vocabulary of messages and streamed chunks. An adapter only
  yields chunks. The runtime sees to it that every call ends with exactly one finish,
  and that every failure has a stable code. Retries are a plugin.
- **Tools** run through a pipeline with a check before the call, a wrapper around it
  and a check of its result. Whatever goes wrong becomes an error result that the model
  reads. Every call gets exactly one result, also when the turn is cancelled.
- **Files** are reached through one seam. It decides about every access, and a file is
  then opened or replaced at the place that was decided about, also when a directory on
  the way has been swapped for a link in between. Restrictions can only refuse, and
  nothing that loosens the default overrules one.
- **Permissions** are a guard of the tool pipeline, which no other plugin can skip, and
  a restriction at the seam, which holds for every file that a search opens on its way.
- **What the model is told** beside the conversation has three parts: a system prompt
  put together from sections, the instruction files, and the facts about the run. The
  last two are messages at the beginning of a turn. The beginning of a conversation
  stays as it is, so a provider can answer from its cache.
- **The agent loop** works in turns and steps. Input can be queued as a task, steered
  into a running turn, or attached as context. Four interception points let plugins
  decide whether a step begins, how a request is made, what happens after a failure,
  and whether a turn may end.
- **The kernel** underneath loads and unloads all of this as plugins.

## The kernel

`columbina.kernel` is a small asyncio plugin framework.

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

from columbina.kernel import Context, EmitEvent, Plugin, ServiceKey

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

```bash
uv sync
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run python -m coverage run -m pytest
uv run python -m coverage report
```

Coverage is run as a module because it loads a small plugin from `tests/support`, which
counts the code of one platform only where that platform runs it.

The tests need no key and reach no service outside the machine. Beside unit tests there
are these kinds:

- **Loop tests** drive the real tool pipeline and the real session files with a
  scripted model.
- **Property tests** hand the agent random tasks, steers and cancellations, and check
  what is left in the log.
- **Replay tests** play every recorded run again, with the plugins it had and with
  tools that really run, and compare the log event by event.
- **Request snapshots** keep what the model was sent as text, in
  `tests/agent_loop/snapshots`, in `tests/apps/snapshots` and beside each recording. Run
  the tests with `COLUMBINA_UPDATE_SNAPSHOTS=1` to write them anew after a change that
  was meant.

Two more things are run by hand:

```bash
uv run pytest tests/live -m live              # the real service; needs DEEPSEEK_API_KEY
uv run python tests/mutation/mutate.py loop   # put faults into the code and see whether tests notice
```

Design notes live in `docs/plans/`.

## License

MIT. See [LICENSE](LICENSE).
