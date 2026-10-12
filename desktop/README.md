# Columbina desktop

The desktop application is a window for a conversation with Columbina. An Electron
shell opens the window, lets the user choose a project folder and starts the service
process `columbina serve` in it. A React interface shows the session and turns what
the user does into requests. The two talk in the messages of
[`docs/protocol`](../docs/protocol/README.md): JSON-RPC, one message to a line, over
the standard input and output of the service process.

**This is work in progress.** It runs from source, and it is told by two environment
variables which service process to start, see below.

Columbina is an unofficial fan project. The illustrations, the video and the icon of
this application are AI-generated, are kept outside Git and are not covered by the MIT
license of the code. See [ASSETS.md](ASSETS.md).

## Run from source

Node.js 22.12 or newer is needed. The build copies three artwork files from a
directory outside the repository, and stops with a message if they are missing.
[ASSETS.md](ASSETS.md) says which files and where to name the directory.

```console
npm install
npm run dev
```

`npm run dev` serves the interface in a browser against a replay of a recorded session.
Nothing is asked of a model, no command is run and no file is changed.

```console
npm run desktop
```

`npm run desktop` builds the interface and opens it in Electron. Without further
settings it starts the same replay as a process of its own.

## The interface

A task is shown turn by turn: what was said, what the agent did on the way, and what
came of it. Once a turn has its answer, the steps fold into one line that says how long
it took. A turn that did not end as it should says what happened and why, and offers
what can be done next.

When the agent needs a yes, a card with the full command or path stands above the
input field, and the conversation stays readable. What is sent while a turn runs is
read at its next step. If the turn is stopped first, what was not read comes back into
the input field.

| Key | What it does |
|---|---|
| Enter | Sends what is written. Shift+Enter starts a new line |
| `/` | Opens the list of commands. Up and Down choose, Enter or Tab takes one, Esc closes the list |
| Esc, twice | Stops the turn that is under way |
| Ctrl+Enter | Says yes, for this once, to what the agent asks |
| Ctrl+N | Begins a new task |
| Ctrl+B | Shows or hides the list of tasks |
| Ctrl+K | Goes to the input field. On an empty field it opens the list of commands |
| Ctrl+, | Opens the settings |

While the window is in the background, the system tells of a question that waits and
of a turn that has ended. The settings switch that off. The first page offers the
projects that were opened before. If the service process ends, the page shows the last
lines that it wrote, which most often say why.

The interface speaks English and Chinese and follows the language of the system
unless another is chosen in the settings. It has a dark theme and a light one.

## The service of the model, and its key

The settings list the services that are set up and say where the key of each comes
from. A service is chosen and a key is given in a window of its own. That window shows
nothing that a model wrote, and it is the only part of the application that ever holds
a key. What is typed there goes to the shell, and the shell writes it to the standard
input of `columbina setup --from-stdin`, which stores it: in the keyring of the system,
or in a file where no keyring takes it, and the window says which. The key is on no
command line, in no environment and in no message of the protocol, and the window that
shows the conversation is told only that something was stored.

This works where the application runs the service itself, and not in a replay. What
was stored is used once the service is started again.

## With the real service

Two environment variables of the shell that starts Electron choose the service process.

| Variable | Value |
|---|---|
| `COLUMBINA_SERVICE_EXECUTABLE` | The absolute path of the program to start, for example the `python` of an environment that has Columbina installed |
| `COLUMBINA_SERVICE_ARGS` | Its arguments as a JSON array of strings. Without it they are `["-m", "columbina", "serve"]` |

```powershell
$env:COLUMBINA_SERVICE_EXECUTABLE = 'C:\path\to\python.exe'
npm run desktop
```

The service process runs in the folder that the user chose and inherits the environment
of the shell, the key of the model service included. The key is in no message of the
protocol, and the interface cannot read it. What the service process logs goes to
`columbina-service.log` in the log directory of the application.

## What the shell allows

- The window runs with context isolation and a sandbox, and without Node integration.
  Every call from the interface is checked to come from the page of the application.
- A service process is started only in a folder that the user chose in the folder
  dialog of the system, now or at an earlier time: the shell keeps the last eight. The
  interface cannot name another folder, another program or other arguments.
- The window for the key has a bridge of its own, with three calls: read what is set
  up, store, and close. The main window cannot make them.
- The page loads only files of the application. Its content security policy allows no
  connection to the network, no frame and no script from elsewhere. Navigation and new
  windows are blocked. A link is opened in the browser of the system, after its full
  address was shown.
- When the window is closed, the interface is given a moment to send `shutdown`. The
  shell then closes the standard input of the service process, waits for it to end, and
  ends it and what it started if it has not ended after a few seconds.

## Checks

```console
npm test
npm run build
```

`npm test` runs the unit tests: the state that is computed from events, the grouping
into turns, the words for every way a call or a turn can end, the matching of requests
and answers, the shell, and the replay process. They ask no model for anything.
