# Columbina

**A plugin-based agent harness, written from scratch in Python.**

Columbina runs an agent against a language model and lets it work in a directory of yours.
The agent reads, searches and changes files and runs commands, and what a call may do is
decided before it runs. Everything that Columbina is made of is a plugin of a small asyncio
kernel: the model adapter, the tool registry, the session log, and the agent loop itself.

- A conversation at the terminal, one task from the command line, and a JSON-RPC service
  for other programs, such as its desktop app.
- Any model service that speaks the OpenAI Chat Completions protocol; DeepSeek by default,
  and Ollama and LM Studio on the same machine.
- Long conversations are made shorter by themselves, and a session can be taken up again
  after the program was killed.
- Skills, hooks, subagents and the tools of MCP servers add to the agent without a change
  to the program.
- With Docker, the commands of the agent run in a container that has the working directory
  and no network.

## Install

```bash
pip install columbina          # or: uv tool install columbina
pip install "columbina[mcp]"   # with the tools of MCP servers
```

Columbina needs Python 3.11 or newer.

## First steps

```bash
columbina setup                 # choose a service and a model, and store the key
columbina                       # a conversation at the terminal
columbina run "What does this project do? Answer in three sentences."
```

## More

The [repository](https://github.com/Yueran-ML/agent-harness) has the source, the
[guide](https://github.com/Yueran-ML/agent-harness/blob/main/docs/guide.md) (also in
[简体中文](https://github.com/Yueran-ML/agent-harness/blob/main/docs/guide.zh-CN.md) and
[日本語](https://github.com/Yueran-ML/agent-harness/blob/main/docs/guide.ja.md)), the
[engineering report](https://github.com/Yueran-ML/agent-harness/blob/main/docs/report.md), and
the [tables of the benchmark](https://github.com/Yueran-ML/agent-harness/blob/main/docs/bench/README.md)
that it is measured with.

Columbina is released under the MIT license.
