# Columbina: an engineering report

**English** · [简体中文](report.zh-CN.md)

Columbina is an agent harness written from scratch in Python: it runs a language model as an
agent that reads, searches and changes the files of a directory and runs commands there, and
it decides about every call before the call runs. It was built in ten stages, and this report
tells how it is put together, which choices shaped it and what they cost, how it is tested,
and how well it works, as a benchmark of 72 tasks that judge themselves measured it.

In short: on the acceptance benchmark `deepseek-flash` passed all 216 attempts and
`deepseek-v4-pro` 214; Columbina composes itself in under a second and takes up a session of
a thousand steps in a fifth of one; and more than 9,700 tests cover every line and branch,
while more than 3,600 mutants written by hand see that the tests notice faults.

## 1. A kernel of plugins

Everything that Columbina is made of is a plugin of a small asyncio kernel: the model
adapter, the tools, the session log, the agent loop, the terminal. The kernel knows nothing
of agents. It offers four things.

- **Services.** A plugin offers a service with `provide` and reads one with `use`. It names
  the services it needs in `inject`, and it is loaded once all of them are there, unloaded
  when one goes away, and loaded again when it comes back.
- **Effects.** Whatever a plugin registers through its context, a listener, a tool, a
  service, is undone when the plugin unloads, in reverse order. A plugin does not clean up
  after itself, and so cannot forget to.
- **Events** in five kinds of dispatch: to every listener at once, in parallel, one after
  another, until one answers, and through all of them in turn as a chain. The checks of a
  tool call are such a chain, and so is the decision whether a step begins.
- **Composition.** A program is a profile, a list of plugin entries in YAML. A profile can
  extend another, and a layer changes an entry by its id. The program that runs one task
  without a person is the base profile and one entry more; a test or a benchmark changes a
  program with a patch of a few lines.

What this costs is indirection. A reader who wants to know what happens to a tool call
follows events through several plugins, and the order of their listeners matters. Two
things make that bearable. The checks of a call can refuse and never let through what
another refused, so the order of the guards does not decide what is allowed. And the
composed program can always be printed (`columbina run --dump-config`), so the plugins that
are loaded are never a matter of guessing.

## 2. The session log is the only truth

A session is one file of events that are only ever appended. Whatever the model is sent was
written to the log first, and every request is derived from the log. Three things follow.

- A session that the program died in the middle of is taken up again: the log is read, a
  turn that was cut off is closed so that every tool call has a result, and the next
  request is made from what is there.
- A session can be copied from any of its turns, and a copy shares nothing with the
  original but the past.
- Shortening a conversation loses nothing. A summary, or a result that was cut short,
  takes the place of older messages in the view that the model is sent, and the events
  themselves stay. Whatever shows a session can show all of it.

The price is that state is never simply changed. The list of things to do, the inbox of
messages that arrive while a turn runs, the count of tokens: each is a projection, folded
from the events. A new kind of state needs a new event and a fold, and the fold must give
the same answer when the log is read again from the start. The tests fold a session that was
taken up again from its first event, and build what the model is sent from the events alone.

## 3. The loop and the pipeline of a call

The agent works in turns, and a turn in steps: a request to the model, then the calls of
tools that the reply asked for. Input reaches it in three ways: as a task, as words steered
into a turn that is running, and as context for the next one. Four points let plugins decide
whether a step begins, how a request is made, what happens after a failure, and whether a
turn that is about to end goes on. The budget of a turn, its steps, its tokens and its time,
is such a plugin, and so is the compaction of a long conversation.

A call goes through a pipeline: the arguments are checked against the schema of the tool,
the guards are asked, the person is asked where a rule says so, the tool runs inside a
wrapper, and its result is checked before the model reads it. Whatever goes wrong becomes a
result that the model reads, with a stable code. Every call gets exactly one result, also
when the turn is cancelled while it runs, so a log never holds a call without an answer.

