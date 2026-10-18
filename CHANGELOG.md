# Changelog

All notable changes are written down here. The version follows
[Semantic Versioning](https://semver.org/), and while it is below 1.0 a minor version may
change what was there before.

## Unreleased

- Files go with a message in the desktop application: chosen with the paperclip at the foot
  of the field or dropped on it, ten at most and of up to 25 MB each. When the message is
  sent they are copied into `.columbina/attachments/` of the project, which a `.gitignore`
  keeps out of git, and the message ends with a line for each file that says where it is.
  The page knows the files by an id, their names and sizes, and never by where they are.
- A package of the desktop application looks for a newer version of itself a few seconds
  after it starts and once a day after that, and asks GitHub for nothing but the list of
  releases. A button at the end of the bar of the window names a version that is out and
  opens the page of its release. The settings show the version that runs, look again when
  asked, skip a version, and stop the looking.

## 0.1.1 (2026-10-16)

- `columbina doctor` sees whether a session could start, and says what is in the way: the
  profile, the model and its service, which is asked for the list of its models with the
  key that a session would send, the shell, and for commands in containers the Docker
  engine and its image. It starts nothing that a session would leave behind, and ends with
  1 where something is in the way.
- How long a tool ran is written down beside its result in the session log, as
  `duration_ms` in its `meta`. `/tools`, `columbina sessions show` and the last line of
  `columbina run --json` say for each tool how many calls it had, how many of them failed,
  and how long it ran.
- The table of local models: twelve models of Qwen2.5 and Llama, of 3 to 8 billion
  parameters and from 2 to 16 bits, on twenty of the tasks. None passes half of its
  attempts, and `docs/bench` says where they fail. The Ollama of a run of the benchmark
  holds one request to one model, and `--ollama-port` lets two runs go side by side.
- The desktop application can be downloaded from the releases of the repository, as a zip
  for Windows on x64 and one for macOS on Apple silicon. Each brings a Python of its own
  with Columbina in it, at the versions of `uv.lock`, and starts that service, so nothing
  else needs to be installed. A tag such as `desktop-v0.1.0` has the workflow `desktop.yml`
  make both packages, try them and publish them. The opening film, its poster and the icon
  are kept in the repository for it, and are still outside the MIT license.
- A key is checked before it is stored. `columbina setup --check` asks the service for the
  list of its models with the key that it is given, stores nothing, and says what kind of
  failure it was where there is none: the network, the key, a key without the right or an
  account without money, the address, a busy service. `columbina setup --forget NAME --json`
  says what it deleted, and `columbina setup --list` names the page of each service on which
  a key is made.
- Where no variable of the environment names a proxy, the requests of Columbina to a service
  go through the proxy of the system: the Internet settings of Windows or the network
  settings of macOS. The commands that the agent runs keep their environment as it is.
- `columbina login github` signs in to GitHub with the device flow of the GitHub App of
  Columbina, Columbina-Agent, without a server of Columbina: it shows a code, opens
  `github.com/login/device` where there is a desktop, and waits until the code is entered
  there. Then it says who signed in and which repositories Columbina may reach. The tokens
  are kept in the keyring of the system apart from the keys of models, and are renewed
  before they run out; two programs that would renew them at once take turns.
  `columbina login --status` says who is signed in, and `columbina logout github` deletes
  the sign-in. With `--json` each answers in lines of JSON for another program, which stops
  a sign-in by closing its standard input.
- The window for the key in the desktop application checks a service before anything is
  stored, offers the models that it lists, and says what to look at where the check fails.
  A service on this computer is checked as soon as it is chosen. The window marks a model
  that looks like a key and a key with a blank inside it, links to the page on which a key
  is made, and deletes a stored key once it has asked whether to.
- The agent acts on GitHub in the name of whoever signed in, in the repositories that were
  chosen for the app. `github_repos`, `github_read` and `github_checks` read repositories,
  issues, pull requests and their checks without a question. `github_clone` clones into the
  working directory, checked as writing a file there is. `github_push`,
  `github_open_pull_request` and `github_comment` show in full what they would do, and are
  asked about every time. A push never goes to the default branch and is never forced. git
  gets the token for one run, as a header of its requests to GitHub, and what comes from
  GitHub is put before the model as material and not as instructions.
- The window fits each system. On macOS the buttons of the window stand at the left end of
  its bar and the bar gives their room back in full screen, shortcuts are written with ⌘,
  sheets come down from under the bar, and the menu copies, pastes, closes and leaves. A
  package asks the login shell of the user for its `PATH`, so that the commands of the agent
  find what is installed with Homebrew, and for the variables of its proxy, which go where
  the application was given none.
- The first page of the desktop application goes through the steps before the work: a model
  service with its key, GitHub if the user likes, and the project. Once they are done or
  left for later they fold into one line, and they come back where a key was deleted. The
  first page and the settings sign in to GitHub: the code is shown large with a clock, and
  the settings show who is signed in, the repositories that Columbina may reach, and a
  sign-out that is asked first. A call on GitHub says in the conversation what it does
  there, and a question about one says what Columbina wants to do on GitHub.
- Where another service, address or model is stored and nothing is under way, the desktop
  application starts its service again at once, at the task that is open.
- A model that is named with `/model` during a run no longer takes the context window of the
  model that the run started with. Nothing is known of the window of another model, as where
  a run starts with it, and naming the first model again gives its window back. The share of
  the window that the terminal and the desktop application show, and when a conversation is
  made shorter, follow the model that was named.
- The field of the desktop application has a button at its foot with the model in use, and
  the settings have it where the model is set. It opens the models that the service in use
  lists, with a filter over them, and a name that the service does not list can be written
  there too. The model that is chosen is used from the next request on, and is stored as
  the model of its service for the next start. The window asks a service for its models with
  the key that a session would send, so the page never has the key.

## 0.1.0 (2026-10-15)

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
  quantizations against each other. On it `deepseek-flash` passed all 216 attempts at the
  tasks, three at each, and `deepseek-v4-pro` 214.
- Line and branch coverage of 100 percent, more than 3,600 mutants that the tests have to
  notice, pyright in strict mode, and continuous integration on Windows, Linux and macOS
  with Python 3.11 and 3.12. On Linux the tests with a real Docker engine have to run.
