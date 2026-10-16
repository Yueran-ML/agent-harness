const { app, BrowserWindow, Menu, Notification, dialog, ipcMain, shell } = require('electron');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');
const {
  constants, createWriteStream, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, statSync, writeFileSync,
} = require('node:fs');
const { copyFile, mkdir, open, realpath, stat, writeFile } = require('node:fs/promises');
const path = require('node:path');
const { StringDecoder } = require('node:string_decoder');
const { pathToFileURL, fileURLToPath } = require('node:url');

let window;
let service = null;
let stopping = null;
let exiting = false;
let serviceOperation = Promise.resolve();
let pendingServiceOperations = 0;
let closeTimer;
const approvedDirectories = new Set();
const indexPath = path.resolve(__dirname, '../dist/index.html');
const indexUrl = pathToFileURL(indexPath).href;
// The page of the window that takes a key. It shows nothing that a model wrote.
const keyPath = path.resolve(__dirname, '../dist/key.html');
const keyUrl = pathToFileURL(keyPath).href;
// The bar of the window in each theme: the furthest surface of `src/ui/tokens.css`, and its text.
const titleBarThemes = {
  moonveil: { color: '#090b12', symbolColor: '#eef0f8', height: 40 },
  moonwhite: { color: '#f5f4f0', symbolColor: '#1b1c24', height: 40 },
};

// How much is kept of what a service wrote for errors: so many lines, and of a line so many characters.
const TAIL_LINES = 30;
const TAIL_LINE_CHARS = 400;
// How many folders are kept as recent projects.
const RECENT_MOST = 8;
// How long `columbina setup` may take. It waits up to 30 seconds for a keyring that is locked.
const SETUP_MS = 45_000;
// How much of what it writes is read, in characters. Its answer is one line.
const SETUP_OUTPUT_MOST = 512 * 1024;
const NO_SETUP = 'No service program is set up for this application, so its services cannot be set up from here.';
// How much of what `columbina login --status` writes is read: its line may hold the picture of the user.
const ACCOUNT_OUTPUT_MOST = 2 * 1024 * 1024;
// How long the program that signs in to GitHub is given to end after its input was closed.
const SIGN_IN_STOP_MS = 3_000;
// How long the login shell of the user may take on macOS to say what it has, and how much of what it writes is read.
const LOGIN_SHELL_MS = 5_000;
const LOGIN_SHELL_OUTPUT_MOST = 64 * 1024;
// What the PATH of an application opened from the Finder leaves out on macOS: the programs of Homebrew, and /usr/local.
const MAC_PROGRAMS = ['/opt/homebrew/bin', '/opt/homebrew/sbin', '/usr/local/bin', '/usr/local/sbin'];
// The variables of the proxy, each in the two cases that programs read it in. The login shell is
// asked for them with the PATH: a user who sets a proxy for a terminal sets it there.
const SHELL_PROXIES = [['HTTPS_PROXY', 'https_proxy'], ['HTTP_PROXY', 'http_proxy'], ['ALL_PROXY', 'all_proxy'], ['NO_PROXY', 'no_proxy']];
const SHELL_VARIABLES = ['PATH', ...SHELL_PROXIES.flat()];

/**
 * How the bar of the window is drawn. The page draws the bar itself and leaves room for the
 * buttons of the system: on Windows and Linux they lie over its right end, in the colours of
 * the theme, and on macOS they stand at its left end, in the middle of its height.
 */
function windowBar(theme) {
  if (process.platform === 'darwin') return { titleBarStyle: 'hidden', trafficLightPosition: { x: 16, y: 13 } };
  return { titleBarStyle: 'hidden', titleBarOverlay: theme };
}
// What the shell says by itself, in every language that the interface speaks, each under the
// name that a page gives it. Everything else that is said is said by a page.
const WORDS = {
  en: { chooseFolder: 'Choose a project folder', chooseFiles: 'Choose files to attach' },
  ja: { chooseFolder: 'プロジェクトフォルダーを選択', chooseFiles: '添付するファイルを選択' },
  'zh-CN': { chooseFolder: '选择项目文件夹', chooseFiles: '选择要附上的文件' },
};
// The project that the service of the moment works in: the process, its folder, and whether it is a replay.
let serviceProject = null;
// The files that the user chose or dropped to go with a message, by the ids that the page knows
// them by. The page is told their names and sizes, and never where they lie.
const attachments = new Map();
let attachmentNumber = 0;
/** How large a file that goes with a message may be, and how many go with one message. */
const ATTACHMENT_MOST_BYTES = 25 * 1024 * 1024;
const ATTACHMENTS_MOST = 10;
/** How many chosen files the shell holds at most, for all the messages that are being written. */
const ATTACHMENTS_HELD_MOST = 200;
/** Where the files that go with messages are kept in a project, from its folder. */
const ATTACHMENTS_FOLDER = ['.columbina', 'attachments'];
/** The language that a page names, if the shell speaks it, and English if not. */
const spoken = (language) => (typeof language === 'string' && Object.hasOwn(WORDS, language) ? language : 'en');
// The window for the key while it is open, what it was opened for, whether something was
// stored or deleted from it, and the promise that says so when it has closed.
let keyWindow = null;
let keyAsked = { service: null, language: 'en' };
let keyChanged = false;
let keyClosed = null;
// The services that the window for the key was told of last, and the pages on which their keys are made.
let keyListed = { names: [], pages: [] };
// The services that the main window was told of last, by name. A model is listed and kept for one of them.
let mainListed = [];
// What the service of the moment wrote last for errors. It is told of when the service has ended.
let lastWords = { lines: [], rest: '' };
// What is kept from one start to the next. Read when the application is ready.
let preferences = { theme: 'moonveil', recent: [], githubSkipped: false };
let flashing = false;
// The notification that shows, held so that it is not cleared away while it does.
let shown = null;
// On macOS, what the login shell of the user said of its variables, and the asking for it while it lasts.
let loginShell = null;
let askingLoginShell = null;
// The program that signs in to GitHub while it waits for the user. There is one at most.
let signingIn = null;

