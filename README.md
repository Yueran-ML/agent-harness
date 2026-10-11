<p align="center">
  <img src="docs/assets/columbina-wings.png" width="100%"
       alt="A winged figure floats before a full moon, above ruins that stand in still water.">
</p>
<p align="center">
  <sub>Fan art, made with an AI image generator. <a href="#the-character-behind-the-name">About the pictures</a></sub>
</p>

<h1 align="center">Columbina</h1>

<p align="center">
  <strong>A plugin-based agent harness, written from scratch in Python.</strong>
</p>

<p align="center">
  <b>English</b> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="https://github.com/Yueran-ML/agent-harness/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Yueran-ML/agent-harness/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Python 3.11 and 3.12" src="https://img.shields.io/badge/python-3.11%20%7C%203.12-8b7fd6?labelColor=1b1b2f&logo=python&logoColor=white">
  <img alt="Coverage: 100%" src="https://img.shields.io/badge/coverage-100%25-8b7fd6?labelColor=1b1b2f">
  <img alt="Types: pyright, strict" src="https://img.shields.io/badge/types-pyright%20strict-8b7fd6?labelColor=1b1b2f">
  <img alt="Lint: ruff" src="https://img.shields.io/badge/lint-ruff-8b7fd6?labelColor=1b1b2f">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-8b7fd6?labelColor=1b1b2f">
</p>

<p align="center">
  <a href="#the-character-behind-the-name">The name</a> ·
  <a href="#what-it-is">What it is</a> ·
  <a href="#download-and-set-up">Download and set up</a> ·
  <a href="#documentation">Documentation</a>
</p>

## The character behind the name

The project is named after Columbina, a character of *Genshin Impact*. She is introduced
as the third of the Eleven Fatui Harbingers and goes by the name Damselette. Like the
other Harbingers she is named after a figure of the Italian commedia dell'arte, in her
case Colombina. Her story is set in Nod-Krai and has to do with the moon, which is why
there is a moon in every picture here. She has been a playable character since early in
2026, a 5-star Hydro character who fights with a catalyst.

The pictures show her as a fan sees her. The author made them with an AI image generator,
ChatGPT, and they are not official artwork. Each file is kept as the generator wrote it,
with the Content Credentials that say how it was made.

<p align="center">
  <img src="docs/assets/columbina-ruins.png" width="100%"
       alt="The same figure sits among ruins by the water, with a small light between her hands.">
</p>

<details>
<summary>A storyboard</summary>
<br>
<p align="center">
  <img src="docs/assets/columbina-storyboard.png" width="100%"
       alt="Twelve small scenes of the same figure, with the name Columbina and a mark for it.">
</p>

The words in this picture belong to the picture. They are no list of what the program
does.

</details>

If the pictures make you curious about her, you are welcome to meet her in the game.
*Genshin Impact* is free to play. Its official site is
[genshin.hoyoverse.com](https://genshin.hoyoverse.com/), and the one for mainland China is
[ys.mihoyo.com](https://ys.mihoyo.com/).

*Genshin Impact* and its characters belong to HoYoverse (miHoYo). This project is the work
of a fan. It is not affiliated with HoYoverse and not endorsed by it. The pictures are not
part of the software, and the license below does not cover them. See
[`docs/assets`](docs/assets/README.md).

## What it is

Columbina runs an agent against a language model and lets it work in a directory of
yours. The agent reads, searches and changes files and runs commands, and what a call may
do is decided before it runs. Everything that Columbina is made of is a plugin of a small
asyncio kernel: the model adapter, the tool registry, the session log, and the agent loop
itself.

**Status:** early development, in the sixth of ten stages. `columbina` holds a
conversation with an agent at the terminal, and `columbina run` gives it one task and
prints the answer. The agent talks to a model over the OpenAI Chat Completions protocol,
DeepSeek by default. A conversation that outgrows its model is made shorter by itself,
and a session can be taken up again after the program was killed.

## Download and set up

Columbina needs Python 3.11 or newer and [uv](https://docs.astral.sh/uv/). It is not on PyPI
yet, so it is fetched with git, as a clone of this repository.

```bash
git clone https://github.com/Yueran-ML/agent-harness.git
cd agent-harness
uv sync
```

`uv sync` makes a virtual environment inside the clone and installs into it what Columbina
needs.

A model comes from a service, and a service wants an API key. Columbina asks for both, in
the clone:

```bash
uv run columbina setup
```

It asks which service the model is to come from, which model to use, and for the key.
DeepSeek, OpenAI, OpenRouter, Moonshot, Zhipu and SiliconFlow are known by name, and so are
Ollama and LM Studio on the same machine. Any other service that speaks the OpenAI Chat
Completions protocol is added by its address. The key is typed without being shown, and it
is stored in `~/.columbina/credentials.json`. The
[guide](docs/guide.md#services-and-keys) says how it is kept.

A key can be given in an environment variable instead, and the variable goes before a
stored key. Where nothing was chosen, Columbina talks to DeepSeek and reads
`DEEPSEEK_API_KEY`. In PowerShell on Windows:

```powershell
$env:DEEPSEEK_API_KEY = "your-key"
```

In a shell on Linux or macOS:

```bash
export DEEPSEEK_API_KEY="your-key"
```

Set like this, the variable lasts until that window is closed. A key is read when a request
is made. It is never part of a profile or of a stored session, and it is cut out of every
error message before the message is shown or stored.

A conversation at the terminal begins with this command, given in the clone:

```bash
uv run columbina
```

A task can also be given on the command line, and the answer is printed:

```bash
uv run columbina run "What does this project do? Answer in three sentences."
```

Started like this, the agent works in the clone itself, and `--cwd DIR` names another
directory for it to work in. To have the command in every directory, install it once from
the clone:

```bash
uv tool install .
```

After that, `columbina` starts a conversation in whatever directory it is called from. If
the shell does not find the command, `uv tool update-shell` puts the directory that uv
installs commands into on the path. After a later `git pull`, `uv tool install . --reinstall`
brings the command up to date.

## Documentation

- [The guide](docs/guide.md) tells the rest: the terminal and its commands, long
  conversations, running one task, services and keys, permissions, how Columbina is built,
  how it is tested, and where the work stands.
- [The design document](docs/plans/2026-10-09-columbina-design.md) is in Chinese. The plans
  of the stages are beside it.
- [`docs/research`](docs/research) holds what was learned from the harnesses that
  Columbina takes after.
- [`docs/protocol`](docs/protocol) describes the messages that the desktop app will
  exchange with the program.

## License

MIT, for the code and the documents. See [LICENSE](LICENSE).
