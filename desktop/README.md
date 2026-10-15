# Columbina desktop

The desktop application is a window for a conversation with Columbina. An Electron
shell opens the window, lets the user choose a project folder and starts the service
process `columbina serve` in it. A React interface shows the session and turns what
the user does into requests. The two talk in the messages of
[`docs/protocol`](../docs/protocol/README.md): JSON-RPC, one message to a line, over
the standard input and output of the service process.

A package for Windows on x64 and one for macOS on Apple silicon are downloaded from the
releases of the repository, as the [README](../README.md#the-desktop-application) says.
Each brings a Python of its own with Columbina in it, and nothing else needs to be
installed. The application also runs from source.

Columbina is an unofficial fan project. The illustrations, the video and the icon of
this application are AI-generated, are kept in [`assets/`](assets) and are not covered
by the MIT license of the code. See [ASSETS.md](ASSETS.md).

## Run from source

Node.js 22.12 or newer is needed. The build copies the three artwork files from
`assets/`, and [ASSETS.md](ASSETS.md) says how other ones are used instead.

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
the input field. If the task is left first, it is the draft of that task when the task
is opened again.

The page of a new task says what the task may do and names the skills that there are.
A click on a skill writes the command that hands it to the agent into the input field.

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

On macOS ⌘ is held where the table says Ctrl, and the window writes its shortcuts with ⌘.
Ctrl does the same there.

While the window is in the background, the system tells of a question that waits and
of a turn that has ended. The settings switch that off. The first page offers the
projects that were opened before. If the service process ends, the page shows the last
lines that it wrote, which most often say why.

Before any project is open, the first page goes through the steps that come before the
work: a model service with its key, GitHub if the user likes, and the project. The
keyboard begins at the first step that is still to be done. Once all that is offered is
done or left for later, the steps fold into one line under the button that opens a
project, such as `DeepSeek · deepseek-flash · @octocat`, and a click on it opens them
again. They come back where a key was deleted meanwhile. The page asks again what is set
up whenever the window comes back to the front.

The interface speaks English, Chinese and Japanese and follows the language of the
system unless another is chosen in the settings. It has a dark theme and a light one,
and three sizes of type.

The first start begins with a short film. Any key or a click leaves it, and the starts
after that go without it unless the settings say that it is to play every time.

## The service of the model, and its key

The settings list the services that are set up and say where the key of each comes
from. A service is chosen and a key is given in a window of its own. That window shows
nothing that a model wrote, and it is the only part of the application that ever holds
a key. What is typed there goes to the shell, and the shell writes it to the standard
input of `columbina setup --from-stdin`, which stores it: in the keyring of the system,
or in a file where no keyring takes it, and the window says which. The key is on no
command line, in no environment and in no message of the protocol. Its field is given
no value by the page, so the key is in no markup of the window either. The window that
shows the conversation is told only that something was stored. A service of the user's
own gets a name there, which the window holds to what the program takes for one.

Before anything is stored, the window can check a service: it hands the service, its
address and the key that is typed to `columbina setup --check`, which asks the service for
its models and stores nothing. The window then offers those models for the field of the
model, and says so where the model that is written is none of them. A check that fails
says what to look at: the network and the proxy, the key, an account without money, the
address, or a busy service. A service on this computer, such as Ollama, is checked as soon
as it is chosen, and the window says how to start one that does not answer. The window
marks a model that looks like a key and a key with a blank inside it, and links to the
page of the service on which a key is made, naming the machine that it lies on. A key that
is stored can be deleted there, once the window has asked whether it should be.

The button at the foot of the field shows the model in use, and the settings show the same
button where the model is set. It opens the models of the service in use, which the shell
asks for with `columbina setup --check` and the key that a session would send, so that no key
comes to the page this way either. A filter goes over them: the model that is written comes
first, then those that begin with it, then the others that hold it. A name that the service
does not list can be used as it is written, unless it looks like a key, so a model can be
chosen where the service lists none or cannot be asked. A list is asked for again once it is
five minutes old, and at once after a failure, which says what to look at as the window for
the key does. The model that is chosen is used from the next request on, and it is stored
for its service through `columbina setup --from-stdin`, without a key, so that the next start
uses it too. While Columbina is at work the list can be looked at, and nothing in it chosen.
Where another service was chosen in the window for the key and waits for the next start, a
model that is chosen is used and not stored, and the other service stays chosen.

This works where the application runs the service itself, and not in a replay. The
service looks for the key with every request, so a key is used as soon as it is stored,
and a turn that failed for want of it can be tried again with the button under it.
Another service, address or model is read when the service starts. Where nothing is under
way when the window for the key is closed, the service is started again at once, at the
task that is open. A turn is not cut off for it: the settings offer the new start for when
the turn has ended.

## GitHub

The first page and the settings sign in to GitHub with the device flow of the GitHub App of
Columbina. The page shows the code large, copies it, and opens the page of GitHub on which
it is entered, with its full address written beside the button. A clock says how long the
code holds, and a code that has run out offers a new one. The sign-in can be stopped at any
time. Each kind of failure says what to look at: the network and the proxy, a code that ran
out, a sign-in that was turned down on GitHub, a pause that GitHub asks for, or a build of
Columbina that has no app to sign in with.

Once somebody is signed in, the settings show the picture, the name and the login, and the
repositories that Columbina may reach in that name. Repositories are chosen on GitHub: a
user who has chosen none is led there, and the list is asked for again when the window
comes back to the front. A sign-out is asked first, and then says where on GitHub the right
that was given to the app is taken back.

The shell runs `columbina login github --json` for a sign-in, `columbina login --status
--json` to say who is signed in and `columbina logout github --json` for a sign-out. No
token comes to the page: the program keeps the tokens, and tells the page only who signed
in and with which picture, as an address of data that the content security policy allows.
In the conversation a call on GitHub says what it does there, such as `Push fix-parser`, and
a question asks with a sentence of its own whether Columbina may push, open a pull request or
comment, above the whole of what it would send.

## On Windows and on macOS

The page draws the bar of the window, and leaves room for the buttons of the system. On
Windows they lie over its right end, in the colours of the theme. On macOS they stand at its
left end, in the middle of its height, and the bar gives their room back in full screen. A
sheet of macOS, such as the dialog for a folder or the window for the key, comes down from
under the bar. The menu of the application there has what copies and pastes, closes the
window and leaves the application, and nothing that would load the page again.

An application that is opened from the Finder is given a short `PATH`, without Homebrew or
`/usr/local/bin`, and no proxy. So a package asks the login shell of the user once, as a
terminal starts it, for its `PATH` and for `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and
`NO_PROXY`, in either case. The service and the commands of the agent then find what the
user has installed and reach what the user reaches through the proxy. A proxy that the
application was given goes first. A shell that says nothing within five seconds leaves the
`PATH` as it is, with those two places added.

## The service

A package starts the Python that it brings, in `resources/python`:
`python -I -B -m columbina serve`. `-I` reads no `PYTHON` variable and no site of the user,
and puts no folder of the project before the packages of the package. `-B` writes nothing
into the package, whose modules are all compiled ahead. The environment of the service is
that of the application, and the commands of the agent inherit it. A package never replays a
recorded session: where its Python is missing, it says so.

From source, without further settings, the shell replays a recorded session. Two
environment variables of the shell that starts Electron choose another service process, in
a package as well.

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
- The window for the key has a bridge of its own, with six calls: read what is set up,
  store, check a service, delete a key, open the page on which a key is made, and close.
  The main window cannot make them. A key is deleted only for a service that the window
  was told of, and a page is opened only where a service links to it over https.
- One sign-in to GitHub runs at a time. One that is begun takes the place of one that runs,
  whose page no longer waits for it. The page hears only the documents of JSON that the
  program writes, each line at most 2 MB, and that it has ended. A sign-in is stopped by
  closing its input, and ended with what it started where it has not ended three seconds
  later; it is stopped when the page is gone and when the window is closed.
- The page loads only files of the application. Its content security policy allows no
  connection to the network, no frame and no script from elsewhere. Navigation and new
  windows are blocked. A link is opened in the browser of the system, after its full
  address was shown.
- When the window is closed, the interface is given a moment to send `shutdown`. The
  shell then closes the standard input of the service process, waits for it to end, and
  ends it and what it started if it has not ended after a few seconds.

## Packages

```console
npm run package
npm run package:try
```

`npm run package` builds the interface, puts the Python of the package together and makes
the application around them, with the icon of the system, and a zip of it in `release/`. It
runs on Windows on x64 and on macOS on Apple silicon, and makes the package of the system
that it runs on. The Python is a CPython of python-build-standalone at a fixed version,
fetched by uv. Columbina is built from the checkout, and what it needs is installed at the
versions and with the hashes of `uv.lock`. What a service never uses is taken out, and every
module is compiled ahead. On macOS the application is signed without a certificate, which
Apple silicon needs, and zipped with its links as they are.

`npm run package:try` tries the package as someone who downloads it has it: the pages and the
artwork are in it, and its Python runs a task with a scripted model, greets and leaves as the
window has it do, and writes nothing into the package.

The workflow `desktop.yml` does both on GitHub, for both systems. A tag such as
`desktop-v0.1.0`, the version of `package.json`, publishes the two zips as a release.

## Checks

```console
npm test
npm run build
```

`npm test` runs the unit tests: the state that is computed from events, the grouping
into turns, the words for every way a call or a turn can end, the matching of requests
and answers, the shell, and the replay process. Four of the files mount the page in a
DOM without a browser, part by part and as a whole, and read what stands there and what
a click or a key calls: the conversation, the question of the agent, the field, the
status line, the list of tasks, the page of a new task, the settings and the panel
beside the conversation. The whole page is mounted with a workspace that the test
holds, so what one part does to another is tried without a service process. None of the
tests asks a model for anything, and none sees how the page is laid out or what it looks
like.