app.setName('Columbina');
if (process.platform === 'win32') app.setAppUserModelId('dev.columbina.desktop');

const logPath = () => path.join(app.getPath('logs'), 'columbina-service.log');
const preferencesPath = () => path.join(app.getPath('userData'), 'preferences.json');

/** Text as it may be shown: on one line, without what steers a terminal. */
function plain(text) {
  return text
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/[\u0000-\u001f\u007f]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}
/** The beginning of a text that is longer than it may be, marked as cut. */
function short(text, most) {
  const characters = Array.from(text);
  return characters.length > most ? `${characters.slice(0, most).join('')}…` : text;
}
/** Keep the last lines that a service writes for errors, and return where they are kept. */
function listenForErrors(child) {
  const words = { lines: [], rest: '' };
  const decoder = new StringDecoder('utf8');
  child.stderr.on('data', (chunk) => {
    // What ends a line on Windows beside the line feed is taken out with whatever else steers a terminal.
    const parts = (words.rest + decoder.write(chunk)).split('\n');
    // A line without an end is not held without bound either.
    words.rest = parts.pop().slice(0, 4 * TAIL_LINE_CHARS);
    for (const part of parts) {
      const line = plain(part);
      if (line) words.lines.push(short(line, TAIL_LINE_CHARS));
    }
    if (words.lines.length > TAIL_LINES) words.lines.splice(0, words.lines.length - TAIL_LINES);
  });
  return words;
}
/** What was kept at the last start, as far as it can be used. Anything else is left out. */
function readPreferences() {
  let kept = null;
  try { kept = JSON.parse(readFileSync(preferencesPath(), 'utf8')); } catch { /* Nothing was kept, or it cannot be read. */ }
  // Whatever the file holds, only a theme that there is and folders that can be folders are taken from it.
  const given = kept ?? {};
  const theme = typeof given.theme === 'string' && Object.hasOwn(titleBarThemes, given.theme) ? given.theme : 'moonveil';
  const recent = [];
  for (const folder of Array.isArray(given.recent) ? given.recent : []) {
    if (typeof folder !== 'string' || !path.isAbsolute(folder)) continue;
    if (recent.some((known) => directoryKey(known) === directoryKey(folder))) continue;
    recent.push(folder);
    if (recent.length === RECENT_MOST) break;
  }
  // Whether the user said on the first page that GitHub can wait.
  return { theme, recent, githubSkipped: given.githubSkipped === true };
}
/** Write down what is kept. It is written beside its place and moved there, so that a start never finds half of it. */
function keepPreferences() {
  const file = preferencesPath();
  try {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(`${file}.tmp`, `${JSON.stringify(preferences, null, 2)}\n`, 'utf8');
    renameSync(`${file}.tmp`, file);
  } catch { /* What cannot be kept is chosen again at the next start. */ }
}
/** Put a folder that the user chose first among the recent projects. */
function remember(cwd) {
  const others = preferences.recent.filter((known) => directoryKey(known) !== directoryKey(cwd));
  preferences.recent = [cwd, ...others].slice(0, RECENT_MOST);
  keepPreferences();
}
function isFolder(folder) {
  try { return statSync(folder).isDirectory(); } catch { return false; }
}

