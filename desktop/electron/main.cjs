const { app, BrowserWindow, Menu, Notification, dialog, ipcMain, shell } = require('electron');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');
const {
  createWriteStream, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, statSync, writeFileSync,
} = require('node:fs');
const { open, realpath } = require('node:fs/promises');
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
  en: { chooseFolder: 'Choose a project folder' },
  ja: { chooseFolder: 'プロジェクトフォルダーを選択' },
  'zh-CN': { chooseFolder: '选择项目文件夹' },
};
/** The language that a page names, if the shell speaks it, and English if not. */
const spoken = (language) => (typeof language === 'string' && Object.hasOwn(WORDS, language) ? language : 'en');
// The window for the key while it is open, what it was opened for, whether something was
// stored from it, and the promise that says so when it has closed.
let keyWindow = null;
let keyAsked = { service: null, language: 'en' };
let keyStored = false;
let keyClosed = null;
// What the service of the moment wrote last for errors. It is told of when the service has ended.
let lastWords = { lines: [], rest: '' };
// What is kept from one start to the next. Read when the application is ready.
let preferences = { theme: 'moonveil', recent: [] };
let flashing = false;
// The notification that shows, held so that it is not cleared away while it does.
let shown = null;

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
  return { theme, recent };
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
function serviceCommand() {
  const executable = process.env.COLUMBINA_SERVICE_EXECUTABLE;
  if (!executable) return { executable: process.execPath, args: [path.join(__dirname, 'mock-service.mjs')], mode: 'mock', env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' } };
  if (!path.isAbsolute(executable)) throw new Error('COLUMBINA_SERVICE_EXECUTABLE must be an absolute executable path.');
  const args = process.env.COLUMBINA_SERVICE_ARGS ? JSON.parse(process.env.COLUMBINA_SERVICE_ARGS) : ['-m', 'columbina', 'serve'];
  if (!Array.isArray(args) || args.some((value) => typeof value !== 'string')) throw new Error('COLUMBINA_SERVICE_ARGS must be a JSON array of strings.');
  return { executable, args, mode: 'service', env: { ...process.env } };
}
/**
 * The command that sets a service up, made from the command of the service process.
 * Null where there is none to make it from: in a replay, or where the command does not serve.
 */
function setupCommand(more) {
  const config = serviceCommand();
  const serves = config.args.indexOf('serve');
  if (config.mode !== 'service' || serves < 0) return null;
  // What follows `serve` was meant for serving, such as a profile.
  return { executable: config.executable, args: [...config.args.slice(0, serves), 'setup', ...more], env: config.env };
}
/**
 * Run `columbina setup` in a form that asks nothing, and return the document that it answers with.
 *
 * What it is given goes to its standard input and nowhere else: not to its command line, not
 * into its environment, not into a file. That is how a key reaches it.
 */
function runSetup(more, input = '') {
  const command = setupCommand(more);
  if (!command) return Promise.reject(new Error(NO_SETUP));
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
      if (written.length > SETUP_OUTPUT_MOST) {
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
    if (!setupCommand([])) return null;
    // The document says which services there are and where their keys come from. It holds no key.
    const document = await runSetup(['--list']);
    if (typeof document.error === 'string') throw new Error(document.error);
    return document;
  });
  ipcMain.handle('columbina:set-up-service', async (event, asked) => {
    checkSender(event);
    if (!setupCommand([])) throw new Error(NO_SETUP);
    if (keyWindow) {
      keyWindow.focus();
      return keyClosed;
    }
    keyAsked = {
      service: typeof asked?.service === 'string' && asked.service ? asked.service : null,
      language: spoken(asked?.language),
    };
    keyStored = false;
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
        resolve({ stored: keyStored });
      });
    });
    void opened.loadFile(keyPath);
    return keyClosed;
  });
  ipcMain.handle('columbina-key:context', async (event) => {
    checkKeySender(event);
    return { ...keyAsked, theme: preferences.theme, list: await runSetup(['--list']) };
  });
  ipcMain.handle('columbina-key:store', async (event, given) => {
    checkKeySender(event);
    // The five things that the program reads, each of its kind. The program checks the rest.
    const document = {};
    for (const name of ['service', 'base_url', 'model', 'api_key']) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    if (Number.isInteger(given?.context_window)) document.context_window = given.context_window;
    const answer = await runSetup(['--from-stdin'], JSON.stringify(document));
    if (answer.stored) keyStored = true;
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
    // A package has its icon in itself. In development the Dock would show that of Electron.
    if (!app.isPackaged) app.dock?.setIcon(path.resolve(__dirname, '../dist/columbina-icon.png'));
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
  window.webContents.on('render-process-gone', () => { void enqueueService(stopService); });
  void window.loadFile(indexPath);
});
app.on('before-quit', (event) => { if ((service || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
app.on('window-all-closed', () => { exiting = true; void enqueueService(stopService).finally(() => app.quit()); });
