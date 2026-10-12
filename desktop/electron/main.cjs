const { app, BrowserWindow, dialog, ipcMain, shell } = require('electron');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');
const { createWriteStream, existsSync, mkdirSync, realpathSync, statSync } = require('node:fs');
const { open, realpath } = require('node:fs/promises');
const path = require('node:path');
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
// The bar of the window in each theme: the furthest surface of `src/ui/tokens.css`, and its text.
const titleBarThemes = {
  moonveil: { color: '#090b12', symbolColor: '#eef0f8', height: 40 },
  moonwhite: { color: '#f5f4f0', symbolColor: '#1b1c24', height: 40 },
};

app.setName('Columbina');
if (process.platform === 'win32') app.setAppUserModelId('dev.columbina.desktop');

function checkSender(event) {
  if (!window || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== indexUrl) {
    throw new Error('IPC sender is not the application window.');
  }
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
function installHandlers() {
  ipcMain.handle('columbina:select-folder', async (event) => {
    checkSender(event);
    const result = await dialog.showOpenDialog(window, { title: 'Choose a project folder', properties: ['openDirectory'] });
    if (result.canceled || !result.filePaths[0]) return null;
    const cwd = realpathSync(result.filePaths[0]);
    if (!statSync(cwd).isDirectory()) throw new Error('The selected path is not a folder.');
    approvedDirectories.add(directoryKey(cwd));
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
      const logDirectory = app.getPath('logs');
      mkdirSync(logDirectory, { recursive: true });
      const log = createWriteStream(path.join(logDirectory, 'columbina-service.log'), { flags: 'a' });
      log.on('error', () => {});
      const child = spawn(config.executable, config.args, {
        cwd, env: config.env, windowsHide: true,
        detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
      });
      service = child;
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
  window = new BrowserWindow({
    width: 1380, height: 920, minWidth: 860, minHeight: 640, backgroundColor: titleBarThemes.moonveil.color, title: 'Columbina',
    icon: path.join(__dirname, 'icon.ico'),
    titleBarStyle: 'hidden', titleBarOverlay: titleBarThemes.moonveil,
    webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true, autoplayPolicy: 'no-user-gesture-required' },
  });
  window.setMenuBarVisibility(false);
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
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', (event) => event.preventDefault());
  window.webContents.on('will-attach-webview', (event) => event.preventDefault());
  window.on('close', (event) => { if ((service || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
  window.webContents.on('render-process-gone', () => { void enqueueService(stopService); });
  void window.loadFile(indexPath);
});
app.on('before-quit', (event) => { if ((service || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
app.on('window-all-closed', () => { exiting = true; void enqueueService(stopService).finally(() => app.quit()); });