function checkSender(event) {
  if (!window || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== indexUrl) {
    throw new Error('IPC sender is not the application window.');
  }
}
function checkKeySender(event) {
  if (!keyWindow || event.sender !== keyWindow.webContents || event.senderFrame !== keyWindow.webContents.mainFrame || event.senderFrame.url !== keyUrl) {
    throw new Error('IPC sender is not the window for the key.');
  }
}
/** A window opens no other window, goes to no other page and holds no page inside its own. */
function closeToTheWorld(opened) {
  opened.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  opened.webContents.on('will-navigate', (event) => event.preventDefault());
  opened.webContents.on('will-attach-webview', (event) => event.preventDefault());
}
function tell(channel, value) {
  if (window && !window.isDestroyed() && !window.webContents.isDestroyed()) window.webContents.send(channel, value);
}
function directoryKey(directory) {
  return process.platform === 'win32' ? directory.toLowerCase() : directory;
}
function approvedDirectory(options) {
  if (typeof options?.cwd !== 'string' || !options.cwd || !path.isAbsolute(options.cwd)) {
    throw new Error('Choose a project folder before starting the service.');
  }
  const cwd = realpathSync(options.cwd);
  if (!approvedDirectories.has(directoryKey(cwd))) {
    throw new Error('The working directory must be selected in the native folder dialog.');
  }
  if (!statSync(cwd).isDirectory()) throw new Error('The working directory is not a folder.');
  return cwd;
}
function enqueueService(operation) {
  pendingServiceOperations += 1;
  const result = serviceOperation.then(operation);
  serviceOperation = result.catch(() => {}).finally(() => { pendingServiceOperations -= 1; });
  return result;
}
function killTree(child) {
  if (!child.pid) return;
  if (process.platform === 'win32') {
    const killer = spawn('taskkill.exe', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
    killer.on('error', () => { try { child.kill(); } catch {} });
  } else {
    try { process.kill(-child.pid, 'SIGKILL'); } catch { try { child.kill('SIGKILL'); } catch {} }
  }
}
async function stopService() {
  if (stopping) return stopping;
  const child = service;
  if (!child) return;
  stopping = new Promise((resolve) => {
    let done = false;
    let timer;
    const finish = () => {
      if (done) return;
      done = true; clearTimeout(timer);
      if (service === child) service = null;
      resolve();
    };
    child.once('close', finish);
    // EOF is the protocol's graceful shutdown fallback if the renderer has gone away.
    child.stdin.end();
    timer = setTimeout(() => { killTree(child); setTimeout(finish, 500); }, 2500);
    if (child.exitCode !== null || child.signalCode !== null) finish();
  });
  try { await stopping; } finally { stopping = null; }
}
/**
 * Ask the login shell of the user, the way a terminal starts it, what its PATH is and which proxy
 * it sets. The answer holds what it said of each variable, which is empty for one that it does not
 * set, and is null where it says nothing in time.
 *
 * An application that is opened from the Finder or the Dock on macOS is given only
 * `/usr/bin:/bin:/usr/sbin:/sbin` and no proxy. The commands that an agent runs would not find
 * what the user has installed, such as Homebrew, Node or Docker, nor reach what the user reaches
 * through the proxy. Each variable is marked at both ends by its own name, so that what the files
 * of the shell print by themselves is left out.
 */
function askLoginShell() {
  const program = process.env.SHELL && path.isAbsolute(process.env.SHELL) ? process.env.SHELL : '/bin/zsh';
  const markOf = (name) => `_COLUMBINA_${name}_`;
  const script = SHELL_VARIABLES.map((name) => `printf '${markOf(name)}%s${markOf(name)}' "$${name}"`).join('; ');
  return new Promise((resolve) => {
    let child = null;
    let written = '';
    const timer = setTimeout(() => {
      try { child?.kill(); } catch { /* It has ended. */ }
      resolve(null);
    }, LOGIN_SHELL_MS);
    const answer = (found) => { clearTimeout(timer); resolve(found); };
    try {
      child = spawn(program, ['-ilc', script], {
        // Oh My Zsh would ask first whether it may update itself.
        env: { ...process.env, DISABLE_AUTO_UPDATE: 'true' }, stdio: ['ignore', 'pipe', 'ignore'],
      });
    } catch {
      answer(null);
      return;
    }
    child.stdout.on('data', (chunk) => { if (written.length < LOGIN_SHELL_OUTPUT_MOST) written += chunk; });
    child.on('error', () => answer(null));
    child.on('close', () => {
      const said = {};
      for (const name of SHELL_VARIABLES) {
        const parts = written.split(markOf(name));
        if (parts.length >= 3) said[name] = parts[1].trim();
      }
      answer(said);
    });
  });
}
/** The PATH of the commands of the service on macOS: the one of the login shell, or the usual places added to the one there is. */
function macPath() {
  if (loginShell?.PATH) return loginShell.PATH;
  const places = (process.env.PATH ?? '').split(':').filter(Boolean);
  return [...places, ...MAC_PROGRAMS.filter((place) => !places.includes(place))].join(':');
}
/**
 * The Python that a package brings, with Columbina in it, or null in development.
 * It lies in `resources/python`, the way the packaging puts it there.
 */
function bundledPython() {
  if (!app.isPackaged) return null;
  const root = path.join(process.resourcesPath, 'python');
  return process.platform === 'win32' ? path.join(root, 'python.exe') : path.join(root, 'bin', 'python3');
}
/**
 * The environment of the service of a package: that of the application, with the PATH and the
 * proxy of the user on macOS. Nothing in it is meant for the Python of the package, since the
 * commands of the agent inherit it: that Python keeps to itself through the flags that it is
 * started with.
 */
function bundledEnvironment() {
  const env = { ...process.env };
  if (process.platform === 'darwin') {
    env.PATH = macPath();
    // A proxy that the application was given goes first, in whichever case it is written.
    for (const names of SHELL_PROXIES) {
      if (names.some((name) => env[name])) continue;
      for (const name of names) if (loginShell?.[name]) env[name] = loginShell[name];
    }
  }
  return env;
}
/**
 * The program of the service: the one that COLUMBINA_SERVICE_EXECUTABLE names, else the Python
 * of the package, else, in development only, the replay. A package never replays: what it shows
 * is a real service, or that there is none.
 */
function serviceCommand() {
  const executable = process.env.COLUMBINA_SERVICE_EXECUTABLE;
  if (!executable) {
    const python = bundledPython();
    if (python === null) return { executable: process.execPath, args: [path.join(__dirname, 'mock-service.mjs')], mode: 'mock', env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' } };
    if (!existsSync(python)) throw new Error('This package has no service of its own. Download it again from the releases of Columbina.');
    // -I reads no PYTHON variable and no site of the user, and puts no folder of the project before
    // the packages of the package. -B writes nothing into the package, whose files are all compiled.
    return { executable: python, args: ['-I', '-B', '-m', 'columbina', 'serve'], mode: 'service', env: bundledEnvironment() };
  }
  if (!path.isAbsolute(executable)) throw new Error('COLUMBINA_SERVICE_EXECUTABLE must be an absolute executable path.');
  const args = process.env.COLUMBINA_SERVICE_ARGS ? JSON.parse(process.env.COLUMBINA_SERVICE_ARGS) : ['-m', 'columbina', 'serve'];
  if (!Array.isArray(args) || args.some((value) => typeof value !== 'string')) throw new Error('COLUMBINA_SERVICE_ARGS must be a JSON array of strings.');
  return { executable, args, mode: 'service', env: { ...process.env } };
}
/**
 * Another command of the program, such as `setup --list`, made from the command of the service
 * process. Null where there is none to make it from: in a replay, or where the command does not serve.
 */
function programCommand(words) {
  const config = serviceCommand();
  const serves = config.args.indexOf('serve');
  if (config.mode !== 'service' || serves < 0) return null;
  // What follows `serve` was meant for serving, such as a profile.
  return { executable: config.executable, args: [...config.args.slice(0, serves), ...words], env: config.env };
}
/**
 * Run a command of the program in a form that asks nothing, such as `setup --list`, and return
 * the document that it answers with.
 *
 * What it is given goes to its standard input and nowhere else: not to its command line, not
 * into its environment, not into a file. That is how a key reaches it. Of what it writes, so
 * many characters are read at most.
 */
async function runProgram(words, input = '', most = SETUP_OUTPUT_MOST) {
  if (askingLoginShell) await askingLoginShell;
  const command = programCommand(words);
  if (!command) throw new Error(NO_SETUP);
  return new Promise((resolve, reject) => {
    const child = spawn(command.executable, command.args, {
      cwd: app.getPath('home'), env: command.env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
    });
    const decoder = new StringDecoder('utf8');
    let written = '';
    let over = false;
    let done = false;
    const settle = (act, value) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      act(value);
    };
    const timer = setTimeout(() => {
      killTree(child);
      settle(reject, new Error('The setup program did not end in time.'));
    }, SETUP_MS);
    child.stdout.on('data', (chunk) => {
      if (over) return;
      written += decoder.write(chunk);
      if (written.length > most) {
        over = true;
        written = '';
      }
    });
    // What a person would be shown. It is taken so that the program is not held up, and kept nowhere.
    child.stderr.on('data', () => {});
    // A program that ended before it read is told of by how it ended.
    child.stdin.on('error', () => {});
    child.on('error', (error) => settle(reject, new Error(`The setup program could not be started: ${error.message}`)));
    child.on('close', () => {
      if (over) {
        settle(reject, new Error('The setup program wrote more than is read.'));
        return;
      }
      let document = null;
      try { document = JSON.parse((written + decoder.end()).trim().split('\n').at(-1)); } catch { /* It wrote no document. */ }
      if (document === null || typeof document !== 'object' || Array.isArray(document)) {
        settle(reject, new Error('The setup program did not answer with a document.'));
      } else {
        settle(resolve, document);
      }
    });
    // Its input is closed after what it is given, so that it waits for nothing more.
    child.stdin.end(input);
  });
}
/** The folder of the project that the service of the moment works in, or null where none runs or it replays. */
function projectOfTheService() {
  // The service of the project is gone once there is another service, or none.
  return serviceProject?.child === service && serviceProject.mode === 'service' ? serviceProject.cwd : null;
}
/**
 * Hold files that the user chose or dropped, each where it is a file and not too large. Returns
 * what the page is told: the files that are held, by id, name and size, and the names of the
 * others with why they are not. How many go with one message, the page sees to.
 */
async function attach(paths) {
  const attached = [];
  const refused = [];
  for (const chosen of paths) {
    const name = path.basename(chosen);
    let found;
    try {
      found = await stat(chosen);
    } catch {
      refused.push({ name, why: 'gone' });
      continue;
    }
    if (!found.isFile()) refused.push({ name, why: 'notAFile' });
    else if (found.size > ATTACHMENT_MOST_BYTES) refused.push({ name, why: 'tooBig' });
    else if (attachments.size >= ATTACHMENTS_HELD_MOST) refused.push({ name, why: 'tooMany' });
    else {
      const id = `attachment-${++attachmentNumber}`;
      attachments.set(id, { path: chosen, name, size: found.size });
      attached.push({ id, name, size: found.size });
    }
  }
  return { attached, refused };
}
/** The name of the folder that the files of one message are kept in: when they were sent, to the second. */
function sentAt(date) {
  const two = (number) => String(number).padStart(2, '0');
  return `${date.getFullYear()}${two(date.getMonth() + 1)}${two(date.getDate())}-`
    + `${two(date.getHours())}${two(date.getMinutes())}${two(date.getSeconds())}`;
}
/**
 * Copy a file into a folder under its name, or under the name with a number where one of that name
 * is there already. What cannot be seen, such as the end of a line, is no part of the name: the
 * message names each file on a line of its own. Returns the name that it has there.
 */
async function copiedInto(folder, file) {
  const name = file.name.replace(/[\u0000-\u001f\u007f]/g, '_');
  const dot = name.lastIndexOf('.');
  const [stem, ending] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let number = 1; number <= ATTACHMENTS_HELD_MOST; number += 1) {
    const taken = number === 1 ? name : `${stem} (${number})${ending}`;
    try {
      await copyFile(file.path, path.join(folder, taken), constants.COPYFILE_EXCL);
      return taken;
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error;
    }
  }
  throw new Error(`There is no name left for ${file.name}.`);
}
/** The names of the services in what `setup --list` answered, and the pages on which their keys are made. */
function namesAndPages(list) {
  const services = Array.isArray(list?.services) ? list.services : [];
  const text = (value) => (typeof value === 'string' && value ? [value] : []);
  return {
    names: services.flatMap((service) => text(service?.name)),
    pages: services.flatMap((service) => text(service?.key_page)),
  };
}
function installHandlers() {
  ipcMain.handle('columbina:select-folder', async (event, asked) => {
    checkSender(event);
    // The page names its language and nothing more: the words of the dialog are the shell's own.
    const title = WORDS[spoken(asked?.language)].chooseFolder;
    const result = await dialog.showOpenDialog(window, { title, properties: ['openDirectory'] });
    if (result.canceled || !result.filePaths[0]) return null;
    const cwd = realpathSync(result.filePaths[0]);
    if (!statSync(cwd).isDirectory()) throw new Error('The selected path is not a folder.');
    approvedDirectories.add(directoryKey(cwd));
    remember(cwd);
    return cwd;
  });
  ipcMain.handle('columbina:recent-projects', (event) => {
    checkSender(event);
    return preferences.recent.filter(isFolder).map((folder) => ({ path: folder, name: path.basename(folder) }));
  });
  ipcMain.handle('columbina:open-recent', (event, asked) => {
    checkSender(event);
    // Only a folder that the user chose in the dialog at an earlier time, and that the shell wrote down then.
    const kept = typeof asked === 'string'
      ? preferences.recent.find((known) => directoryKey(known) === directoryKey(asked)) : undefined;
    if (kept === undefined) throw new Error('That folder is not among the recent projects.');
    const cwd = realpathSync(kept);
    if (directoryKey(cwd) !== directoryKey(kept)) throw new Error('That folder leads somewhere else than when it was chosen.');
    if (!statSync(cwd).isDirectory()) throw new Error('The selected path is not a folder.');
    approvedDirectories.add(directoryKey(cwd));
    remember(cwd);
    return cwd;
  });
  ipcMain.handle('columbina:start', async (event, options) => {
    checkSender(event);
    // Validate before queueing so an unapproved request cannot stop a running project.
    approvedDirectory(options);
    return enqueueService(async () => {
      if (exiting) throw new Error('The application is closing.');
      const cwd = approvedDirectory(options);
      await stopService();
      if (askingLoginShell) await askingLoginShell;
      if (exiting) throw new Error('The application is closing.');
      const config = serviceCommand();
      mkdirSync(path.dirname(logPath()), { recursive: true });
      const log = createWriteStream(logPath(), { flags: 'a' });
      log.on('error', () => {});
      const child = spawn(config.executable, config.args, {
        cwd, env: config.env, windowsHide: true,
        detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
      });
      service = child;
      serviceProject = { child, cwd, mode: config.mode };
      lastWords = listenForErrors(child);
      const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
      lines.on('line', (line) => { if (service === child) tell('columbina:line', line); });
      child.stderr.pipe(log);
      child.stdin.on('error', (error) => {
        if (service === child) tell('columbina:closed', `The service input closed: ${error.message}`);
      });
      child.on('error', (error) => {
        if (service === child) tell('columbina:closed', `The service could not start: ${error.message}`);
      });
      child.on('close', (code, signal) => {
        lines.close();
        log.end();
        if (service === child) {
          service = null;
          tell('columbina:closed', code === 0 ? 'The service has shut down.' : `The service process exited (${signal || code}).`);
        }
      });
      await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
      return { mode: config.mode, cwd };
    });
  });
  ipcMain.on('columbina:send', (event, line) => {
    try {
      checkSender(event);
      if (typeof line !== 'string' || /[\r\n]/.test(line)) throw new Error('Each protocol message must occupy one line.');
      if (!service?.stdin.writable) throw new Error('Service is not connected.');
      service.stdin.write(`${line}\n`, 'utf8');
    } catch (error) { tell('columbina:closed', error.message); }
  });
  ipcMain.handle('columbina:stop', async (event) => {
    checkSender(event);
    await enqueueService(stopService);
    if (exiting) finishWindowClose();
  });
  ipcMain.handle('columbina:local-music', async (event) => {
    checkSender(event);
    const musicPath = path.join(realpathSync(app.getPath('userData')), 'music', 'somnias-a-luna.m4a');
    let file;
    try {
      if (directoryKey(await realpath(musicPath)) !== directoryKey(musicPath)) {
        throw new Error('The default music file must stay in the application music folder.');
      }
      file = await open(musicPath, 'r');
      const details = await file.stat();
      if (!details.isFile() || details.size > 64 * 1024 * 1024) throw new Error('The default music file is too large or is not a file.');
      const bytes = await file.readFile();
      return { name: 'Somnias a Luna', mimeType: 'audio/mp4', bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
    } catch (error) {
      if (error.code === 'ENOENT') return null;
      throw error;
    } finally { await file?.close(); }
  });
  ipcMain.handle('columbina:set-theme', (event, theme) => {
    checkSender(event);
    if (!Object.hasOwn(titleBarThemes, theme)) throw new Error('Unknown application theme.');
    if (process.platform === 'win32' || process.platform === 'linux') window.setTitleBarOverlay(titleBarThemes[theme]);
    window.setBackgroundColor(titleBarThemes[theme].color);
    // The next start opens in this theme, before the page has read anything.
    if (preferences.theme !== theme) {
      preferences.theme = theme;
      keepPreferences();
    }
  });
  ipcMain.handle('columbina:service-tail', (event) => {
    checkSender(event);
    // A line that is still being written is told of as far as it has come.
    const rest = plain(lastWords.rest);
    return rest ? [...lastWords.lines.slice(1 - TAIL_LINES), short(rest, TAIL_LINE_CHARS)] : [...lastWords.lines];
  });
  ipcMain.handle('columbina:show-log', (event) => {
    checkSender(event);
    shell.showItemInFolder(logPath());
  });
  ipcMain.handle('columbina:notify', (event, message) => {
    checkSender(event);
    // What is shown may hold what a model wrote, such as a command: one line, and not a long one.
    const said = (value, most) => (typeof value === 'string' ? short(plain(value), most) : '');
    const title = said(message?.title, 80);
    if (!title) throw new Error('A notification needs a title.');
    // Whoever looks at the window sees it there.
    if (window.isFocused()) return false;
    window.flashFrame(true);
    if (!flashing) {
      flashing = true;
      window.once('focus', () => { flashing = false; window.flashFrame(false); });
    }
    if (Notification.isSupported()) {
      shown = new Notification({ title, body: said(message?.body, 240) });
      shown.on('click', () => {
        if (window.isMinimized()) window.restore();
        window.show();
        window.focus();
      });
      shown.show();
    }
    return true;
  });
  ipcMain.handle('columbina:services', async (event) => {
    checkSender(event);
    if (!programCommand([])) return null;
    // The document says which services there are and where their keys come from. It holds no key.
    const document = await runProgram(['setup', '--list']);
    if (typeof document.error === 'string') throw new Error(document.error);
    mainListed = namesAndPages(document).names;
    return document;
  });
  ipcMain.handle('columbina:models', async (event, name) => {
    checkSender(event);
    // Only a service that the window was told of. The document has no key: the program asks with
    // the one that a session would send, so no key comes this way.
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    return runProgram(['setup', '--check'], JSON.stringify({ service: name }));
  });
  ipcMain.handle('columbina:choose-model', async (event, asked) => {
    checkSender(event);
    const name = asked?.service;
    const model = asked?.model;
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    // A name as a service gives one: of one piece, and not long. What looks like a key is none:
    // a model is kept as it is written, and not in the keyring. The program checks the rest.
    if (typeof model !== 'string' || !/^\S{1,200}$/.test(model) || /[\x00-\x1f\x7f]/.test(model)
      || /^sk-/i.test(model) || /^[A-Za-z0-9_]{40,}$/.test(model)) throw new Error('That is no name of a model.');
    // Without a key, the key that is stored for the service stays as it is.
    return runProgram(['setup', '--from-stdin'], JSON.stringify({ service: name, model }));
  });
  ipcMain.handle('columbina:choose-attachments', async (event, asked) => {
    checkSender(event);
    const title = WORDS[spoken(asked?.language)].chooseFiles;
    const result = await dialog.showOpenDialog(window, { title, properties: ['openFile', 'multiSelections'] });
    if (result.canceled) return null;
    return attach(result.filePaths);
  });
  ipcMain.handle('columbina:attach-paths', async (event, paths) => {
    checkSender(event);
    // The paths of files that were dropped on the window, as the preload has them from the files
    // themselves. The page has no way of its own to name a path here.
    if (!Array.isArray(paths) || paths.length > ATTACHMENTS_HELD_MOST
      || paths.some((each) => typeof each !== 'string' || !path.isAbsolute(each))) throw new Error('Those are no files that were dropped.');
    return attach(paths);
  });
  ipcMain.handle('columbina:forget-attachments', (event, ids) => {
    checkSender(event);
    for (const id of Array.isArray(ids) ? ids : []) attachments.delete(id);
  });
  ipcMain.handle('columbina:keep-attachments', async (event, ids) => {
    checkSender(event);
    if (!Array.isArray(ids) || ids.length === 0 || ids.length > ATTACHMENTS_MOST) throw new Error('That is no set of files for a message.');
    const chosen = ids.map((id) => (typeof id === 'string' ? attachments.get(id) : undefined));
    if (chosen.includes(undefined)) throw new Error('A file that is to go with the message was not chosen.');
    const project = projectOfTheService();
    if (project === null) throw new Error('No project is open, so the files have nowhere to be kept.');
    // Each file is as it was chosen, or none is copied.
    for (const file of chosen) {
      const found = await stat(file.path).catch(() => null);
      if (found === null) throw new Error(`${file.name} is not there any more.`);
      if (!found.isFile() || found.size > ATTACHMENT_MOST_BYTES) throw new Error(`${file.name} is no longer a file that can go with a message.`);
    }
    const root = path.join(project, ...ATTACHMENTS_FOLDER);
    const folder = path.join(root, sentAt(new Date()));
    await mkdir(folder, { recursive: true });
    // What is kept for the messages stays out of the history of the project.
    await writeFile(path.join(root, '.gitignore'), '*\n', { flag: 'wx' }).catch((error) => {
      if (error?.code !== 'EEXIST') throw error;
    });
    const kept = [];
    for (const file of chosen) kept.push([...ATTACHMENTS_FOLDER, path.basename(folder), await copiedInto(folder, file)].join('/'));
    // The files are still held until the page lets go of them: a message that does not go is sent
    // again with them.
    return { kept };
  });
  ipcMain.handle('columbina:set-up-service', async (event, asked) => {
    checkSender(event);
    if (!programCommand([])) throw new Error(NO_SETUP);
    if (keyWindow) {
      keyWindow.focus();
      return keyClosed;
    }
    keyAsked = {
      service: typeof asked?.service === 'string' && asked.service ? asked.service : null,
      language: spoken(asked?.language),
    };
    keyChanged = false;
    keyListed = { names: [], pages: [] };
    // A window of its own, so that the key is never in the window that shows what a model wrote.
    const opened = new BrowserWindow({
      parent: window, modal: true, width: 540, height: 680, resizable: false, minimizable: false, maximizable: false,
      show: false, backgroundColor: titleBarThemes[preferences.theme].color, title: 'Columbina',
      icon: path.join(__dirname, 'icon.ico'),
      webPreferences: { preload: path.join(__dirname, 'key-preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true },
    });
    keyWindow = opened;
    opened.setMenuBarVisibility(false);
    closeToTheWorld(opened);
    opened.once('ready-to-show', () => opened.show());
    keyClosed = new Promise((resolve) => {
      opened.once('closed', () => {
        if (keyWindow === opened) keyWindow = null;
        // The main window is told that something was stored, and nothing of what.
        resolve({ changed: keyChanged });
      });
    });
    void opened.loadFile(keyPath);
    return keyClosed;
  });
  ipcMain.handle('columbina-key:context', async (event) => {
    checkKeySender(event);
    const list = await runProgram(['setup', '--list']);
    keyListed = namesAndPages(list);
    return { ...keyAsked, theme: preferences.theme, list };
  });
  ipcMain.handle('columbina-key:check', async (event, given) => {
    checkKeySender(event);
    // What the program reads to check a service, each of its kind. A model and a size are not checked.
    const document = {};
    for (const name of ['service', 'base_url', 'api_key']) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    return runProgram(['setup', '--check'], JSON.stringify(document));
  });
  ipcMain.handle('columbina-key:forget', async (event, name) => {
    checkKeySender(event);
    // Only a service that the window was told of: the name goes on a command line.
    if (typeof name !== 'string' || !keyListed.names.includes(name)) throw new Error('That service is not among those that are set up.');
    const answer = await runProgram(['setup', '--forget', name, '--json']);
    if (answer.forgotten) keyChanged = true;
    return answer;
  });
  ipcMain.handle('columbina-key:open-page', async (event, address) => {
    checkKeySender(event);
    // Only a page that a service links to, as the program names it, and over https.
    if (typeof address !== 'string' || !keyListed.pages.includes(address)) throw new Error('That page is none that a service links to.');
    const url = new URL(address);
    if (url.protocol !== 'https:') throw new Error('Only pages over https are opened.');
    await shell.openExternal(url.href);
  });
  ipcMain.handle('columbina-key:store', async (event, given) => {
    checkKeySender(event);
    // The five things that the program reads, each of its kind. The program checks the rest.
    const document = {};
    for (const name of ['service', 'base_url', 'model', 'api_key']) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    if (Number.isInteger(given?.context_window)) document.context_window = given.context_window;
    const answer = await runProgram(['setup', '--from-stdin'], JSON.stringify(document));
    if (answer.stored) keyChanged = true;
    return answer;
  });
  ipcMain.handle('columbina-key:close', (event) => {
    checkKeySender(event);
    keyWindow.close();
  });
  ipcMain.handle('columbina:open-external', async (event, address) => {
    checkSender(event);
    if (typeof address !== 'string') throw new Error('The link address must be a string.');
    const url = new URL(address);
    if (url.protocol !== 'https:' && url.protocol !== 'http:') throw new Error('Only http and https links can be opened.');
    await shell.openExternal(url.href);
  });
  ipcMain.handle('columbina:github-status', async (event) => {
    checkSender(event);
    if (!programCommand([])) return null;
    // Who is signed in, the picture as an address of data, and what Columbina may reach. No token.
    const answer = await runProgram(['login', '--status', '--json'], '', ACCOUNT_OUTPUT_MOST);
    return { ...answer, skipped: preferences.githubSkipped };
  });
  ipcMain.handle('columbina:github-sign-in', async (event) => {
    checkSender(event);
    if (askingLoginShell) await askingLoginShell;
    const command = programCommand(['login', 'github', '--json']);
    if (!command) throw new Error(NO_SETUP);
    // One sign-in at a time, and the page hears the one that runs. One that is begun takes the
    // place of one that runs: nobody waits for that one any more, as after the page was loaded again.
    stopSigningIn();
    const child = spawn(command.executable, command.args, {
      cwd: app.getPath('home'), env: command.env, windowsHide: true,
      detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
    });
    signingIn = child;
    const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
    lines.on('line', (line) => {
      // Each line is a document of JSON: the code first, how the sign-in ended last. No line holds a token.
      if (signingIn !== child || line.length > ACCOUNT_OUTPUT_MOST) return;
      let said = null;
      try { said = JSON.parse(line); } catch { return; }
      if (said !== null && typeof said === 'object' && !Array.isArray(said)) tell('columbina:github', said);
    });
    // What a person would be shown. It is taken so that the program is not held up, and kept nowhere.
    child.stderr.on('data', () => {});
    child.stdin.on('error', () => {});
    child.on('close', () => {
      lines.close();
      if (signingIn !== child) return;
      signingIn = null;
      tell('columbina:github', { ended: true });
    });
    let started = false;
    await new Promise((resolve, reject) => {
      child.once('spawn', () => { started = true; resolve(); });
      child.on('error', (error) => {
        // A program that started ends by itself, and its end is told of. One that did not never will.
        if (started) return;
        if (signingIn === child) signingIn = null;
        reject(new Error(`The sign-in could not be started: ${error.message}`));
      });
    });
  });
  ipcMain.handle('columbina:github-cancel', (event) => {
    checkSender(event);
    stopSigningIn();
  });
  ipcMain.handle('columbina:github-sign-out', async (event) => {
    checkSender(event);
    return runProgram(['logout', 'github', '--json']);
  });
  ipcMain.handle('columbina:skip-github', (event, skipped) => {
    checkSender(event);
    preferences.githubSkipped = skipped === true;
    keepPreferences();
  });
}
/**
 * Stop the sign-in to GitHub, if one runs. What it writes from now on, and its end, are told of
 * to nobody. Its input is closed, which the program takes for a stop, and the program and what
 * it started are ended where it has not ended a few seconds later.
 */
function stopSigningIn() {
  const child = signingIn;
  if (!child) return;
  signingIn = null;
  child.stdin.end();
  const timer = setTimeout(() => killTree(child), SIGN_IN_STOP_MS);
  child.once('close', () => clearTimeout(timer));
}
function finishWindowClose() {
  clearTimeout(closeTimer);
  if (window && !window.isDestroyed()) window.destroy();
  app.quit();
}
function requestWindowClose() {
  if (exiting) return;
  exiting = true;
  tell('columbina:shutdown');
  // A responsive renderer finishes immediately through the stop handler. This
  // timer only covers a missing renderer or an unanswered shutdown request.
  closeTimer = setTimeout(() => { void enqueueService(stopService).finally(finishWindowClose); }, 1800);
}
app.whenReady().then(() => {
  if (!existsSync(indexPath)) throw new Error('Build the desktop client before starting Electron: npm run build');
  installHandlers();
  preferences = readPreferences();
  // The window opens in the theme that was chosen last, so that it does not begin in another one.
  const theme = titleBarThemes[preferences.theme];
  if (process.platform === 'darwin') {
    // On macOS the menu stands at the top of the screen, and the keys that copy and paste, close
    // the window or leave go through it. These are all the menu that the application has there.
    Menu.setApplicationMenu(Menu.buildFromTemplate([{ role: 'appMenu' }, { role: 'fileMenu' }, { role: 'editMenu' }, { role: 'windowMenu' }]));
    if (app.isPackaged) {
      // A package is opened from the Finder, with a short PATH and no proxy. The login shell is asked while the window opens.
      askingLoginShell = askLoginShell().then((found) => { loginShell = found; askingLoginShell = null; });
    } else {
      // A package has its icon in itself. In development the Dock would show that of Electron.
      app.dock?.setIcon(path.resolve(__dirname, '../dist/columbina-icon.png'));
    }
  }
  window = new BrowserWindow({
    width: 1380, height: 920, minWidth: 860, minHeight: 640, backgroundColor: theme.color, title: 'Columbina',
    icon: path.join(__dirname, 'icon.ico'),
    ...windowBar(theme),
    webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true, autoplayPolicy: 'no-user-gesture-required' },
  });
  window.setMenuBarVisibility(false);
  if (process.platform === 'darwin') {
    // A sheet, such as the dialog for a folder or the window for the key, comes down from under the
    // bar that the page draws, and not over it.
    window.setSheetOffset(theme.height);
    // In full screen the buttons of the window are hidden, and the page gives their room back.
    window.on('enter-full-screen', () => tell('columbina:full-screen', true));
    window.on('leave-full-screen', () => tell('columbina:full-screen', false));
  }
  // The production window only loads bundled assets. Vite's development CSP
  // can permit its local HMR socket; the desktop window has no network channel.
  window.webContents.session.webRequest.onBeforeRequest((details, callback) => {
    // User-selected music is stored locally and played through an object URL.
    if (details.url.startsWith('blob:') && details.resourceType === 'media') {
      callback({ cancel: false }); return;
    }
    try {
      const relative = path.relative(path.dirname(indexPath), fileURLToPath(details.url));
      callback({ cancel: relative.startsWith('..') || path.isAbsolute(relative) });
    } catch { callback({ cancel: true }); }
  });
  window.webContents.session.webRequest.onHeadersReceived((details, callback) => callback({
    responseHeaders: {
      ...details.responseHeaders,
      'Content-Security-Policy': ["default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; font-src 'self'; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'"],
    },
  }));
  closeToTheWorld(window);
  window.on('close', (event) => { if ((service || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
  window.webContents.on('render-process-gone', () => { stopSigningIn(); void enqueueService(stopService); });
  void window.loadFile(indexPath);
});
app.on('before-quit', (event) => { if ((service || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
app.on('window-all-closed', () => { exiting = true; stopSigningIn(); void enqueueService(stopService).finally(() => app.quit()); });
