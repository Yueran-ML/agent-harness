# Changelog

All notable changes are written down here. The version follows
[Semantic Versioning](https://semver.org/), and while it is below 1.0 a minor version may
change what was there before.

## 0.1.0 (not yet released)

The first release. It is the work of ten stages, and each of them ended with something that
could be run.

### The kernel and the agent

- A small asyncio kernel of plugins, with services, scopes and fibers. Everything else is a
  plugin of it, and a program is a profile: a list of plugin entries that may extend another
  profile and be patched.
- The session log: an append-only log of events that is the truth of a session, with
  projections that are folded from it. A session is taken up again after the program was
  killed, and can be copied from any of its turns.
- Models over the OpenAI Chat Completions protocol: DeepSeek, OpenAI, OpenRouter, Moonshot,
  Zhipu, SiliconFlow, and Ollama and LM Studio on the same machine, or any service by its
  address. Requests that fail for a passing reason are made again.
- The tool pipeline and the agent loop: calls are checked, asked about, run, limited and
  logged; a reply that was cut off is taken up again, and the end of every turn says in a
  code why it ended.

### Tools, permissions and the sandbox

- Six tools that read, write and edit files, find them, search them and run commands, all
  through one seam of the file system: a file is changed only after it was read, and a path
  that leads out of the working directory is asked about.
- Three presets of permissions, rules of a project and of the user, and a yes at the prompt
  for one call or for the rest of a session.
- A sandbox in a Docker container for the commands of the agent: the working directory and
  nothing else of the machine, no network by default, and what a command leaves running is
  stopped with it.

### Long conversations and extensions

- An estimate of the size of a conversation, a place for results that are too large to be
  shown whole, and a summary of the oldest part when the conversation comes near the limit
  of its model, with what the summary leaves out said to the model.
- Skills, hooks, subagents, the tools of servers that speak the Model Context Protocol, and
  plugins of one's own, with an example of a tool.

### Frontends

- A terminal interface, `columbina`, and one task at a time with `columbina run`, which can
  report the run as lines of JSON.
- `columbina serve`, a JSON-RPC service over standard input and output, and a desktop
  application on top of it.

### Measured

- A benchmark of 72 tasks that judge themselves: fixes of code, work on files, queries of a
  database and reckonings of several steps. Its tables set two models of DeepSeek against
  each other, each part of the program against the whole of it, and local models in several
  quantizations against each other.
- Line and branch coverage of 100 percent, more than 3,600 mutants that the tests have to
  notice, pyright in strict mode, and continuous integration on Windows, Linux and macOS
  with Python 3.11 and 3.12. On Linux the tests with a real Docker engine have to run.