The model is reached over the OpenAI Chat Completions protocol alone. One adapter talks to
six services known by name, DeepSeek among them, to Ollama and LM Studio on the same machine,
and to any other service by its address. An adapter only turns a stream into chunks of one
vocabulary; that every call ends with exactly one finish, and that every failure has a code,
is seen to around it. Retries
are a plugin.

## 4. One seam for files, and a container for commands

Six tools reach the machine: read, write, edit, find, search and run a command. The five
that touch files reach them through one seam. A tool asks the seam to locate a path and to
decide about the access, and gets a permit; the file is then opened or replaced through the
permit, at the place that was decided about. A directory on the way that is swapped for a
link between the decision and the access does not move the access. The seam also remembers
which files the model was shown and in what state, and a file is replaced only if the model
saw it as it is now: what it has not read it would overwrite blindly.

Restrictions at the seam can only refuse, and nothing that loosens the default overrules
one. The permissions are such a restriction, and also a guard of the pipeline that no other
plugin can skip. Three presets set what is allowed by default, and rules of a project and of
the user add to them.

Commands run in a sandbox. One provider runs them on the machine. The other keeps a Docker
container for each session, with the working directory mounted and nothing else of the
machine, no network unless the config gives one, no capabilities and a limit on processes.
The container lives exactly as long as the input stream of the process that started it, so
a harness that is killed takes its containers with it, without any record kept to clean up
later. A command that the container fully holds is not asked about; one that it holds only
in part is.

## 5. Choices and what they cost

| Choice | What it buys | What it costs |
|---|---|---|
| Everything is a plugin | Any part can be left out, replaced or switched off by a line of YAML; the benchmark switches parts off this way | Indirection; the order of listeners has to be thought about |
| The log is the only truth | A session survives a crash, can be copied, played again and shown whole | Every new kind of state needs an event and a fold |
| One protocol for every model | One adapter, tested once, for eight services known by name and any other | What a service offers beyond the protocol is not used |
| One seam for files | One place decides, and what is touched is what was decided about | Every tool that touches a file has to go through it, the examples included |
| A container per session | Commands are held without a native sandbox of each system | Docker is needed for it; without it commands run on the machine and are asked about |
| Tests that drive the real pipeline | Faults show where they would show to a user | The suite is large and takes minutes |

Some things were left out on purpose: native sandboxes of each operating system, agents that
stay running, workflows, and anything that sends a session to a provider beyond the request
itself.

## 6. How it is tested

The suite has more than 9,700 tests, and they cover every line and every branch of the
program and of the benchmark: 100 percent, held in the checks of every change. Beside unit
tests there are these kinds:

- **Loop tests** drive the real pipeline and the real session files with a scripted model,
  which answers from a list.
- **Property tests** hand the agent random tasks, steers and cancellations, and check what
  is left in the log.
- **Replay tests** play recorded runs of a real model again, with tools that really run, and
  compare the log event by event.
- **Screen tests** run the terminal on an emulated terminal and read what stands on the
  screen.
- **Engine tests** start real containers and look at what a command in one can reach and
  what is left of it afterwards.

Coverage says that a line ran, not that a test would notice it doing the wrong thing. So
more than 3,600 mutants, small faults written by hand, are put into the code one at a time,
and a test has to fail for each; a mutant that no test notices is a test that is missing.
The types are checked by pyright in strict mode, for Windows and for Linux. CI runs all of
this on Windows, Linux and macOS with Python 3.11 and 3.12, and on Linux the tests with a
real Docker engine have to run rather than be passed over.

## 7. The benchmark

### 7.1 Tasks that judge themselves

The benchmark has 72 tasks in four categories. Each has a working directory to begin with,
what a right answer looks like, and a way to judge the work that the model never sees.

| Category | Tasks | What is asked | How it is judged |
|---|---|---|---|
| code | 15 | A small Python project has a fault that a test shows. Fix it without changing the tests | Hidden tests, run in a container without a network |
| files | 12 | Sort, rename, merge, change configs, split, make an index | Rules on the tree that is left |
| sql | 15 | A question about the database of a shop, reached only through three tools | The submitted query runs on the database and on a copy with other rows, and must answer rightly on both |
| compute | 30 | A reckoning of three to six steps, every number behind a tool | The answer, and whether every number was looked up |

