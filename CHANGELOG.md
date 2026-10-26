# Changelog

All notable changes are written down here. The version follows
[Semantic Versioning](https://semver.org/), and while it is below 1.0 a minor version may
change what was there before.

## Unreleased

- The agent reads the web. `web_fetch` takes a page and gives the model what a reader sees of it,
  in the shape of Markdown, and plain text, JSON or XML as it stands. `web_search` searches the
  web and gives the titles, addresses and a few lines of the results: by DuckDuckGo, which needs
  no key, or by Tavily or Brave, whose key is kept with `columbina search-key` or given in
  `TAVILY_API_KEY` or `BRAVE_API_KEY`. Both only read and run without a question unless a rule
  asks for one, and the model is told before every page that it is material and not
  instructions. A place of this machine or of its own networks is not asked for a page, nor is
  one that a page redirects to. Every program loads the plugin `columbina.web`.
- A call that is made again in a turn, with the same tool and the same arguments and nothing
  written or run in between, gives what it gave before. The plugin `columbina.guard.repeats`
  tells the model so beside the result the third time, and the fifth time the call does not run,
  with the code `REPEATED_CALL`. Every program loads it.
- Fewer calls go wrong for reasons that can be avoided. The model is told that only the tools
  it is given are there, and that a program such as Python is run with a command; and that a
  file is changed only after it was read with the tool that reads, where the lines around the
  place to change are enough. A write or an edit of a file that was not read says the same.
- On Windows the cmdlets that read files, `Get-Content`, `Select-String` and `Import-Csv`, read
  UTF-8 in the commands of the agent and of hooks, and Python opens files in UTF-8 there unless
  `PYTHONUTF8` says otherwise. A file of UTF-8 in Chinese no longer comes out garbled.

## 0.2.0 (2026-10-25)

- The desktop application can show a pet over the other windows that follows the task that is
  open: it thinks, reads while a tool runs, waits with the user for a yes, waves when a turn is
  done and shows a failure. A click brings the window back, and a drag moves it. Three pets of
  pixel art come with it, drawn by a script of the repository. A pet of the user's own is a
  folder with `pet.json` and a sheet of frames in the layout of the pets of Codex. docs/pets.md
  says how.
- The plugin `columbina.pets` writes what a conversation is doing to a small file for a pet:
  thinking, a tool that runs, a question for the user, done or failed, with the name of the tool
  and nothing of the conversation. Every program loads it, and it writes only where
  `COLUMBINA_PET_STATE` names the file, as the desktop application does for its service. The pet
  of the desktop application reads that file, and so can a pet program of somebody else.
- What the user says while a turn runs can be read at once, with Ctrl+Enter in the desktop
  application and with `now` in `session/steer`: the step that runs is cut short, and the turn
  goes on with what was said, its steps counted on. What had arrived of the reply is kept and
  shown as turned away from; tools that only read are stopped, tools that change something or
  run a program finish first, and calls that had not begun are not carried out. `Agent.steer`
  takes `now`, and `Agent.cut_short` cuts the step alone.
- The steps of a turn, unfolded in the desktop application, are no longer drawn as the first
  steps of the first page, with their frame and their entrance, which left them unseen where the
  entrance did not run.
- Reasoning that DeepSeek was writing when its reply was cut short goes back to it with the
  next request, as far as it had arrived, and a reply without text or a tool call goes back
  with empty content: a request with tools needs the reasoning of every reply before it.
- Each assistant, in Feishu, in WeCom and in QQ, may ask a model of its own and think as hard as
  its owner chose for it, apart from the others: a chat in QQ quick, work in Feishu careful.
  What is left empty follows what holds for all, and the service and its key stay the same for
  all. The settings of the desktop application choose them in the part of each assistant, which
  starts again for a change, and the command line takes `--effort` beside `--model`. Another
  program changes them with a document of JSON that holds `model` and `effort` alone.
- An assistant in QQ answers through a bot of the open platform of QQ whose events come by
  WebSocket, so that it needs no address on the internet. `columbina qq --setup` checks the
  AppID and the AppSecret with QQ and keeps them, the AppSecret in the keyring of the system,
  and `columbina qq` runs the assistant. In a chat of two it answers the one who sent the code
  of pairing; in a group it answers whoever mentions the bot, in a conversation for each group
  with a folder of its own. It is for chatting: the model thinks not at all, whatever was
  chosen for the service, its agents change no file and run no command, the agent of a group
  has no tool, and none is given the instructions that the user keeps for their own work. An
  answer goes out as QQ takes answers: within five minutes of a message in a group and an hour
  in a chat of two, in a few messages at most. The package comes with
  `pip install "columbina[qq]"`, and the settings of the desktop application have a part for it
  with a window of its own that takes the AppID and the AppSecret.
- The assistant in WeCom answers in groups as well: whoever mentions the bot there, owner or
  not. Each group has a conversation of its own, taken up again after a new start, and a folder
  of its own in `groups`. Its agent is told who wrote each message, reads the files of that
  folder alone, and changes nothing. It is given none of the owner's files or skills, and not
  the instructions that the user keeps for their own work: an agent whose spec says that
  `strangers` talk with it is given neither.
- A turn whose steps are limited tells the model when five are left and when the last one
  begins, so that it keeps what it found and answers in time. A subagent that is stopped
  before it has finished hands back, beside what it was writing, the files that its tools
  changed and the last two things it wrote before.
- `columbina balance` says how much money the account of a service has left, as DeepSeek tells
  it, and `--json` says it to another program. The settings of the desktop application show it
  for the service in use, with a button to ask again. At another address than its own, a
  service is not asked.
- Instructions that change in the middle of a conversation, such as an instruction file, the
  skills, the list of things to do or the system prompt that ultra adds to, no longer take the
  place of the old text at the beginning of the conversation. The new text follows the
  conversation and says that it holds instead. The beginning stays as it was, so a service
  reads it from its cache, and Claude no longer refuses the thinking it did before. Requests to
  Anthropic itself ask it to leave out thinking whose conversation changed, as a summary
  changes it, rather than refuse them. A refusal of that kind has the code `THINKING_REFUSED`,
  and the desktop application says that a new conversation goes on.
- `columbina reasoning-language` chooses the language that the model reasons in, from the
  next turn on and in every conversation: `zh`, `en`, `ja`, or `none`. The model is told in a
  message of its own, which follows the conversation when the choice changes. The settings of
  the desktop application offer the language of the interface, Chinese, English, or none.
- An assistant in WeCom answers its owner through a smart bot that uses the API mode with a long
  connection, as the one in Feishu answers through an app. `columbina wecom --setup` checks the
  Bot ID and the Secret with WeCom and keeps them, the Secret in the keyring of the system, and
  `columbina wecom` runs the assistant, which pairs with the one who sends the code that it
  shows. It answers in a single chat alone, reads text, what WeCom writes out of a voice
  message, quotes and files, which it decrypts, and works in `~/Columbina/wecom`, whose files
  its owner may let it change, as with the one in Feishu. Its answer streams into the chat while
  the turn runs. The package comes with `pip install "columbina[wecom]"`.
- The settings of the desktop application have a part for the assistant in WeCom beside the one
  in Feishu, and a window of its own takes the Bot ID and the Secret. Both assistants can run at
  once, each starts with the application where it ran when the application was left, and while
  either runs, closing the window keeps the application in the tray.
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
- `/effort` chooses how hard the model thinks, in the terminal and in the service, from the
  next request on. A service of the style of DeepSeek offers default, off, low, high and max,
  and is sent `reasoning_effort` with `thinking`, over what the options of the profile say of
  either. A service of the style of OpenAI offers the level that its profile asks for, and is
  asked for it until another level is chosen. Every service offers ultra. Ultra asks for the
  most reasoning there is and has the agent plan, hand the parts of the work that stand alone
  to subagents that work side by side, and have one of them review the result. The status of a
  session tells of the level and of those on offer. `columbina setup --from-stdin` keeps a
  level for a service from a document that says nothing else, and leaves the service in use
  and all else as it is; ultra is not kept. A level that is kept holds for the service that
  the profile names where no service is chosen, and not for another service that the command
  line names. The desktop application offers the levels on a button beside the model.
- A model or a level that is named during a run holds for the subagents that are made from then
  on, as it does for the agent of the conversation, and for a session that is opened while it
  is named.
- The desktop application writes what is chosen through the setup program one write at a time,
  so that a model and a level that are chosen one after the other are both kept. Where a model
  or a level that is chosen in the settings is in use and not kept, the settings say so.
- The line under the field of the desktop application says how far the task has come: its turns
  and steps, how many tokens the model wrote a second on average, and the tokens of all its
  requests with what came from the cache. A task can be shown as its trace, with a switch in the
  bar of the window or Ctrl+J: its steps side by side, and turn by turn what was said, each answer
  with its time and tokens, and each call with its arguments, result and time. Rows can be left
  out by their kind or looked through, only what went wrong can be shown, and a row opens to
  show all that it holds.
- In the desktop application the panel of the model and that of the levels open below their
  button where there is too little room above it and more below, and the other way round. They
  are no higher than the room where they open, and their lists scroll, so that the head of a
  panel is never cut off.
- `columbina feishu` is an assistant that its owner talks to in Feishu or Lark, through a
  custom app of the owner's own. It holds the long connection of the app, so nothing on the
  computer is reached from outside, and it answers one person: whoever first sends the code of
  pairing that it shows. Each message is a turn of a conversation that is taken up again when
  the assistant starts again, and the answer is a reply to the message, shown as Markdown and
  sent in parts where it is long. `/new`, `/stop`, `/skills` and `/help` are done by the
  assistant itself. A `SKILL.md` that is sent becomes a skill of the assistant, and another
  file of text is kept for the agent to read. The agent reads the documents, pages of a wiki
  and spreadsheets of Feishu that are shared with the app, and changes and runs nothing. The
  App Secret is kept in the keyring of the system. `columbina feishu --setup` takes the app at
  a terminal and `--from-stdin` from another program, `--status` says how the assistant is
  doing, and `--forget` deletes the app; an app is kept or deleted only while no assistant
  runs. An app of Lark that is said to be one of Feishu, or the other way round, is not kept,
  and is said to be so: either platform gives a token to an app of the other, and only the
  address of a connection tells them apart. `--language` chooses whether the assistant says
  what it says by itself, such as its help, in Chinese or in English. The package of the
  connection comes with `columbina[feishu]`.
- The settings of the desktop application have a part for the assistant in Feishu. The app and
  its App Secret are given in a window of their own, which lists what to do in the developer
  console, and Feishu is asked whether it takes the app before it is stored. Where the app is
  one of the other platform, the window says so and chooses that one at a click. The assistant
  is started and stopped there and shows its code of pairing, and says what it says by itself
  in the language of the application. It runs while the application is open, and starts again
  with the application where it ran when the application was closed.
- Anthropic is a service that is known by name, as `anthropic`, with `claude-opus-5-5` and a
  window of a million tokens. It is spoken to in its own protocol, the Messages API, by the
  new adapter `columbina.llm.adapters.anthropic_messages`, and not through the layer of
  Anthropic that answers as OpenAI does. Every request asks the service to keep the
  conversation in its cache, for five minutes unless the config says an hour or not at all,
  so that a request that begins as the one before it did reads that beginning at a fraction
  of the price. The thinking of the model is shown as its summary, and goes back with its
  signature in the requests that follow. `/effort` offers it every level but off, which not
  every model of Claude allows, and sends the level as the effort of the request.
  `columbina setup --check`, `columbina doctor` and the list of models of the desktop
  application ask Anthropic for its models with the version of the API that it wants. The key
  is looked for, and kept out of every failure, as for the other services. A conversation
  that began with another service goes on with Claude: what Anthropic would refuse in it,
  such as the ids that some services give their calls or text that is only blank, is put in
  a shape that it takes, and calls and their results are told of in text where a request
  offers no tools, as one for a summary does.
- In the trace of a task in the desktop application, a call shows how long it ran as the
  program measured it, and not the time until its result was written, which held the wait
  for a place to run and for the results of the calls before it. A call that the end of its
  turn cut short, or did not run, is cut short and not failed, and what was said while a step
  ran belongs to the step after it, which reads it.
- The list of models of the desktop application names the services that are set up, and
  shows the models of the one that is clicked. A model of another service chooses that
  service, and the service process is started again at the task that is open, which goes on
  with it. This is done at rest only.
- The assistant in Feishu works in a folder of its own, `~/Columbina/feishu`, beside the home
  of the program, or where `COLUMBINA_FEISHU_HOME` says, and no longer in `~/.columbina/feishu`,
  where the program keeps its lock, how it is doing and which messages it took. The first time
  that it starts after that, its instruction file, skills, the files that it was sent and its
  session are copied into its folder, each one whole and none over a file that is there already,
  and its conversation goes on. That is done once; links stay behind. Its owner may let its agent
  change the files of that folder: `columbina feishu --from-stdin` keeps `{"writable": true}`
  alone, while no assistant runs, and the assistant then runs with the profile `feishu-writable`,
  which gives it the tools that write and edit files in the mode that holds them to its folder.
  It still runs no commands. Its conversation goes by what its owner lets it do, whatever it was
  let do before and whatever the program lets other agents do, and the rules of the user in
  `permissions.yml` hold for it. A folder that lies in the home of the program is not one whose
  files it may change. `columbina feishu --status` says where the folder is and whether its files
  may be changed.
- The settings of the desktop application have a switch for that, which stops an assistant that
  runs and starts it again, and open the folder of the assistant. While the assistant runs,
  closing the window puts the application in the tray, where the assistant goes on, and the
  tray opens the window again or quits. Only one application runs at a time, and starting it
  again shows the window. On Windows a package can start with the system, in the tray.
- A conversation can belong to no project. The profile `chat` is that of `columbina serve`,
  whose agent changes no file, runs no command, reaches no GitHub and hands no work to other
  agents, and is told that it is in no project.
- *Just chat*, on the first page of the desktop application and at the foot of the list of
  tasks, starts the service with that profile in a folder of its own, `~/Columbina/chat`. The
  window calls it a chat, and the line under the field says what it may do in its own words.

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
