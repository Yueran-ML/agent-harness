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

<p align="center">
  <img src="docs/assets/demo.gif" width="100%"
       alt="Columbina at the terminal: the agent reads check.py and calc.py, mends calc.py, asks before it runs python check.py, and says what was wrong.">
</p>
<p align="center">
  <sub>A run of DeepSeek, played again at the terminal: the replies are those of a recorded run, and the tools ran for real. <a href="docs/assets/README.md#the-demo">How it was made</a></sub>
</p>

**Status:** early development. Nine of ten stages are done, and Columbina is on PyPI. Of
the last, the service for other programs is built, and the desktop app on top of it is under way. `columbina` holds a
conversation with an agent at the terminal, and `columbina run` gives it one task and
prints the answer. The agent talks to a model over the OpenAI Chat Completions protocol,
DeepSeek by default, and to Claude over the Messages API of Anthropic. A conversation that
outgrows its model is made shorter by itself,
and a session can be taken up again after the program was killed. Skills, hooks, subagents
and the tools of MCP servers add to the agent without a change to the program. With
Docker, the commands of the agent run in a container that has the working directory and no
network.

## Download and set up

### The desktop application

The desktop application holds a conversation with Columbina in a window. It brings a Python
and the service of its own, so nothing else needs to be installed, and it is downloaded from
the [releases](https://github.com/Yueran-ML/agent-harness/releases) of this repository:

- On Windows 10 or 11 on x64, unpack `Columbina-…-windows-x64.zip` and open `Columbina.exe`.
  The application is not signed, so SmartScreen asks once: choose *More info*, then
  *Run anyway*.
- On macOS 12 or later on Apple silicon, unpack `Columbina-…-macos-arm64.zip` and move
  `Columbina.app` into *Applications*. It is not signed with a certificate of Apple, and
  macOS opens it once this command has been run in a terminal:

```bash
xattr -dr com.apple.quarantine /Applications/Columbina.app
```

The first page leads to a service of models and its key, to GitHub if you like, and to a
project folder. [desktop/README.md](desktop/README.md) says what the window does.

### The command line

Columbina needs Python 3.11 or newer, and is installed from
[PyPI](https://pypi.org/project/columbina/):

```bash
pip install columbina
```

With [uv](https://docs.astral.sh/uv/), `uv tool install columbina` installs the command apart
from other projects, and `uv tool upgrade columbina` brings it up to date later. The tools of
servers that speak the Model Context Protocol need a few more packages, which
`pip install "columbina[mcp]"` brings along.

A model comes from a service, and a service wants an API key. Columbina asks for both:

```bash
columbina setup
```

It asks which service the model is to come from, which model to use, and for the key.
DeepSeek, OpenAI, Anthropic (Claude), OpenRouter, Moonshot, Zhipu and SiliconFlow are known
by name, and so are Ollama and LM Studio on the same machine. Claude is spoken to in the
protocol of Anthropic itself, which keeps the beginning of a conversation in a cache and
tells what the model thought. Any other service that speaks the OpenAI Chat Completions
protocol is added by its address. The key is typed without being shown. It is
stored in the keyring of the system, such as the Credential Manager of Windows or the
Keychain of macOS, and in a file in `~/.columbina` where the system has none.

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

`columbina doctor` sees whether all of this is in place. It asks the service for its models
with the key, looks for the shell and the sandbox, and says in a line each what would be in
the way of a session.

A conversation at the terminal begins with this command, and the agent works in the
directory that it is given in:

```bash
columbina
```

A task can also be given on the command line, and the answer is printed:

```bash
columbina run "What does this project do? Answer in three sentences."
```

`--cwd DIR` names another directory for the agent to work in.

Signed in to GitHub, the agent also works on the repositories that you choose for it there.
It reads issues, pull requests and their checks, clones, pushes a branch, opens a pull
request and comments, and it asks you before every push, pull request and comment.

```bash
columbina login github
```

### An assistant in Feishu

Columbina can also answer you in [Feishu](https://www.feishu.cn) or Lark, through a custom app
of your own that only you talk to. It reads the documents that you share with the app and the
files of text that you send it, and takes a skill that is sent as a `SKILL.md`. It runs no
commands and changes none of your files: what you send it is kept in a folder of its own,
`~/Columbina/feishu`, and you may let it change the files in that folder. In the desktop
application it is set up under *Settings*, *Assistant in Feishu*, which also lists what to do in
the developer console of Feishu. While it runs, closing the window keeps the application in the
tray, and on Windows the application can start in the tray when you sign in. At the command
line, `columbina feishu --setup` takes the app and `columbina feishu` runs the assistant. The
package of its connection comes with `pip install "columbina[feishu]"`.

## Documentation

- [The engineering report](docs/report.md) tells how Columbina is put together, which choices
  shaped it, how it is tested and how well it works ([in Chinese](docs/report.zh-CN.md)).
- [The tables of the benchmark](docs/bench/README.md) give its results with two models of
  DeepSeek, part by part, and what they cost.

## License

MIT, for the code and the documents. See [LICENSE](LICENSE).
