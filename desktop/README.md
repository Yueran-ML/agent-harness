# Columbina desktop

The desktop application is a window for a conversation with Columbina. An Electron
shell opens the window, lets the user choose a project folder and starts the service
process `columbina serve` in it. A React interface shows the session and turns what
the user does into requests. The two talk in the messages of
[`docs/protocol`](../docs/protocol/README.md): JSON-RPC, one message to a line, over
the standard input and output of the service process.

**This is work in progress.** The shell and the protocol layer are in use. The
interface is being redone, and what is described here may change.

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
  dialog of the system. The interface cannot name another folder, another program or
  other arguments.
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

`npm test` runs the unit tests: the state that is computed from events, the matching of
requests and answers, and the replay process. They ask no model for anything.