Two of the categories are built so that a model cannot pass by guessing. A query of SQL
that writes the numbers of the database into itself fails on the copy with other rows. A
reckoning names its numbers in brackets, and an answer counts only when each was looked up.
A check that runs with the tests sees, for every task, that its start fails and its
reference passes.

### 7.2 An attempt

An attempt runs `columbina run` in a process of its own, in a fresh copy of the working
directory, with a home of its own. It has what its task gives it and nothing of the machine:
none of the user's skills or agents and none of the variables that set Columbina, and a run
must lie outside any repository, whose instructions every attempt would otherwise read. The
commands of the model run in a container. A line of results is written as soon as an attempt
is judged, so a run that is broken off is taken up by running it again.

A rate is given with its interval of Wilson at 95 percent, and the difference of a part with
the interval of Newcombe; the tests hold both to the examples that Newcombe published.

### 7.3 Results

**Acceptance.** Each task three times with each of the two models of DeepSeek, the whole
program on. The rates are given with their intervals of Wilson; the tables with the tokens
of each row are in [`docs/bench`](bench/README.md).

| Model | code | files | sql | compute | All |
|---|---|---|---|---|---|
| `deepseek-flash` | 45/45 | 36/36 | 45/45 | 90/90 | 216/216 = 100% [98%, 100%] |
| `deepseek-v4-pro` | 43/45 | 36/36 | 45/45 | 90/90 | 214/216 = 99% [97%, 100%] |

`deepseek-flash` passed every attempt. The two that `deepseek-v4-pro` failed are at one task,
where its fix split the words of a text at every letter outside English. The design makes
this the floor of every release: a release is run on `deepseek-flash`, and its rate may not
fall below that of the one before.

**The parts of the program.** Each of four parts switched off in turn, with `deepseek-flash`
told that its window is 16,384 tokens, the window of the local models:

| Switched off | Passed | Input tokens of a median attempt |
|---|---|---|
| nothing | 215/216 | 16,837 |
| the compaction | 216/216 | 16,909 |
| the list of things to do | 216/216 | 15,582 |
| the limit on the size of a result | 216/216 | 16,867 |
| the rules of conduct | 216/216 | 11,382 |

No part changes whether the model succeeds at these tasks, and each shows what it costs.
The rules of conduct are about a third of the input of a median attempt and the list of
things to do about 7 percent, most of it read from the cache of the service at a fiftieth of
the price. The compaction never started, and results were cut short in one attempt of
1,080: these tasks keep their conversations small, so the two parts that keep conversations
small had nothing to do, and cost nothing. All four stay on by default. The rules of conduct
guard against what these tasks do not hold, an act that cannot be undone or an instruction
hidden in a file, and tasks of that kind are the next to be added.

### 7.4 Cost and speed

The three runs, 1,512 attempts, cost 1.67 US dollars at the off-peak prices of DeepSeek. A
median attempt of `deepseek-flash` puts in about 17,000 tokens, 94 percent of them read from
the cache, and gives out about 600, for less than a tenth of a cent.

Columbina itself is quick. On the machine of the author, composing the program takes 0.96 s
and one task with a scripted model 1.14 s; a session of 1,000 steps, 3.9 MB on disk, is
taken up again in 0.18 s with 21 MB of memory at the peak, and its tokens are added up in
9 ms. Tests hold these to loose budgets in CI.

## 8. What is next

- **The table of local models.** Qwen2.5 and Llama in their quantizations, on twenty of the
  tasks, with the window of 16,384 tokens at which the parts that keep a conversation small
  come into play. Its runs need the graphics card of the machine to themselves.
- **Tasks for the rules of conduct**: an act that cannot be undone, an instruction hidden in
  a file that the agent reads.
- **The desktop app**, and a study of how usable it is.
- **PyPI.** The workflow that builds, tries and publishes the package is ready, and a tag
  starts it.
