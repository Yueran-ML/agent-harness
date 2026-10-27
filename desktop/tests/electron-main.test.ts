import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { describe, expect, it, vi } from 'vitest';
import { languages } from '../src/app/locale';
import { languageFor } from '../src/i18n';
import { themes } from '../src/themes';
import { blockOf, themeColour } from './stylesheet';
// The parts of the pets that only look at what they are given are the shell's own here.
import realPets from '../electron/pets.cjs';
import { EngineWorld, loadEngine } from './engineFakes';

const mainPath = fileURLToPath(new URL('../electron/main.cjs', import.meta.url));
const source = readFileSync(mainPath, 'utf8');

const preferencesFile = path.resolve('/columbina-test/userData/preferences.json');
const logFile = path.resolve('/columbina-test/logs/columbina-service.log');

const keyPage = pathToFileURL(path.resolve(path.dirname(mainPath), '../dist/key.html')).href;
const python = path.resolve('/tools/python.exe');

/** What a made-up run of `columbina setup` writes to its standard output, and how it ends. */
interface SetupRun {
  out: string;
  code: number;
}

/** The system that the shell runs on, and whether it runs as a package or from the checkout. */
interface System {
  platform?: 'win32' | 'darwin' | 'linux';
  packaged?: boolean;
  /** The version of the application, 0.1.2 unless it is named. */
  version?: string;
  /** What the login shell of the user writes when it is asked for its PATH. Null is a shell that never ends. */
  loginShell?: string | null;
  /** Whether no other application of Columbina runs, which it does not unless that is said. */
  alone?: boolean;
  /** The language of the system, English unless it is named. */
  locale?: string;
  /** What the application was started with besides its program. */
  argv?: string[];
  /** What happens to the application after its script ran and before it is ready, such as a second start. */
  beforeReady?: (app: EventEmitter) => void;
}

/** What a login shell writes by itself, before it says its PATH. */
const SAID_PATH = 'Last login: Thu Oct 15 on ttys000\n_COLUMBINA_PATH_/opt/homebrew/bin:/usr/bin:/bin_COLUMBINA_PATH_';

/** Where a package keeps what it brings beside the application, such as its Python. */
const resources = path.resolve('/columbina-test/resources');

/** Let what the shell has to do next come about. */
const settled = () => new Promise(resolve => setTimeout(resolve, 0));
/** End a run of the setup program that waits, with this document as its answer. */
function answer(run: { child: any }, document: unknown) {
  run.child.stdout.emit('data', Buffer.from(`${JSON.stringify(document)}\n`, 'utf8'));
  run.child.exitCode = 0;
  run.child.emit('close', 0, null);
}

/**
 * The shell, with everything around it made up.
 *
 * Args:
 *   kept: What the file of preferences holds when the shell starts, as text. Without it there is no such file.
 *   env: The environment of the shell. Without the variable that names the service, the shell replays
 *     in development and runs the Python of the package in a package.
 *   system: The system, Windows unless it is named, and whether the shell is a package, which it is not
 *     unless that is said.
 */
async function shellHarness(kept?: string, env: Record<string, string> = {}, system: System = {}) {
  const platform = system.platform ?? 'win32';
  const handlers = new Map<string, (...args: any[]) => any>();
  // The files that the shell reads and writes whole, by their paths.
  const files = new Map<string, string>();
  if (kept !== undefined) files.set(preferencesFile, kept);
  // The folders that are not there any more.
  const gone = new Set<string>();
  const missing = (file: string) => Object.assign(new Error(`ENOENT: ${file}`), { code: 'ENOENT' });
  // The files that are not there, beside the folders that are gone. Every other file is.
  const absent = new Set<string>();
  // Whether the application starts with the system, as the system keeps it.
  const loginItem = { openAtLogin: false };
  const app = Object.assign(new EventEmitter(), {
    requestSingleInstanceLock: vi.fn(() => system.alone ?? true),
    getLocale: vi.fn(() => system.locale ?? 'en-US'),
    setLoginItemSettings: vi.fn((settings: { openAtLogin: boolean }) => { loginItem.openAtLogin = settings.openAtLogin; }),
    getLoginItemSettings: vi.fn((_asked?: unknown) => ({ openAtLogin: loginItem.openAtLogin })),
    setName: vi.fn(), setAppUserModelId: vi.fn(), isPackaged: system.packaged ?? false,
    getVersion: () => system.version ?? '0.1.2',
    dock: { setIcon: vi.fn() },
    whenReady: () => { system.beforeReady?.(app); return Promise.resolve(); }, quit: vi.fn(),
    getPath: vi.fn((name: string) => {
      if (name === 'documents') throw new Error('Documents must never be used as a project.');
      return path.resolve(`/columbina-test/${name}`);
    }),
  });
  const dialog = { showOpenDialog: vi.fn().mockResolvedValue({ canceled: true, filePaths: [] }) };
  const sent: [string, unknown][] = [];
  // Every window that the shell opened, the main one first.
  const windows: any[] = [];
  let window: any;
  class TestWindow extends EventEmitter {
    webContents = Object.assign(new EventEmitter(), {
      mainFrame: { url: pathToFileURL(path.resolve(path.dirname(mainPath), '../dist/index.html')).href },
      isDestroyed: () => false,
      send: (channel: string, value: unknown) => sent.push([channel, value]),
      session: { webRequest: { onBeforeRequest: vi.fn(), onHeadersReceived: vi.fn() } },
      setWindowOpenHandler: vi.fn(),
      setAudioMuted: vi.fn(),
      zoom: 0,
      setZoomLevel: vi.fn((level: number) => { this.webContents.zoom = level; }),
      getZoomLevel: () => this.webContents.zoom,
      reload: vi.fn(),
      toggleDevTools: vi.fn(),
    });
    options: any;
    constructor(options: unknown) { super(); this.options = options; windows.push(this); window ??= this; }
    loadFile = vi.fn((file: string) => { this.webContents.mainFrame.url = pathToFileURL(path.resolve(file)).href; });
    close = vi.fn(() => { this.emit('closed'); });
    focused = true;
    minimized = false;
    isFocused = () => this.focused;
    isMinimized = () => this.minimized;
    flashFrame = vi.fn();
    show = vi.fn();
    hide = vi.fn();
    focus = vi.fn();
    restore = vi.fn();
    isDestroyed = () => false;
    setMenuBarVisibility = vi.fn();
    destroy = vi.fn();
    setBackgroundColor = vi.fn();
    setTitleBarOverlay = vi.fn();
    setSheetOffset = vi.fn();
    showInactive = vi.fn();
    position: [number, number] = [0, 0];
    getPosition = () => this.position;
    setPosition = vi.fn((x: number, y: number) => { this.position = [x, y]; });
    removeMenu = vi.fn();
    fullScreen = false;
    isFullScreen = () => this.fullScreen;
    setFullScreen = vi.fn((full: boolean) => { this.fullScreen = full; });
    /** The handle of the window, as Electron gives it: 8 bytes, of a number larger than JavaScript counts exactly. */
    handle = (() => { const bytes = Buffer.alloc(8); bytes.writeBigUInt64LE(0xFFFF_FFFF_0000_1234n); return bytes; })();
    getNativeWindowHandle = vi.fn(() => this.handle);
  }
  // The engines of the studio that the shell starts and reaches.
  const engines = new EngineWorld();
  let live = 0;
  let peakLive = 0;
  const children: any[] = [];
  // The runs of `columbina setup`: what each was started with and given, and what it answers.
  const setups: { args: string[]; cwd: string; input: string | null; child: any }[] = [];
  // The runs of `columbina login github`, which write their lines through a reader of lines.
  const signIns: any[] = [];
  // The runs of an assistant, `columbina feishu --json` or `columbina wecom --json`, which write their
  // lines through a reader of lines too, and end when the test says so.
  const bots: any[] = [];
  const lineReaders: any[] = [];
  const setup = {
    answer: ((): SetupRun => ({ out: '{"use": null, "services": []}\n', code: 0 })) as (args: string[], input: string) => SetupRun | null,
  };
  // The runs of the login shell, asked for its PATH, and what it writes. Null is a shell that says nothing and does not end.
  const logins: any[] = [];
  const says = system.loginShell === undefined ? SAID_PATH : system.loginShell;
  const spawn = vi.fn((_executable, args: string[], options) => {
    if (args[0] === '-ilc') {
      const child = Object.assign(new EventEmitter(), { stdout: new EventEmitter(), kill: vi.fn() });
      logins.push(child);
      if (says !== null) {
        queueMicrotask(() => {
          if (says) child.stdout.emit('data', Buffer.from(says, 'utf8'));
          child.emit('close', 0, null);
        });
      }
      return child;
    }
    if (args.includes('login') && !args.includes('--status')) {
      // A sign-in to GitHub: it writes lines while it waits, and ends when the test says so.
      const child = Object.assign(new EventEmitter(), {
        pid: 700 + signIns.length, exitCode: null as number | null, signalCode: null, args, options,
        stdout: new EventEmitter(), stderr: new EventEmitter(),
        stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
      });
      signIns.push(child);
      queueMicrotask(() => child.emit('spawn'));
      return child;
    }
    // A run of an assistant, which the shell starts with `--json` first and maybe a language after.
    const assistant = ['feishu', 'wecom', 'qq'].find(kind => args.includes(kind));
    if (assistant && args[args.indexOf(assistant) + 1] === '--json') {
      const child = Object.assign(new EventEmitter(), {
        pid: 900 + bots.length, exitCode: null as number | null, signalCode: null, args, options,
        stdout: new EventEmitter(), stderr: Object.assign(new EventEmitter(), { pipe: vi.fn() }),
        stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
      });
      bots.push(child);
      queueMicrotask(() => child.emit('spawn'));
      return child;
    }
    if (args.includes('setup') || args.includes('--status') || args.includes('logout') || args.includes('balance') || args.includes('reasoning-language') || args.includes('schedules') || args.includes('worktrees') || assistant) {
      const child = Object.assign(new EventEmitter(), {
        pid: 500 + setups.length, exitCode: null as number | null, signalCode: null,
        stdout: new EventEmitter(), stderr: new EventEmitter(),
        stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
      });
      const run: { args: string[]; cwd: string; input: string | null; child: any } = { args, cwd: options.cwd, input: null, child };
      setups.push(run);
      // The program reads its input to the end, and answers then.
      child.stdin.end.mockImplementation((input?: string) => {
        run.input = input ?? '';
        const answer = setup.answer(args, run.input);
        if (answer === null) return;
        queueMicrotask(() => {
          if (answer.out) child.stdout.emit('data', Buffer.from(answer.out, 'utf8'));
          child.exitCode = answer.code;
          child.emit('close', answer.code, null);
        });
      });
      return child;
    }
    if (args.includes('/pid')) {
      engines.kill(Number(args[args.indexOf('/pid') + 1]));
      return Object.assign(new EventEmitter(), { killed: args });
    }
    // The engine of the studio, which is started for a project and never headless.
    if (args[0] === '--project') return engines.spawn(_executable, args, options);
    live += 1;
    peakLive = Math.max(peakLive, live);
    const child = Object.assign(new EventEmitter(), {
      pid: 100 + children.length, cwd: options.cwd, exitCode: null as number | null, signalCode: null,
      stdout: new EventEmitter(), stderr: Object.assign(new EventEmitter(), { pipe: vi.fn() }),
      stdin: Object.assign(new EventEmitter(), { writable: true, write: vi.fn(), end: vi.fn() }),
    });
    child.stdin.end.mockImplementation(() => queueMicrotask(() => {
      if (child.exitCode !== null) return;
      child.exitCode = 0;
      child.stdin.writable = false;
      live -= 1;
      child.emit('close', 0, null);
    }));
    children.push(child);
    queueMicrotask(() => child.emit('spawn'));
    return child;
  });
  const realpathSync = vi.fn((value: string) => path.resolve(value));
  const mkdirSync = vi.fn();
  const readMusic = vi.fn().mockResolvedValue(Buffer.from([1, 2, 3]));
  const file = {
    stat: vi.fn().mockResolvedValue({ isFile: () => true, size: 3 }),
    readFile: readMusic, close: vi.fn().mockResolvedValue(undefined),
  };
  // The files that can go with a message, and what is copied for them: by their paths, a file of a
  // size or a folder. Every other path is not there.
  const drive = new Map<string, { size: number } | 'folder'>();
  const there = (error: string) => Object.assign(new Error(`EEXIST: ${error}`), { code: 'EEXIST' });
  const disk = {
    realpath: vi.fn(async (value: string) => path.resolve(value)),
    open: vi.fn().mockResolvedValue(file),
    stat: vi.fn(async (value: string) => {
      const found = drive.get(path.resolve(value));
      if (found === undefined) throw missing(value);
      return found === 'folder' ? { isFile: () => false, size: 0 } : { isFile: () => true, size: found.size };
    }),
    copyFile: vi.fn(async (from: string, to: string, _mode?: number) => {
      const source = drive.get(path.resolve(from));
      if (source === undefined || source === 'folder') throw missing(from);
      if (drive.has(path.resolve(to))) throw there(to);
      drive.set(path.resolve(to), { ...source });
    }),
    mkdir: vi.fn(async (_folder: string, _options?: unknown) => undefined),
    writeFile: vi.fn(async (to: string, text: string, options?: { flag?: string }) => {
      if (options?.flag === 'wx' && drive.has(path.resolve(to))) throw there(to);
      drive.set(path.resolve(to), { size: String(text).length });
      files.set(path.resolve(to), String(text));
    }),
  };
  const notifications: any[] = [];
  class TestNotification extends EventEmitter {
    static supported = true;
    static isSupported = () => TestNotification.supported;
    show = vi.fn();
    constructor(public options: unknown) { super(); notifications.push(this); }
  }
  const world = { openExternal: vi.fn(), showItemInFolder: vi.fn(), openPath: vi.fn(async (_folder: string) => '') };
  // The icons that the shell put in the tray, and the pictures that it made for them.
  const trays: any[] = [];
  class TestTray extends EventEmitter {
    setToolTip = vi.fn();
    setContextMenu = vi.fn();
    constructor(public image: unknown) { super(); trays.push(this); }
  }
  const pictures = { createFromPath: vi.fn((file: string) => ({ file, resize: vi.fn((size: unknown) => ({ file, size })) })) };
  // The network, which answers nothing unless a test says what, and the sessions of their own that
  // the shell asks it through, by the names of their partitions.
  const net = { fetch: vi.fn(async (_address: string, _init?: any): Promise<any> => { throw new Error('No network in the tests.'); }) };
  const partitions: string[] = [];
  const session = { fromPartition: vi.fn((name: string) => { partitions.push(name); return { fetch: net.fetch }; }) };
  const writeFileSync = vi.fn((file: string, text: string) => { files.set(path.resolve(file), String(text)); });
  const menu = { buildFromTemplate: vi.fn((template: unknown[]) => ({ template, popup: vi.fn() })), setApplicationMenu: vi.fn() };
  // The pets that the shell finds, by the folders it looks in, and what it writes for other programs.
  const petsFound: any[] = [];
  const pets = {
    listPets: vi.fn((_places: unknown) => petsFound),
    sheetOf: vi.fn((pet: { key: string }) => `data:image/png;base64,${pet.key}`),
    stateOf: realPets.stateOf,
  };
  // The processes that are there, by their ids: the shell, and whatever a test adds.
  const running = new Set([4242]);
  const kill = (pid: number, _signal: number) => {
    // The first process of the system is there, and somebody else's.
    if (pid === 1) throw Object.assign(new Error('EPERM: 1'), { code: 'EPERM' });
    if (!running.has(pid)) throw Object.assign(new Error(`ESRCH: ${pid}`), { code: 'ESRCH' });
  };
  // What the shell does again and again, until it stops: each by its number, and how often.
  const repeated = new Map<number, { act: () => void; ms: number }>();
  let ticket = 0;
  const clock = {
    setInterval: (act: () => void, ms: number) => { ticket += 1; repeated.set(ticket, { act, ms }); return ticket; },
    clearInterval: (id: number | null) => { if (id !== null) repeated.delete(id); },
  };
  // Where the pointer is on the screen.
  const pointer = { x: 0, y: 0 };
  const screen = { getCursorScreenPoint: vi.fn(() => ({ ...pointer })) };
  // What the windows say without waiting for an answer, by the channel.
  const listeners = new Map<string, (...args: any[]) => any>();
  const modules: Record<string, unknown> = {
    electron: {
      app, BrowserWindow: TestWindow, Menu: menu, dialog, shell: world, Notification: TestNotification, session,
      Tray: TestTray, nativeImage: pictures, screen,
      ipcMain: {
        handle: (name: string, callback: (...args: any[]) => any) => handlers.set(name, callback),
        on: (name: string, callback: (...args: any[]) => any) => listeners.set(name, callback),
      },
    },
    'node:child_process': { spawn },
    'node:readline': {
      createInterface: (options: { input: unknown }) => {
        const reader = Object.assign(new EventEmitter(), { close: vi.fn(), input: options.input });
        lineReaders.push(reader);
        return reader;
      },
    },
    'node:fs': {
      constants: { COPYFILE_EXCL: 1 },
      existsSync: (file: string) => !absent.has(path.resolve(file)), mkdirSync, realpathSync,
      statSync: (value: string) => {
        if (gone.has(path.resolve(value))) throw missing(value);
        return { isDirectory: () => true };
      },
      readFileSync: (file: string) => {
        const text = files.get(path.resolve(file));
        if (text === undefined) throw missing(file);
        return text;
      },
      writeFileSync,
      renameSync: (from: string, to: string) => {
        const text = files.get(path.resolve(from));
        if (text === undefined) throw missing(from);
        files.delete(path.resolve(from));
        files.set(path.resolve(to), text);
      },
      createWriteStream: () => Object.assign(new EventEmitter(), { end: vi.fn(), write: vi.fn() }),
    },
    'node:fs/promises': disk,
    'node:path': path,
    'node:url': { pathToFileURL, fileURLToPath },
    'node:string_decoder': { StringDecoder },
    './pets.cjs': pets,
  };
  const shellProcess = { platform, env, execPath: 'Columbina.exe', resourcesPath: resources, argv: ['Columbina.exe', ...(system.argv ?? [])], pid: 4242, kill };
  modules['node:net'] = engines.net;
  modules['./engine.cjs'] = loadEngine(modules, { process: shellProcess, setTimeout, clearTimeout });
  runInNewContext(source, {
    require: (name: string) => { if (!(name in modules)) throw new Error(`Unexpected module: ${name}`); return modules[name]; },
    __dirname: path.dirname(mainPath),
    process: shellProcess,
    // The clock of the tests, so that a test that sets it sets it for the shell too.
    URL, setTimeout, clearTimeout, Date, AbortSignal, ...clock,
  }, { filename: mainPath });
  await Promise.resolve();
  // A second application of Columbina makes no window, and nothing calls it.
  const event = window ? { sender: window.webContents, senderFrame: window.webContents.mainFrame } : { sender: null, senderFrame: null };
  const invoke = (name: string, ...args: unknown[]) => handlers.get(`columbina:${name}`)!(event, ...args);
  const select = async (folder: string) => {
    dialog.showOpenDialog.mockResolvedValueOnce({ canceled: false, filePaths: [folder] });
    return invoke('select-folder') as Promise<string>;
  };
  const unrelated = window ? { sender: window.webContents, senderFrame: { url: window.webContents.mainFrame.url } } : { sender: null, senderFrame: null };
  const preferences = () => JSON.parse(files.get(preferencesFile) ?? 'null');
  // The window for the key, once the shell has opened it, and a call as that window makes it.
  const keyWindow = () => windows[1];
  const asKeyWindow = (name: string, ...args: unknown[]) => handlers.get(`columbina-key:${name}`)!(
    { sender: keyWindow().webContents, senderFrame: keyWindow().webContents.mainFrame }, ...args);
  // The window for the app of Feishu, once the shell has opened it, and a call as that window makes it.
  const feishuWindow = () => windows.filter(each => String(each.options?.webPreferences?.preload).endsWith('feishu-preload.cjs')).at(-1);
  const asFeishuWindow = (name: string, ...args: unknown[]) => handlers.get(`columbina-feishu:${name}`)!(
    { sender: feishuWindow().webContents, senderFrame: feishuWindow().webContents.mainFrame }, ...args);
  // The window for the bot of WeCom, likewise.
  const wecomWindow = () => windows.filter(each => String(each.options?.webPreferences?.preload).endsWith('wecom-preload.cjs')).at(-1);
  const asWecomWindow = (name: string, ...args: unknown[]) => handlers.get(`columbina-wecom:${name}`)!(
    { sender: wecomWindow().webContents, senderFrame: wecomWindow().webContents.mainFrame }, ...args);
  // The window of the pet, once the shell has opened it, and a call as that window makes it.
  const petWindow = () => windows.filter(each => String(each.options?.webPreferences?.preload).endsWith('pet-preload.cjs')).at(-1);
  const asPetWindow = (name: string, ...args: unknown[]) => handlers.get(`columbina-pet:${name}`)!(
    { sender: petWindow().webContents, senderFrame: petWindow().webContents.mainFrame }, ...args);
  // What the main window says without waiting, as it says it.
  const say = (channel: string, ...args: unknown[]) => listeners.get(channel)!(event, ...args);
  return {
    petsFound, pets, pointer, screen, petWindow, asPetWindow, say, listeners, running, repeated,
    // Do once what the shell does again and again every so many milliseconds.
    again: (ms: number) => { for (const one of [...repeated.values()]) if (one.ms === ms) one.act(); },
    handlers, invoke, select, app, window, windows, event, unrelated, children, spawn, realpathSync, mkdirSync, disk, drive, file, sent,
    dialog, files, gone, absent, preferences, writeFileSync, notifications, Notification: TestNotification, system: world, net,
    partitions, trays, pictures,
    setups, setup, keyWindow, asKeyWindow, feishuWindow, asFeishuWindow, wecomWindow, asWecomWindow, menu, logins, signIns, bots,
    lineReaders, engines,
    live: () => live, peakLive: () => peakLive,
  };
}

describe('Electron project and process boundary', () => {
  it('does not start a service on launch or use Documents as a fallback', async () => {
    const shell = await shellHarness();
    expect(shell.spawn).not.toHaveBeenCalled();
    await expect(shell.invoke('start')).rejects.toThrow('Choose a project folder');
    await expect(shell.invoke('start', { cwd: '' })).rejects.toThrow('Choose a project folder');
    await expect(shell.invoke('start', { cwd: path.resolve('/unselected') })).rejects.toThrow('native folder dialog');
    expect(await shell.invoke('select-folder')).toBeNull();
    expect(shell.spawn).not.toHaveBeenCalled();
    expect(shell.app.getPath).not.toHaveBeenCalledWith('documents');
  });

  it('titles the dialog for a folder in the language of the interface', async () => {
    const shell = await shellHarness();
    const titled = async (...asked: unknown[]) => {
      await shell.invoke('select-folder', ...asked);
      const [over, options] = shell.dialog.showOpenDialog.mock.calls.at(-1)!;
      expect(over).toBe(shell.window);
      expect(options.properties).toEqual(['openDirectory']);
      return options.title as string;
    };
    expect(await titled({ language: 'en' })).toBe('Choose a project folder');
    expect(await titled({ language: 'zh-CN' })).toBe('选择项目文件夹');
    expect(await titled({ language: 'ja' })).toBe('プロジェクトフォルダーを選択');
    // The shell has words of its own for every language that the main window can show.
    const shown = languages.map(offered => languageFor(offered.id));
    const titles = new Set<string>();
    for (const language of shown) titles.add(await titled({ language }));
    expect(titles.size).toBe(shown.length);
    // In English where no language is named, or one that the shell does not speak.
    const unknown = [
      undefined, null, 'ja', {}, { language: 'de' }, { language: 'zh' }, { language: ['ja'] }, { language: 'constructor' },
      { language: { toString: () => 'ja' } },
    ];
    for (const asked of unknown) expect(await titled(asked)).toBe('Choose a project folder');
    expect(await titled()).toBe('Choose a project folder');
  });

  it('starts only a directory approved by the native dialog and keeps that service for invalid requests', async () => {
    const shell = await shellHarness();
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).resolves.toEqual({ mode: 'mock', cwd });
    await expect(shell.invoke('start', { cwd: path.resolve('/another-project') })).rejects.toThrow('native folder dialog');
    expect(shell.live()).toBe(1);
    expect(shell.children[0].cwd).toBe(cwd);
    await shell.invoke('stop');
    expect(shell.live()).toBe(0);
  });

  it('rechecks a selected directory if its real path changes before starting', async () => {
    const shell = await shellHarness();
    const cwd = await shell.select('/selected-project');
    shell.realpathSync.mockImplementation((value: string) => value === cwd ? path.resolve('/outside-project') : path.resolve(value));
    await expect(shell.invoke('start', { cwd })).rejects.toThrow('native folder dialog');
    expect(shell.spawn).not.toHaveBeenCalled();
  });

  it('serializes overlapping starts and stops so every child is accounted for', async () => {
    const shell = await shellHarness();
    const first = await shell.select('/first-project');
    const second = await shell.select('/second-project');
    const startingFirst = shell.invoke('start', { cwd: first });
    const startingSecond = shell.invoke('start', { cwd: second });
    const stopping = shell.invoke('stop');
    await Promise.all([startingFirst, startingSecond, stopping]);
    expect(shell.spawn).toHaveBeenCalledTimes(2);
    expect(shell.peakLive()).toBe(1);
    expect(shell.live()).toBe(0);
    expect(shell.children.every(child => child.stdin.end.mock.calls.length === 1)).toBe(true);
  });

  it('rejects another frame before showing a dialog, starting a service, or reading music', async () => {
    const shell = await shellHarness();
    const unrelatedFrame = { sender: shell.event.sender, senderFrame: { url: shell.event.senderFrame.url } };
    for (const method of ['select-folder', 'start', 'start-chat', 'local-music']) {
      await expect(shell.handlers.get(`columbina:${method}`)!(unrelatedFrame, { cwd: '/selected-project' })).rejects.toThrow('IPC sender');
    }
    expect(shell.spawn).not.toHaveBeenCalled();
    expect(shell.disk.open).not.toHaveBeenCalled();
  });

  it('starts a chat in a folder of its own beside the home of the program, with the profile for it', async () => {
    const shell = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python });
    const chat = path.join(path.resolve('/columbina-test/home'), 'Columbina', 'chat');
    await expect(shell.invoke('start-chat')).resolves.toEqual({ mode: 'service', cwd: chat });
    expect(shell.mkdirSync).toHaveBeenCalledWith(chat, { recursive: true });
    const [executable, args, options] = shell.spawn.mock.calls.at(-1)!;
    expect([executable, args, options.cwd]).toEqual([python, ['-m', 'columbina', 'serve', '--profile', 'chat'], chat]);
    // It is no project: it is not among the recent ones, and it cannot be started as one.
    expect(await shell.invoke('recent-projects')).toEqual([]);
    await expect(shell.invoke('start', { cwd: chat })).rejects.toThrow('native folder dialog');
    // Where the home of the program is elsewhere, the folder of a chat is beside it there.
    const elsewhere = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python, COLUMBINA_HOME: path.resolve('/data/.columbina') });
    expect((await elsewhere.invoke('start-chat')).cwd).toBe(path.join(path.resolve('/data'), 'Columbina', 'chat'));
  });

  it('starts a chat after the project that runs has stopped, and a replay for it as for a project', async () => {
    const shell = await shellHarness();
    const project = await shell.select('/selected-project');
    await shell.invoke('start', { cwd: project });
    const started = await shell.invoke('start-chat');
    expect(started.mode).toBe('mock');
    expect(shell.children[0].stdin.end).toHaveBeenCalledOnce();
    expect(shell.peakLive()).toBe(1);
    // A replay takes no profile.
    expect(shell.spawn.mock.calls.at(-1)![1]).toEqual(shell.spawn.mock.calls[0][1]);
    expect(shell.children[1].cwd).toBe(started.cwd);
  });

  it('starts a chat with what the service is given, and no profile where the command does not serve', async () => {
    const serves = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python, COLUMBINA_SERVICE_ARGS: JSON.stringify(['-m', 'columbina', 'serve', '--verbose']) });
    await serves.invoke('start-chat');
    expect(serves.spawn.mock.calls.at(-1)![1]).toEqual(['-m', 'columbina', 'serve', '--verbose', '--profile', 'chat']);
    const other = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python, COLUMBINA_SERVICE_ARGS: JSON.stringify(['replay.py']) });
    await other.invoke('start-chat');
    expect(other.spawn.mock.calls.at(-1)![1]).toEqual(['replay.py']);
  });

  it('finishes closing as soon as the renderer stops the service', async () => {
    const shell = await shellHarness();
    await shell.invoke('start', { cwd: await shell.select('/selected-project') });
    const preventDefault = vi.fn();
    shell.window.emit('close', { preventDefault });
    expect(preventDefault).toHaveBeenCalled();
    expect(shell.sent.at(-1)?.[0]).toBe('columbina:shutdown');
    await shell.invoke('stop');
    expect(shell.window.destroy).toHaveBeenCalledOnce();
    expect(shell.live()).toBe(0);
  });
});

describe('Electron window and the world', () => {
  it('opens no other window, goes to no other page and holds no page inside its own', async () => {
    const shell = await shellHarness();
    const page = shell.window.webContents;
    expect(page.setWindowOpenHandler).toHaveBeenCalledOnce();
    expect(page.setWindowOpenHandler.mock.calls[0][0]({ url: 'https://example.com' })).toEqual({ action: 'deny' });
    for (const happening of ['will-navigate', 'will-attach-webview']) {
      const event = { preventDefault: vi.fn() };
      page.emit(happening, event);
      expect(event.preventDefault).toHaveBeenCalledOnce();
    }
  });
});

describe('Electron fixed local music reader', () => {
  it('reads only the fixed music asset, ignoring renderer path arguments', async () => {
    const shell = await shellHarness();
    const music = await shell.invoke('local-music', '/unrelated-secret');
    expect(shell.disk.open).toHaveBeenCalledWith(path.resolve('/columbina-test/userData/music/somnias-a-luna.m4a'), 'r');
    expect(music.name).toBe('Somnias a Luna');
    expect(music.mimeType).toBe('audio/mp4');
    expect(Array.from(new Uint8Array(music.bytes))).toEqual([1, 2, 3]);
    expect(shell.file.close).toHaveBeenCalledOnce();
  });

  it('returns null for an absent asset and rejects redirected or oversized files', async () => {
    const shell = await shellHarness();
    shell.disk.realpath.mockRejectedValueOnce(Object.assign(new Error('missing'), { code: 'ENOENT' }));
    await expect(shell.invoke('local-music')).resolves.toBeNull();
    shell.disk.realpath.mockResolvedValueOnce(path.resolve('/outside-music.m4a'));
    await expect(shell.invoke('local-music')).rejects.toThrow('application music folder');
    expect(shell.disk.open).not.toHaveBeenCalled();
    shell.file.stat.mockResolvedValueOnce({ isFile: () => true, size: 65 * 1024 * 1024 });
    await expect(shell.invoke('local-music')).rejects.toThrow('too large');
    expect(shell.file.readFile).not.toHaveBeenCalled();
    expect(shell.file.close).toHaveBeenCalledOnce();
  });
});

describe('Electron window themes', () => {
  // The bar of the window is drawn by the shell, in the furthest surface of the page and its text.
  const bar = (theme: string) => ({
    color: themeColour(theme, 'ink-0'), symbolColor: themeColour(theme, 'moon-0'), height: 40,
  });

  it('opens in the theme that the page begins with, so that nothing flashes before the page is drawn', async () => {
    const shell = await shellHarness();
    expect(shell.window.options.backgroundColor).toBe(themeColour(themes[0], 'ink-0'));
    expect(shell.window.options.titleBarOverlay).toEqual(bar(themes[0]));
  });

  it('draws the bar of the window in the colours of each theme of the page', async () => {
    const shell = await shellHarness();
    for (const theme of themes) {
      shell.invoke('set-theme', theme);
      expect(shell.window.setTitleBarOverlay).toHaveBeenLastCalledWith(bar(theme));
      expect(shell.window.setBackgroundColor).toHaveBeenLastCalledWith(bar(theme).color);
    }
    expect(shell.window.setTitleBarOverlay).toHaveBeenCalledTimes(themes.length);
  });

  it('refuses a theme that it does not know, and leaves the window as it is', async () => {
    const shell = await shellHarness();
    for (const theme of ['frostlight', 'silvermoon', '', 'MOONVEIL', '__proto__', 'constructor', 'toString', undefined, null, 7]) {
      expect(() => shell.invoke('set-theme', theme)).toThrow('Unknown application theme.');
    }
    expect(shell.window.setTitleBarOverlay).not.toHaveBeenCalled();
    expect(shell.window.setBackgroundColor).not.toHaveBeenCalled();
  });

  it('takes a theme only from its own window', async () => {
    const shell = await shellHarness();
    const unrelatedFrame = { sender: shell.event.sender, senderFrame: { url: shell.event.senderFrame.url } };
    expect(() => shell.handlers.get('columbina:set-theme')!(unrelatedFrame, themes[0])).toThrow('IPC sender');
    expect(shell.window.setTitleBarOverlay).not.toHaveBeenCalled();
  });
});

describe('the window on each system', () => {
  it('leaves the buttons of Windows over the right end of the bar, and the menu as it is', async () => {
    const shell = await shellHarness();
    expect(shell.window.options.titleBarStyle).toBe('hidden');
    expect(shell.window.options.titleBarOverlay).toEqual(expect.objectContaining({ height: 40 }));
    expect(shell.window.options).not.toHaveProperty('trafficLightPosition');
    expect(shell.menu.setApplicationMenu).not.toHaveBeenCalled();
    expect(shell.window.setSheetOffset).not.toHaveBeenCalled();
    expect(shell.app.dock.setIcon).not.toHaveBeenCalled();
  });

  it('stands the buttons of macOS at the left end of the bar, in the middle of its height', async () => {
    const shell = await shellHarness(undefined, {}, { platform: 'darwin' });
    expect(shell.window.options.titleBarStyle).toBe('hidden');
    expect(shell.window.options).not.toHaveProperty('titleBarOverlay');
    expect(shell.window.options.trafficLightPosition).toEqual({ x: 16, y: 13 });
    // A sheet, such as the dialog for a folder, comes down from under the bar that the page draws.
    expect(shell.window.setSheetOffset).toHaveBeenCalledWith(40);
    // The page draws the whole bar there: a theme changes what lies behind it, and no overlay.
    shell.invoke('set-theme', 'moonwhite');
    expect(shell.window.setBackgroundColor).toHaveBeenLastCalledWith(themeColour('moonwhite', 'ink-0'));
    expect(shell.window.setTitleBarOverlay).not.toHaveBeenCalled();
  });

  it('has the page leave room in its bar where the buttons of each system stand', () => {
    // The page names the system in the frame of the window, and says when it is in full screen.
    const bar = readFileSync(new URL('../src/ui/shell.css', import.meta.url), 'utf8');
    expect(blockOf(bar, '.window[data-native] .titlebar')).toContain('padding-right: 148px;');
    const mac = blockOf(bar, ".window[data-native='darwin'] .titlebar");
    expect(mac).toContain('padding-left: 84px;');
    expect(mac).toContain('padding-right: var(--s4);');
    expect(blockOf(bar, ".window[data-native='darwin'][data-full-screen] .titlebar")).toContain('padding-left: var(--s4);');
  });

  it('gives macOS a menu that copies and pastes, closes and leaves, and has nothing that reloads the page', async () => {
    const shell = await shellHarness(undefined, {}, { platform: 'darwin' });
    expect(shell.menu.buildFromTemplate).toHaveBeenCalledOnce();
    expect(shell.menu.buildFromTemplate.mock.calls[0][0]).toEqual([
      { role: 'appMenu' }, { role: 'fileMenu' }, { role: 'editMenu' }, { role: 'windowMenu' },
    ]);
    expect(shell.menu.setApplicationMenu).toHaveBeenCalledWith(shell.menu.buildFromTemplate.mock.results[0].value);
  });

  it('gives the Dock the dove in development, where a package has its own icon', async () => {
    const development = await shellHarness(undefined, {}, { platform: 'darwin' });
    expect(development.app.dock.setIcon).toHaveBeenCalledWith(path.resolve(path.dirname(mainPath), '../dist/columbina-icon.png'));
    const packaged = await shellHarness(undefined, {}, { platform: 'darwin', packaged: true });
    expect(packaged.app.dock.setIcon).not.toHaveBeenCalled();
  });

  it('tells the page on macOS when the window goes into full screen and out of it', async () => {
    const mac = await shellHarness(undefined, {}, { platform: 'darwin' });
    mac.window.emit('enter-full-screen');
    mac.window.emit('leave-full-screen');
    expect(mac.sent).toEqual([['columbina:full-screen', true], ['columbina:full-screen', false]]);
    const windows = await shellHarness();
    windows.window.emit('enter-full-screen');
    expect(windows.sent).toEqual([]);
  });
});

describe('the service of a package', () => {
  const windowsPython = path.join(resources, 'python', 'python.exe');
  const macPython = path.join(resources, 'python', 'bin', 'python3');
  // The PATH of an application that is opened from the Finder, and what is added to it where the login shell says nothing.
  const finder = '/usr/bin:/bin:/usr/sbin:/sbin';
  const added = `${finder}:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin`;
  // The service is told where to write what its task does, for the pet.
  const pet = { COLUMBINA_PET_STATE: path.resolve('/columbina-test/userData/pet-state.json') };
  // And the engine of the studio that the package brings, which its agent reaches the studio through.
  const windowsEngine = { COLUMBINA_ENGINE: path.join(resources, 'engine', 'columbina-engine.exe') };
  const macEngine = { COLUMBINA_ENGINE: path.join(resources, 'engine', 'columbina-engine') };

  it('runs the Python that the package brings, and adds nothing for that Python to the environment', async () => {
    const env = { PATH: 'C:\\Windows', PYTHONPATH: 'C:\\elsewhere', USERPROFILE: 'C:\\Users\\a' };
    const shell = await shellHarness(undefined, env, { packaged: true });
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).resolves.toEqual({ mode: 'service', cwd });
    const [executable, args, options] = shell.spawn.mock.calls[0];
    expect(executable).toBe(windowsPython);
    expect(args).toEqual(['-I', '-B', '-m', 'columbina', 'serve']);
    // The commands of the agent inherit this environment. What keeps that Python to itself are its flags.
    expect(options.env).toEqual({ ...env, ...pet, ...windowsEngine });
    expect(shell.logins).toEqual([]);
  });

  it('sets services up with the same Python', async () => {
    const shell = await shellHarness(undefined, {}, { packaged: true });
    await shell.invoke('services');
    expect(shell.spawn.mock.calls[0][0]).toBe(windowsPython);
    expect(shell.spawn.mock.calls[0][1]).toEqual(['-I', '-B', '-m', 'columbina', 'setup', '--list']);
  });

  it('never replays a recorded session, and says so where its Python is missing', async () => {
    const shell = await shellHarness(undefined, {}, { packaged: true });
    shell.absent.add(windowsPython);
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).rejects.toThrow('This package has no service of its own.');
    await expect(shell.invoke('services')).rejects.toThrow('This package has no service of its own.');
    expect(shell.spawn).not.toHaveBeenCalled();
  });

  it('runs the program that is named instead, where one is', async () => {
    const shell = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python }, { packaged: true });
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).resolves.toEqual({ mode: 'service', cwd });
    expect(shell.spawn.mock.calls[0][0]).toBe(python);
    expect(shell.spawn.mock.calls[0][1]).toEqual(['-m', 'columbina', 'serve']);
  });

  it('on macOS runs bin/python3, with the PATH that the login shell of the user says', async () => {
    const env = { SHELL: '/bin/bash', PATH: finder };
    const shell = await shellHarness(undefined, env, { platform: 'darwin', packaged: true });
    // The shell is asked once, as a terminal starts it, while the window opens.
    expect(shell.logins).toHaveLength(1);
    const [program, asked, options] = shell.spawn.mock.calls[0];
    expect(program).toBe('/bin/bash');
    const said = (name: string) => `printf '_COLUMBINA_${name}_%s_COLUMBINA_${name}_' "$${name}"`;
    expect(asked).toEqual(['-ilc', [
      'PATH', 'HTTPS_PROXY', 'https_proxy', 'HTTP_PROXY', 'http_proxy', 'ALL_PROXY', 'all_proxy', 'NO_PROXY', 'no_proxy',
    ].map(said).join('; ')]);
    expect(options.env).toEqual({ ...env, DISABLE_AUTO_UPDATE: 'true' });
    expect(options.stdio).toEqual(['ignore', 'pipe', 'ignore']);
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).resolves.toEqual({ mode: 'service', cwd });
    const [executable, args, served] = shell.spawn.mock.calls[1];
    expect(executable).toBe(macPython);
    expect(args).toEqual(['-I', '-B', '-m', 'columbina', 'serve']);
    expect(served.env).toEqual({ ...env, PATH: '/opt/homebrew/bin:/usr/bin:/bin', ...pet, ...macEngine });
    await shell.invoke('services');
    expect(shell.spawn.mock.calls[2][2].env.PATH).toBe('/opt/homebrew/bin:/usr/bin:/bin');
    expect(shell.logins).toHaveLength(1);
  });

  it('on macOS takes the proxy that the login shell sets, as a terminal has it', async () => {
    const written = 'Last login: Thu Oct 15 on ttys000\n_COLUMBINA_PATH_/opt/homebrew/bin:/usr/bin_COLUMBINA_PATH_'
      + '_COLUMBINA_HTTPS_PROXY__COLUMBINA_HTTPS_PROXY_'
      + '_COLUMBINA_https_proxy_http://127.0.0.1:7890_COLUMBINA_https_proxy_'
      + '_COLUMBINA_ALL_PROXY_socks5://127.0.0.1:7891_COLUMBINA_ALL_PROXY_'
      + '_COLUMBINA_NO_PROXY_ localhost,127.0.0.1 _COLUMBINA_NO_PROXY_'
      + '_COLUMBINA_no_proxy_   _COLUMBINA_no_proxy_';
    const env = { PATH: finder };
    const shell = await shellHarness(undefined, env, { platform: 'darwin', packaged: true, loginShell: written });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    // What is empty, or no more than blanks, is as good as not said.
    expect(shell.spawn.mock.calls.at(-1)![2].env).toEqual({
      PATH: '/opt/homebrew/bin:/usr/bin',
      https_proxy: 'http://127.0.0.1:7890',
      ALL_PROXY: 'socks5://127.0.0.1:7891',
      NO_PROXY: 'localhost,127.0.0.1',
      ...pet, ...macEngine,
    });
    await shell.invoke('services');
    expect(shell.spawn.mock.calls.at(-1)![2].env.https_proxy).toBe('http://127.0.0.1:7890');
  });

  it('on macOS keeps a proxy that the application was given, in whichever case it is written', async () => {
    const written = '_COLUMBINA_PATH_/usr/bin_COLUMBINA_PATH_'
      + '_COLUMBINA_HTTPS_PROXY_http://shell:1_COLUMBINA_HTTPS_PROXY__COLUMBINA_https_proxy_http://shell:2_COLUMBINA_https_proxy_'
      + '_COLUMBINA_http_proxy_http://shell:3_COLUMBINA_http_proxy_';
    const env = { PATH: finder, https_proxy: 'http://given:1', HTTP_PROXY: '' };
    const shell = await shellHarness(undefined, env, { platform: 'darwin', packaged: true, loginShell: written });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    // A variable that the application has empty is none: the shell fills it.
    expect(shell.spawn.mock.calls.at(-1)![2].env).toEqual({
      PATH: '/usr/bin', https_proxy: 'http://given:1', HTTP_PROXY: '', http_proxy: 'http://shell:3', ...pet, ...macEngine,
    });
  });

  it('on macOS takes a proxy from the login shell also where it says no PATH', async () => {
    const shell = await shellHarness(undefined, { PATH: finder }, {
      platform: 'darwin', packaged: true, loginShell: '_COLUMBINA_https_proxy_http://127.0.0.1:7890_COLUMBINA_https_proxy_',
    });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    expect(shell.spawn.mock.calls.at(-1)![2].env).toEqual({ PATH: added, https_proxy: 'http://127.0.0.1:7890', ...pet, ...macEngine });
  });

  it('on macOS asks zsh where no shell of the user is named by its path', async () => {
    for (const env of [{}, { SHELL: 'zsh' }, { SHELL: '' }] as Record<string, string>[]) {
      const shell = await shellHarness(undefined, env, { platform: 'darwin', packaged: true });
      expect(shell.spawn.mock.calls[0][0]).toBe('/bin/zsh');
    }
  });

  it.each([
    ['nothing', ''],
    ['no mark', '/opt/homebrew/bin:/usr/bin'],
    ['one mark only', '_COLUMBINA_PATH_/opt/homebrew/bin'],
    ['an empty PATH', '_COLUMBINA_PATH_ _COLUMBINA_PATH_'],
  ])('on macOS adds the usual places to the PATH where the login shell writes %s', async (_what, written) => {
    const shell = await shellHarness(undefined, { PATH: finder }, { platform: 'darwin', packaged: true, loginShell: written });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    expect(shell.spawn.mock.calls.at(-1)![2].env.PATH).toBe(added);
  });

  it('on macOS adds a usual place only where it is not in the PATH already', async () => {
    const env = { PATH: '/usr/local/bin:/usr/bin:/bin' };
    const shell = await shellHarness(undefined, env, { platform: 'darwin', packaged: true, loginShell: '' });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    expect(shell.spawn.mock.calls.at(-1)![2].env.PATH)
      .toBe('/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/sbin');
  });

  it('on macOS waits five seconds at most for a login shell that does not answer', async () => {
    vi.useFakeTimers();
    try {
      const shell = await shellHarness(undefined, { PATH: finder }, { platform: 'darwin', packaged: true, loginShell: null });
      const cwd = await shell.select('/selected-project');
      const started = shell.invoke('start', { cwd });
      const listed = shell.invoke('services');
      const signing = shell.invoke('github-sign-in');
      await vi.advanceTimersByTimeAsync(4_900);
      // Only the login shell has been started.
      expect(shell.spawn).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(200);
      await expect(started).resolves.toEqual({ mode: 'service', cwd });
      await expect(listed).resolves.toEqual({ use: null, services: [] });
      await expect(signing).resolves.toBeUndefined();
      expect(shell.logins[0].kill).toHaveBeenCalledOnce();
      const [served, setUp, signedIn] = ['serve', 'setup', 'github'].map(word => shell.spawn.mock.calls.find(([, args]) => args.includes(word))!);
      expect(served[2].env.PATH).toBe(added);
      expect(setUp[2].env.PATH).toBe(added);
      expect(signedIn[2].env.PATH).toBe(added);
      // Away from Windows, the sign-in and what it starts are a group of their own, ended as one.
      expect(signedIn[2].detached).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it('asks no login shell on Windows, nor in development', async () => {
    const windows = await shellHarness(undefined, {}, { packaged: true });
    expect(windows.logins).toEqual([]);
    const development = await shellHarness(undefined, { PATH: finder }, { platform: 'darwin' });
    expect(development.logins).toEqual([]);
    const cwd = await development.select('/selected-project');
    await expect(development.invoke('start', { cwd })).resolves.toEqual({ mode: 'mock', cwd });
  });
});

describe('what the service said last', () => {
  const started = async () => {
    const shell = await shellHarness();
    await shell.invoke('start', { cwd: await shell.select('/selected-project') });
    return shell;
  };
  const says = (shell: Awaited<ReturnType<typeof shellHarness>>, text: string, child = shell.children.at(-1)) =>
    child.stderr.emit('data', Buffer.from(text, 'utf8'));

  it('is kept line by line, however it came in pieces', async () => {
    const shell = await started();
    says(shell, 'one\ntwo\nthr');
    says(shell, 'ee\r\nfour');
    expect(shell.invoke('service-tail')).toEqual(['one', 'two', 'three', 'four']);
    // A line that is still being written is told of as far as it has come, and grows.
    says(shell, 'teen\n');
    expect(shell.invoke('service-tail')).toEqual(['one', 'two', 'three', 'fourteen']);
  });

  it('is the last thirty lines, and of a long line the beginning', async () => {
    const shell = await started();
    says(shell, Array.from({ length: 100 }, (_, index) => `line ${index + 1}`).join('\n') + '\n');
    const tail = shell.invoke('service-tail') as string[];
    expect(tail).toHaveLength(30);
    expect(tail[0]).toBe('line 71');
    expect(tail.at(-1)).toBe('line 100');
    says(shell, `${'x'.repeat(2000)}\n`);
    const long = (shell.invoke('service-tail') as string[]).at(-1)!;
    expect(long).toBe(`${'x'.repeat(400)}…`);
    // A line that is just as long as a line may be is whole, and not marked as cut.
    says(shell, `${'y'.repeat(400)}\n`);
    expect((shell.invoke('service-tail') as string[]).at(-1)).toBe('y'.repeat(400));
  });

  it('is thirty lines at most also while a line is being written', async () => {
    const shell = await started();
    says(shell, `${Array.from({ length: 40 }, (_, index) => `line ${index + 1}`).join('\n')}\nand a beginning`);
    const tail = shell.invoke('service-tail') as string[];
    expect(tail).toHaveLength(30);
    expect(tail[0]).toBe('line 12');
    expect(tail.at(-1)).toBe('and a beginning');
  });

  it('tells only the beginning of a long line that has no end yet', async () => {
    const shell = await started();
    says(shell, 'z'.repeat(3000));
    expect(shell.invoke('service-tail')).toEqual([`${'z'.repeat(400)}…`]);
    // However much of it comes, what is held of it stays small, and the line still ends where it ends.
    says(shell, 'z'.repeat(3000));
    says(shell, '\nnext\n');
    expect(shell.invoke('service-tail')).toEqual([`${'z'.repeat(400)}…`, 'next']);
  });

  it('has nothing in it that steers a terminal, and no line that is empty', async () => {
    const shell = await started();
    says(shell, '\u001b[31mred\u001b[0m and \u001b[1;4mbold\u001b[m\n\n   \nbell\u0007 and tab\there\n');
    expect(shell.invoke('service-tail')).toEqual(['red and bold', 'bell and tab here']);
  });

  it('keeps characters that take several bytes whole when they come in two pieces', async () => {
    const shell = await started();
    const bytes = Buffer.from('月光\n', 'utf8');
    shell.children[0].stderr.emit('data', bytes.subarray(0, 4));
    shell.children[0].stderr.emit('data', bytes.subarray(4));
    expect(shell.invoke('service-tail')).toEqual(['月光']);
  });

  it('begins anew with every service', async () => {
    const shell = await started();
    says(shell, 'of the first\n');
    await shell.invoke('start', { cwd: await shell.select('/other-project') });
    expect(shell.invoke('service-tail')).toEqual([]);
    // What the first one still says is not taken for the words of the second.
    says(shell, 'late words of the first\n', shell.children[0]);
    says(shell, 'of the second\n');
    expect(shell.invoke('service-tail')).toEqual(['of the second']);
  });

  it('is still there after the service has ended, which is when it is asked for', async () => {
    const shell = await started();
    says(shell, 'error: the profile names a plugin that is not there\n');
    const child = shell.children[0];
    child.exitCode = 2;
    child.emit('close', 2, null);
    expect(shell.sent.at(-1)).toEqual(['columbina:closed', 'The service process exited (2).']);
    expect(shell.invoke('service-tail')).toEqual(['error: the profile names a plugin that is not there']);
  });

  it('is nothing before any service was started', async () => {
    const shell = await shellHarness();
    expect(shell.invoke('service-tail')).toEqual([]);
  });

  it('shows its own log in the folder of the logs, whatever it is handed', async () => {
    const shell = await shellHarness();
    shell.invoke('show-log', '/etc/passwd');
    shell.invoke('show-log');
    expect(shell.system.showItemInFolder.mock.calls).toEqual([[logFile], [logFile]]);
  });

  it('answers only its own window', async () => {
    const shell = await started();
    for (const method of ['service-tail', 'show-log']) {
      expect(() => shell.handlers.get(`columbina:${method}`)!(shell.unrelated)).toThrow('IPC sender');
    }
    expect(shell.system.showItemInFolder).not.toHaveBeenCalled();
  });
});

describe('telling the user while the window is in the background', () => {
  const inBackground = async () => {
    const shell = await shellHarness();
    shell.window.focused = false;
    return shell;
  };

  it('says nothing to someone who is looking at the window', async () => {
    const shell = await shellHarness();
    expect(shell.invoke('notify', { title: 'Columbina needs your consent', body: 'git push' })).toBe(false);
    expect(shell.notifications).toEqual([]);
    expect(shell.window.flashFrame).not.toHaveBeenCalled();
  });

  it('flashes the window in the taskbar and shows a notification', async () => {
    const shell = await inBackground();
    expect(shell.invoke('notify', { title: 'Columbina needs your consent', body: 'git push' })).toBe(true);
    expect(shell.window.flashFrame).toHaveBeenCalledWith(true);
    expect(shell.notifications).toHaveLength(1);
    expect(shell.notifications[0].options).toEqual({ title: 'Columbina needs your consent', body: 'git push' });
    expect(shell.notifications[0].show).toHaveBeenCalledOnce();
  });

  it('stops flashing once the window is looked at', async () => {
    const shell = await inBackground();
    shell.invoke('notify', { title: 'Done', body: '' });
    shell.invoke('notify', { title: 'Done again', body: '' });
    shell.window.emit('focus');
    expect(shell.window.flashFrame.mock.calls).toEqual([[true], [true], [false]]);
    // A later look at the window, with nothing new to tell, changes nothing.
    shell.window.emit('focus');
    expect(shell.window.flashFrame).toHaveBeenCalledTimes(3);
  });

  it('flashes anew for what comes after the window was looked at, and stops again', async () => {
    const shell = await inBackground();
    shell.invoke('notify', { title: 'First', body: '' });
    shell.window.emit('focus');
    shell.invoke('notify', { title: 'Second', body: '' });
    shell.window.emit('focus');
    expect(shell.window.flashFrame.mock.calls).toEqual([[true], [false], [true], [false]]);
  });

  it('brings the window forward when the notification is clicked', async () => {
    const shell = await inBackground();
    shell.invoke('notify', { title: 'Done', body: '' });
    shell.notifications[0].emit('click');
    expect(shell.window.restore).not.toHaveBeenCalled();
    expect(shell.window.show).toHaveBeenCalledOnce();
    expect(shell.window.focus).toHaveBeenCalledOnce();
    shell.window.minimized = true;
    shell.notifications[0].emit('click');
    expect(shell.window.restore).toHaveBeenCalledOnce();
  });

  it('makes one short line of each thing that it is given', async () => {
    const shell = await inBackground();
    shell.invoke('notify', { title: `  A\ttitle\nover\u0000two lines  ${'t'.repeat(200)}`, body: `rm -rf\r\n${'b'.repeat(500)}` });
    const { title, body } = shell.notifications[0].options;
    expect(title).toBe(`A title over two lines ${'t'.repeat(57)}…`);
    expect(title).toHaveLength(81);
    // A title that is just as long as a title may be is whole, and so is such a body.
    shell.invoke('notify', { title: 'T'.repeat(80), body: 'B'.repeat(240) });
    expect(shell.notifications[1].options).toEqual({ title: 'T'.repeat(80), body: 'B'.repeat(240) });
    expect(body).toBe(`rm -rf ${'b'.repeat(233)}…`);
    expect(body).toHaveLength(241);
  });

  it('takes a body that is no text for none', async () => {
    const shell = await inBackground();
    for (const body of [undefined, null, 7, { toString: () => 'x' }, ['a']]) shell.invoke('notify', { title: 'Done', body });
    expect(shell.notifications.map(notification => notification.options.body)).toEqual(['', '', '', '', '']);
  });

  it('needs a title', async () => {
    const shell = await inBackground();
    for (const message of [undefined, null, {}, { title: '' }, { title: ' \n ' }, { title: 7 }, 'Done']) {
      expect(() => shell.invoke('notify', message)).toThrow('A notification needs a title.');
    }
    expect(shell.notifications).toEqual([]);
    expect(shell.window.flashFrame).not.toHaveBeenCalled();
  });

  it('still flashes on a system that shows no notifications', async () => {
    const shell = await inBackground();
    shell.Notification.supported = false;
    expect(shell.invoke('notify', { title: 'Done', body: '' })).toBe(true);
    expect(shell.window.flashFrame).toHaveBeenCalledWith(true);
    expect(shell.notifications).toEqual([]);
  });

  it('tells for its own window only', async () => {
    const shell = await inBackground();
    expect(() => shell.handlers.get('columbina:notify')!(shell.unrelated, { title: 'Done' })).toThrow('IPC sender');
    expect(shell.notifications).toEqual([]);
  });
});

describe('what the shell keeps from one start to the next', () => {
  const light = { color: themeColour('moonwhite', 'ink-0'), symbolColor: themeColour('moonwhite', 'moon-0'), height: 40 };
  const dark = { color: themeColour('moonveil', 'ink-0'), symbolColor: themeColour('moonveil', 'moon-0'), height: 40 };
  const folder = (name: string) => path.resolve(`/projects/${name}`);

  it('opens in the theme that was chosen last', async () => {
    const shell = await shellHarness(JSON.stringify({ theme: 'moonwhite' }));
    expect(shell.window.options.backgroundColor).toBe(light.color);
    expect(shell.window.options.titleBarOverlay).toEqual(light);
  });

  it.each([
    ['there is no such file', undefined],
    ['the file is no JSON', '{"theme": "moonwh'],
    ['the file is empty', ''],
    ['the file holds no object', '["moonwhite"]'],
    ['the file holds null', 'null'],
    ['the theme is one that is gone', JSON.stringify({ theme: 'frostlight' })],
    ['the theme is a name that every object has', JSON.stringify({ theme: 'constructor' })],
    ['the theme is no text', JSON.stringify({ theme: { color: '#ffffff' } })],
  ])('opens in the dark theme where %s', async (_what, kept) => {
    const shell = await shellHarness(kept);
    expect(shell.window.options.backgroundColor).toBe(dark.color);
    expect(shell.window.options.titleBarOverlay).toEqual(dark);
  });

  it('keeps the theme when it is set, and nothing of a theme that it refuses', async () => {
    const shell = await shellHarness();
    shell.invoke('set-theme', 'moonwhite');
    expect(shell.preferences()).toEqual({
      theme: 'moonwhite', recent: [], githubSkipped: false, feishuOn: false, feishuLanguage: null,
      wecomOn: false, wecomLanguage: null, qqOn: false, qqLanguage: null, language: null,
      petOn: false, pet: 'builtin/moon', petAt: null,
    });
    expect(() => shell.invoke('set-theme', 'frostlight')).toThrow('Unknown application theme.');
    expect(shell.preferences().theme).toBe('moonwhite');
  });

  it('does not write the theme again while it stays the same', async () => {
    const shell = await shellHarness(JSON.stringify({ theme: 'moonwhite' }));
    shell.invoke('set-theme', 'moonwhite');
    expect(shell.writeFileSync).not.toHaveBeenCalled();
    shell.invoke('set-theme', 'moonveil');
    expect(shell.writeFileSync).toHaveBeenCalledOnce();
  });

  it('writes the file whole or not at all', async () => {
    const shell = await shellHarness();
    shell.invoke('set-theme', 'moonwhite');
    // It is written beside its place and then moved there, so that a start never finds half of it.
    expect(shell.writeFileSync.mock.calls[0][0]).toBe(`${preferencesFile}.tmp`);
    expect([...shell.files.keys()]).toEqual([preferencesFile]);
  });

  it('goes on where nothing can be written', async () => {
    const shell = await shellHarness();
    shell.writeFileSync.mockImplementation(() => { throw new Error('the disk is full'); });
    expect(() => shell.invoke('set-theme', 'moonwhite')).not.toThrow();
    expect(shell.window.setBackgroundColor).toHaveBeenLastCalledWith(light.color);
    await expect(shell.select(folder('a'))).resolves.toBe(folder('a'));
  });

  it('keeps the folders that were chosen: the last one first, each once, eight at most', async () => {
    const shell = await shellHarness();
    await shell.select(folder('a'));
    await shell.select(folder('b'));
    await shell.select(folder('a'));
    expect(shell.preferences().recent).toEqual([folder('a'), folder('b')]);
    for (const name of ['c', 'd', 'e', 'f', 'g', 'h', 'i']) await shell.select(folder(name));
    expect(shell.preferences().recent).toEqual(['i', 'h', 'g', 'f', 'e', 'd', 'c', 'a'].map(folder));
  });

  it('takes two spellings of a folder for one where the system does', async () => {
    const shell = await shellHarness();
    await shell.select(folder('Work'));
    await shell.select(folder('WORK'));
    // On Windows the capitals of a path do not tell two folders apart. The last spelling is kept.
    expect(shell.preferences().recent).toEqual([folder('WORK')]);
  });

  it('keeps nothing where no folder was chosen', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('select-folder')).resolves.toBeNull();
    expect(shell.preferences()).toBeNull();
  });

  it('tells of the kept folders that are still there, with their names', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a'), folder('gone'), folder('b')] }));
    shell.gone.add(folder('gone'));
    expect(shell.invoke('recent-projects')).toEqual([
      { path: folder('a'), name: 'a' },
      { path: folder('b'), name: 'b' },
    ]);
  });

  it('takes of what was kept only absolute paths that are text, each once', async () => {
    const shell = await shellHarness(JSON.stringify({
      recent: [3, null, 'relative/folder', '', folder('a'), { path: folder('x') }, folder('a'), [folder('y')], folder('b')],
    }));
    expect(shell.invoke('recent-projects')).toEqual([
      { path: folder('a'), name: 'a' },
      { path: folder('b'), name: 'b' },
    ]);
  });

  it('takes what was kept for no folders where it is no list', async () => {
    for (const recent of ['"/projects/a"', '{"0": "/projects/a"}', 'null', '7']) {
      const shell = await shellHarness(`{"recent": ${recent}}`);
      expect(shell.invoke('recent-projects')).toEqual([]);
    }
  });

  it('takes no more than eight of what was kept', async () => {
    const many = Array.from({ length: 20 }, (_, index) => folder(`p${index}`));
    const shell = await shellHarness(JSON.stringify({ recent: many }));
    expect(shell.invoke('recent-projects')).toHaveLength(8);
  });

  it('starts a kept folder without the dialog, and puts it first', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a'), folder('b')] }));
    expect(shell.invoke('open-recent', folder('b'))).toBe(folder('b'));
    expect(shell.dialog.showOpenDialog).not.toHaveBeenCalled();
    await expect(shell.invoke('start', { cwd: folder('b') })).resolves.toEqual({ mode: 'mock', cwd: folder('b') });
    expect(shell.preferences().recent).toEqual([folder('b'), folder('a')]);
  });

  it('refuses a folder that it did not keep', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a')] }));
    for (const asked of [folder('elsewhere'), path.resolve('/projects'), `${folder('a')}${path.sep}inner`, '', undefined, null, 7, ['/projects/a']]) {
      expect(() => shell.invoke('open-recent', asked)).toThrow('That folder is not among the recent projects.');
    }
    await expect(shell.invoke('start', { cwd: folder('elsewhere') })).rejects.toThrow('native folder dialog');
    expect(shell.spawn).not.toHaveBeenCalled();
  });

  it('refuses a kept folder that leads somewhere else now', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a')] }));
    shell.realpathSync.mockImplementation((value: string) => (path.resolve(value) === folder('a') ? folder('moved') : path.resolve(value)));
    expect(() => shell.invoke('open-recent', folder('a'))).toThrow('That folder leads somewhere else than when it was chosen.');
    await expect(shell.invoke('start', { cwd: folder('a') })).rejects.toThrow('native folder dialog');
  });

  it('refuses a kept folder that is gone', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a')] }));
    shell.gone.add(folder('a'));
    expect(() => shell.invoke('open-recent', folder('a'))).toThrow();
    await expect(shell.invoke('start', { cwd: folder('a') })).rejects.toThrow('native folder dialog');
  });

  it('keeps and tells for its own window only', async () => {
    const shell = await shellHarness(JSON.stringify({ recent: [folder('a')] }));
    for (const method of ['recent-projects', 'open-recent']) {
      expect(() => shell.handlers.get(`columbina:${method}`)!(shell.unrelated, folder('a'))).toThrow('IPC sender');
    }
    await expect(shell.invoke('start', { cwd: folder('a') })).rejects.toThrow('native folder dialog');
  });
});

describe('asking the program of the service what is set up', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const list = { use: 'openai', services: [{ name: 'openai', title: 'OpenAI', base_url: 'https://api.openai.com/v1', own: false, model: 'gpt-test', context_window: 400000, key_env: 'OPENAI_API_KEY', key: 'stored' }] };

  it('cannot be done while the shell replays a recorded session', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('services')).resolves.toBeNull();
    await expect(shell.invoke('set-up-service', {})).rejects.toThrow('No service program is set up');
    expect(shell.spawn).not.toHaveBeenCalled();
    expect(shell.windows).toHaveLength(1);
  });

  it('starts the program of the service with the words for it, and gives back its document', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: `${JSON.stringify(list)}\n`, code: 0 });
    await expect(shell.invoke('services')).resolves.toEqual(list);
    expect(shell.spawn).toHaveBeenCalledOnce();
    expect(shell.spawn.mock.calls[0][0]).toBe(python);
    expect(shell.spawn.mock.calls[0][1]).toEqual(['-m', 'columbina', 'setup', '--list']);
    // Nothing is written to it, and its input is closed, so that it waits for nothing.
    expect(shell.setups[0].input).toBe('');
    // It runs in the home of the user and in no project.
    expect(shell.setups[0].cwd).toBe(path.resolve('/columbina-test/home'));
  });

  it('makes the command from the command of the service, without what was meant for serving', async () => {
    const shell = await shellHarness(undefined, {
      ...real, COLUMBINA_SERVICE_ARGS: JSON.stringify(['-X', 'utf8', '-m', 'columbina', 'serve', '--profile', 'F:\\x\\serve.yml']),
    });
    await shell.invoke('services');
    expect(shell.spawn.mock.calls[0][1]).toEqual(['-X', 'utf8', '-m', 'columbina', 'setup', '--list']);
  });

  it('cannot be done where the command of the service does not serve', async () => {
    const shell = await shellHarness(undefined, { ...real, COLUMBINA_SERVICE_ARGS: JSON.stringify(['run-it.py']) });
    await expect(shell.invoke('services')).resolves.toBeNull();
    await expect(shell.invoke('set-up-service', {})).rejects.toThrow('No service program is set up');
    expect(shell.spawn).not.toHaveBeenCalled();
  });

  it('gives what the program says is wrong as it says it', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: '{"error": "the file provider.yml is not YAML"}\n', code: 2 });
    await expect(shell.invoke('services')).rejects.toThrow('the file provider.yml is not YAML');
  });

  it.each([
    ['nothing', ''],
    ['a trace and no document', 'Traceback (most recent call last):\n  File "x.py", line 1\n'],
    ['a document that is cut off', '{"use": "openai", "services": [{"name"'],
    ['a list', '["openai"]\n'],
    ['a text', '"openai"\n'],
    ['null', 'null\n'],
  ])('says that the program did not answer where it wrote %s', async (_what, out) => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out, code: 1 });
    await expect(shell.invoke('services')).rejects.toThrow('did not answer with a document');
  });

  it('reads the document from the last line that the program wrote', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: `a line that a plugin printed\r\n${JSON.stringify(list)}\r\n\r\n`, code: 0 });
    await expect(shell.invoke('services')).resolves.toEqual(list);
  });

  it('puts a document together that came in pieces, also in the middle of a character', async () => {
    const shell = await shellHarness(undefined, real);
    const bytes = Buffer.from(`${JSON.stringify({ use: null, services: [{ name: 'mine', title: '我的服务' }] })}\n`, 'utf8');
    const cut = bytes.indexOf(Buffer.from('的', 'utf8')) + 1;
    shell.setup.answer = () => null;
    const asked = shell.invoke('services');
    await Promise.resolve();
    const child = shell.setups[0].child;
    child.stdout.emit('data', bytes.subarray(0, cut));
    child.stdout.emit('data', bytes.subarray(cut));
    child.emit('close', 0, null);
    await expect(asked).resolves.toEqual({ use: null, services: [{ name: 'mine', title: '我的服务' }] });
  });

  it('does not read without end what a program writes', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: `${JSON.stringify({ use: null, services: [], padding: 'x'.repeat(600_000) })}\n`, code: 0 });
    await expect(shell.invoke('services')).rejects.toThrow('wrote more than is read');
  });

  it('says so where the program cannot be started', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => null;
    const asked = shell.invoke('services');
    await Promise.resolve();
    shell.setups[0].child.emit('error', new Error('spawn ENOENT'));
    await expect(asked).rejects.toThrow('could not be started: spawn ENOENT');
  });

  it('does not wait for ever for a program that does not end', async () => {
    vi.useFakeTimers();
    try {
      const shell = await shellHarness(undefined, real);
      shell.setup.answer = () => null;
      const asked = shell.invoke('services');
      const failed = expect(asked).rejects.toThrow('did not end in time');
      await vi.advanceTimersByTimeAsync(44_000);
      expect(shell.spawn).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(2_000);
      await failed;
      // It is ended, with whatever it started.
      expect(shell.spawn.mock.calls.at(-1)![0]).toBe('taskkill.exe');
      expect(shell.spawn.mock.calls.at(-1)![1]).toEqual(['/pid', '500', '/T', '/F']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('answers only its own window', async () => {
    const shell = await shellHarness(undefined, real);
    for (const method of ['services', 'set-up-service']) {
      await expect((async () => shell.handlers.get(`columbina:${method}`)!(shell.unrelated, {}))()).rejects.toThrow('IPC sender');
    }
    expect(shell.spawn).not.toHaveBeenCalled();
    expect(shell.windows).toHaveLength(1);
  });
});

describe('the window for the key', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const list = { use: 'deepseek', services: [{ name: 'deepseek', title: 'DeepSeek', key: 'none' }] };
  const stored = { stored: { name: 'deepseek', base_url: 'https://api.deepseek.com', model: 'deepseek-flash', context_window: 1048576 }, key_in: 'keyring' };
  const KEY = 'sk-made-up-for-a-test-0123456789';
  const opened = async (asked: unknown = {}, kept?: string) => {
    const shell = await shellHarness(kept, real);
    shell.setup.answer = (args) => (args.includes('--list')
      ? { out: `${JSON.stringify(list)}\n`, code: 0 } : { out: `${JSON.stringify(stored)}\n`, code: 0 });
    const closed = shell.invoke('set-up-service', asked) as Promise<unknown>;
    await Promise.resolve();
    return { shell, closed };
  };

  it('is a window of its own under the main one, as closed to the world as the main one', async () => {
    const { shell } = await opened();
    expect(shell.windows).toHaveLength(2);
    const options = shell.keyWindow().options;
    expect(options.parent).toBe(shell.window);
    expect(options.modal).toBe(true);
    expect(options.webPreferences).toEqual({
      preload: path.join(path.dirname(mainPath), 'key-preload.cjs'),
      contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true,
    });
    expect(shell.keyWindow().loadFile).toHaveBeenCalledWith(path.resolve(path.dirname(mainPath), '../dist/key.html'));
    expect(shell.keyWindow().webContents.mainFrame.url).toBe(keyPage);
    // It opens no window and goes nowhere.
    expect(shell.keyWindow().webContents.setWindowOpenHandler).toHaveBeenCalledOnce();
    expect(shell.keyWindow().webContents.setWindowOpenHandler.mock.calls[0][0]()).toEqual({ action: 'deny' });
    for (const happening of ['will-navigate', 'will-attach-webview']) {
      const event = { preventDefault: vi.fn() };
      shell.keyWindow().webContents.emit(happening, event);
      expect(event.preventDefault).toHaveBeenCalledOnce();
    }
  });

  it('opens in the theme of the main window', async () => {
    const { shell } = await opened({}, JSON.stringify({ theme: 'moonwhite' }));
    expect(shell.keyWindow().options.backgroundColor).toBe(themeColour('moonwhite', 'ink-0'));
  });

  it('is told what is set up, and what it was opened for', async () => {
    const { shell } = await opened({ service: 'deepseek', language: 'zh-CN' }, JSON.stringify({ theme: 'moonwhite' }));
    await expect(shell.asKeyWindow('context')).resolves.toEqual({
      service: 'deepseek', language: 'zh-CN', theme: 'moonwhite', list,
    });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'setup', '--list']);
  });

  it('is told of every language that the main window can show', async () => {
    // The shell has a list of its own. A language that is missing there opens this window in English.
    const shown = languages.map(offered => languageFor(offered.id));
    expect(shown).toEqual(['zh-CN', 'en', 'ja']);
    for (const language of shown) {
      const { shell } = await opened({ service: null, language });
      await expect(shell.asKeyWindow('context')).resolves.toMatchObject({ language });
    }
  });

  it('is told of no service and of English where what it was opened for is not what it may be', async () => {
    for (const asked of [undefined, null, 'deepseek', {}, { service: 7, language: 'de' }, { service: ['deepseek'], language: { toString: () => 'zh-CN' } }, { service: '', language: 'constructor' }]) {
      const { shell } = await opened(asked);
      await expect(shell.asKeyWindow('context')).resolves.toMatchObject({ service: null, language: 'en' });
    }
  });

  it('hands what it is given to the program on its standard input, and nowhere else', async () => {
    const { shell } = await opened();
    const document = { service: 'deepseek', model: 'deepseek-flash', context_window: 1048576, api_key: KEY };
    await expect(shell.asKeyWindow('store', document)).resolves.toEqual(stored);
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'setup', '--from-stdin']);
    expect(JSON.parse(run.input!)).toEqual(document);
    // Not on the command line, not in the environment, and not in a file.
    expect(JSON.stringify(shell.spawn.mock.calls.map(call => [call[0], call[1], call[2].env]))).not.toContain(KEY);
    expect([...shell.files.values()].join('')).not.toContain(KEY);
  });

  it('hands on only the five things that the program reads, each of its kind', async () => {
    const { shell } = await opened();
    await shell.asKeyWindow('store', {
      service: 'mine', base_url: 'http://10.0.0.5:8000/v1', model: 'tiny', context_window: 32768, api_key: KEY,
      extra: 'x', __proto__: { polluted: true }, toString: 'x',
    });
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({
      service: 'mine', base_url: 'http://10.0.0.5:8000/v1', model: 'tiny', context_window: 32768, api_key: KEY,
    });
    await shell.asKeyWindow('store', { service: 7, model: ['tiny'], base_url: null, context_window: '32768', api_key: { key: KEY } });
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({});
    await shell.asKeyWindow('store', { service: 'mine', model: 'tiny', context_window: 1.5 });
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ service: 'mine', model: 'tiny' });
    for (const given of [undefined, null, 'x', 7, []]) {
      await shell.asKeyWindow('store', given);
      expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({});
    }
  });

  it('gives back what the program says is wrong, for the window to show', async () => {
    const { shell } = await opened();
    shell.setup.answer = () => ({ out: '{"error": "the document names no model"}\n', code: 2 });
    await expect(shell.asKeyWindow('store', { service: 'deepseek' })).resolves.toEqual({ error: 'the document names no model' });
  });

  it('takes the key from that window and from no other', async () => {
    const { shell } = await opened();
    const before = shell.setups.length;
    for (const method of ['context', 'store', 'check', 'forget', 'open-page', 'close']) {
      // The main window shows what a model wrote. It has no way to this.
      await expect((async () => shell.handlers.get(`columbina-key:${method}`)!(shell.event, { api_key: KEY }))()).rejects.toThrow('IPC sender');
      await expect((async () => shell.handlers.get(`columbina-key:${method}`)!(shell.unrelated, { api_key: KEY }))()).rejects.toThrow('IPC sender');
      // Nor a frame inside the window for the key, or a page that it does not start with.
      const frame = { sender: shell.keyWindow().webContents, senderFrame: { url: keyPage } };
      await expect((async () => shell.handlers.get(`columbina-key:${method}`)!(frame, { api_key: KEY }))()).rejects.toThrow('IPC sender');
    }
    shell.keyWindow().webContents.mainFrame.url = pathToFileURL(path.resolve(path.dirname(mainPath), '../dist/index.html')).href;
    await expect((async () => shell.asKeyWindow('store', { api_key: KEY }))()).rejects.toThrow('IPC sender');
    expect(shell.setups).toHaveLength(before);
  });

  it('answers nothing before it is open, and nothing after it is closed', async () => {
    const shell = await shellHarness(undefined, real);
    const nobody = { sender: {}, senderFrame: { url: keyPage } };
    await expect((async () => shell.handlers.get('columbina-key:store')!(nobody, {}))()).rejects.toThrow('IPC sender');
    const closed = shell.invoke('set-up-service', {});
    await Promise.resolve();
    const was = { sender: shell.keyWindow().webContents, senderFrame: shell.keyWindow().webContents.mainFrame };
    shell.keyWindow().close();
    await closed;
    await expect((async () => shell.handlers.get('columbina-key:store')!(was, {}))()).rejects.toThrow('IPC sender');
  });

  it('tells the main window only whether something was stored, when it is closed', async () => {
    const { shell, closed } = await opened();
    await shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash', api_key: KEY });
    await shell.asKeyWindow('close');
    await expect(closed).resolves.toEqual({ changed: true });
    expect(JSON.stringify(shell.sent)).not.toContain(KEY);
  });

  it('says that nothing was stored where nothing was, or where the program refused', async () => {
    const first = await opened();
    first.shell.keyWindow().close();
    await expect(first.closed).resolves.toEqual({ changed: false });

    const second = await opened();
    second.shell.setup.answer = () => ({ out: '{"error": "what the document says under \'api_key\' cannot be a key"}\n', code: 2 });
    await second.shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash', api_key: 'a b' });
    second.shell.keyWindow().close();
    await expect(second.closed).resolves.toEqual({ changed: false });
  });

  it('is one window, however often it is asked for, and everyone who asked is told how it ended', async () => {
    const { shell, closed } = await opened();
    const again = shell.invoke('set-up-service', {}) as Promise<unknown>;
    await Promise.resolve();
    expect(shell.windows).toHaveLength(2);
    expect(shell.keyWindow().focus).toHaveBeenCalledOnce();
    await shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash' });
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ changed: true });
    await expect(again).resolves.toEqual({ changed: true });
  });

  it('can be opened again after it was closed, and begins anew', async () => {
    const { shell, closed } = await opened();
    await shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash' });
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ changed: true });
    const second = shell.invoke('set-up-service', {}) as Promise<unknown>;
    await Promise.resolve();
    expect(shell.windows).toHaveLength(3);
    shell.windows[2].close();
    await expect(second).resolves.toEqual({ changed: false });
  });

  // What `setup --list` says of two services: one with a page for its key, one whose page is no https page.
  const linked = {
    use: 'deepseek',
    services: [
      { name: 'deepseek', title: 'DeepSeek', key: 'stored', key_page: 'https://platform.deepseek.com/api_keys' },
      { name: 'odd', title: 'Odd', key: 'none', key_page: 'http://odd.example/keys' },
      { name: 'ollama', title: 'Ollama', key: 'none', key_page: null },
    ],
  };
  const told = async () => {
    const { shell, closed } = await opened();
    shell.setup.answer = (args) => (args.includes('--list') ? { out: `${JSON.stringify(linked)}\n`, code: 0 } : null);
    await shell.asKeyWindow('context');
    return { shell, closed };
  };

  it('checks a service with the key on the standard input of the program, and hands on only what a check reads', async () => {
    const { shell } = await opened();
    shell.setup.answer = () => ({ out: '{"models": ["deepseek-flash", "deepseek-pro"]}\n', code: 0 });
    const given = { service: 'deepseek', base_url: 'https://api.deepseek.com', api_key: KEY, model: 'm', context_window: 5, extra: 'x' };
    await expect(shell.asKeyWindow('check', given)).resolves.toEqual({ models: ['deepseek-flash', 'deepseek-pro'] });
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'setup', '--check']);
    expect(JSON.parse(run.input!)).toEqual({ service: 'deepseek', base_url: 'https://api.deepseek.com', api_key: KEY });
    expect(JSON.stringify(shell.spawn.mock.calls.map(call => [call[0], call[1], call[2].env]))).not.toContain(KEY);
    for (const odd of [undefined, null, 'x', { service: 7, api_key: { key: KEY } }]) {
      await shell.asKeyWindow('check', odd);
      expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({});
    }
  });

  it('gives back what kind of failure a check was, for the window to say', async () => {
    const { shell } = await opened();
    shell.setup.answer = () => ({ out: '{"error": "the service does not take the key", "kind": "key"}\n', code: 2 });
    await expect(shell.asKeyWindow('check', { service: 'deepseek', api_key: KEY }))
      .resolves.toEqual({ error: 'the service does not take the key', kind: 'key' });
  });

  it('deletes what is stored for a service that it was told of, and tells the main window that something changed', async () => {
    const { shell, closed } = await told();
    const forgotten = { forgotten: { name: 'deepseek', key: true, chosen: false } };
    shell.setup.answer = () => ({ out: `${JSON.stringify(forgotten)}\n`, code: 0 });
    await expect(shell.asKeyWindow('forget', 'deepseek')).resolves.toEqual(forgotten);
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'setup', '--forget', 'deepseek', '--json']);
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ changed: true });
  });

  it('stores and deletes one thing at a time, in the order in which it was asked', async () => {
    const { shell } = await told();
    const before = shell.setups.length;
    const asked = () => shell.setups.slice(before).map(run => run.args.slice(3));
    const storing = shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash', api_key: KEY });
    const forgetting = shell.asKeyWindow('forget', 'deepseek');
    await settled();
    expect(asked()).toEqual([['--from-stdin']]);
    const stored = { stored: { name: 'deepseek', model: 'deepseek-flash' }, key_in: 'keyring' };
    answer(shell.setups.at(-1)!, stored);
    await expect(storing).resolves.toEqual(stored);
    await settled();
    expect(asked()).toEqual([['--from-stdin'], ['--forget', 'deepseek', '--json']]);
    const forgotten = { forgotten: { name: 'deepseek', key: true, chosen: false } };
    answer(shell.setups.at(-1)!, forgotten);
    await expect(forgetting).resolves.toEqual(forgotten);
  });

  it('deletes nothing for a service that it was not told of, or before it was told of any', async () => {
    const first = await opened();
    await expect((async () => first.shell.asKeyWindow('forget', 'deepseek'))()).rejects.toThrow('not among');
    const { shell, closed } = await told();
    const before = shell.setups.length;
    for (const name of ['nowhere', '--list', '', 7, null, ['deepseek']]) {
      await expect((async () => shell.asKeyWindow('forget', name))()).rejects.toThrow('not among');
    }
    expect(shell.setups).toHaveLength(before);
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ changed: false });
    // A window that is opened anew has been told of nothing yet.
    void shell.invoke('set-up-service', {});
    await Promise.resolve();
    const again = { sender: shell.windows[2].webContents, senderFrame: shell.windows[2].webContents.mainFrame };
    await expect((async () => shell.handlers.get('columbina-key:forget')!(again, 'deepseek'))()).rejects.toThrow('not among');
  });

  it('changes nothing where the program refuses to delete', async () => {
    const { shell, closed } = await told();
    shell.setup.answer = () => ({ out: '{"error": "the keyring did not answer"}\n', code: 2 });
    await expect(shell.asKeyWindow('forget', 'deepseek')).resolves.toEqual({ error: 'the keyring did not answer' });
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ changed: false });
  });

  it('opens the page on which a key is made where a service links to it over https, and no other page', async () => {
    const { shell } = await told();
    await shell.asKeyWindow('open-page', 'https://platform.deepseek.com/api_keys');
    expect(shell.system.openExternal).toHaveBeenCalledExactlyOnceWith('https://platform.deepseek.com/api_keys');
    await expect((async () => shell.asKeyWindow('open-page', 'http://odd.example/keys'))()).rejects.toThrow('https');
    for (const address of ['https://evil.example/', 'file:///C:/Windows/System32/calc.exe', null, 7]) {
      await expect((async () => shell.asKeyWindow('open-page', address))()).rejects.toThrow('none that a service links to');
    }
    expect(shell.system.openExternal).toHaveBeenCalledOnce();
  });
});

describe('the models of a service, for the main window', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const list = { use: 'deepseek', services: [{ name: 'deepseek' }, { name: 'ollama' }, { name: 7 }, null] };
  /** A shell that has told the main window of the services, as the window asks for them when it opens. */
  const told = async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = (args: string[]) => (args.includes('--list') ? { out: `${JSON.stringify(list)}\n`, code: 0 } : null);
    await shell.invoke('services');
    return shell;
  };

  it('lists the models of a service that the window was told of, asked with the key that a session would send', async () => {
    const shell = await told();
    shell.setup.answer = () => ({ out: '{"models": ["deepseek-flash", "deepseek-pro"]}\n', code: 0 });
    await expect(shell.invoke('models', 'deepseek')).resolves.toEqual({ models: ['deepseek-flash', 'deepseek-pro'] });
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'setup', '--check']);
    // No key goes this way: the document names the service alone.
    expect(JSON.parse(run.input!)).toEqual({ service: 'deepseek' });
    // What the program says of a failure is answered as it is.
    shell.setup.answer = () => ({ out: '{"error": "The key was refused.", "kind": "key"}\n', code: 0 });
    await expect(shell.invoke('models', 'ollama')).resolves.toEqual({ error: 'The key was refused.', kind: 'key' });
  });

  it('asks the program for the balance of a service that the window was told of, and of no other', async () => {
    const shell = await told();
    const said = { service: 'deepseek', title: 'DeepSeek', supported: true, available: true, balances: [] };
    shell.setup.answer = () => ({ out: `${JSON.stringify(said)}\n`, code: 0 });
    await expect(shell.invoke('balance', 'deepseek')).resolves.toEqual(said);
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'balance', '--json', '--service', 'deepseek']);
    // No key goes this way, and nothing else either.
    expect(run.input).toBe('');
    const runs = shell.setups.length;
    for (const name of ['openai', 7, undefined, ['deepseek']]) {
      await expect((async () => shell.invoke('balance', name))(), String(name)).rejects.toThrow('not among those that are set up');
    }
    expect(shell.setups).toHaveLength(runs);
    await expect((async () => shell.handlers.get('columbina:balance')!(shell.unrelated, 'deepseek'))()).rejects.toThrow('IPC sender');
  });

  it('has the program keep the language of reasoning, and says why where it could not', async () => {
    const shell = await told();
    shell.setup.answer = () => ({ out: '{"language": "zh"}\n', code: 0 });
    await expect(shell.invoke('reasoning-language', 'zh')).resolves.toEqual({ language: 'zh' });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'reasoning-language', 'zh', '--json']);
    shell.setup.answer = () => ({ out: '{"error": "the language of reasoning cannot be kept in x"}\n', code: 2 });
    await expect(shell.invoke('reasoning-language', 'none')).rejects.toThrow('cannot be kept in x');
    const runs = shell.setups.length;
    for (const language of ['fr', 7, undefined, ['zh']]) {
      await expect((async () => shell.invoke('reasoning-language', language))(), String(language))
        .rejects.toThrow('Unknown language of reasoning.');
    }
    expect(shell.setups).toHaveLength(runs);
    await expect((async () => shell.handlers.get('columbina:reasoning-language')!(shell.unrelated, 'zh'))())
      .rejects.toThrow('IPC sender');
  });

  it('lists nothing for a service that the window was not told of', async () => {
    const before = await shellHarness(undefined, real);
    await expect((async () => before.invoke('models', 'deepseek'))()).rejects.toThrow('not among those that are set up');
    const shell = await told();
    const runs = shell.setups.length;
    for (const name of ['openai', 7, undefined, '7', ['deepseek']]) {
      await expect((async () => shell.invoke('models', name))(), String(name)).rejects.toThrow('not among those that are set up');
    }
    expect(shell.setups).toHaveLength(runs);
  });

  it('keeps a model for a service that the window was told of, and with it nothing else and no key', async () => {
    const shell = await told();
    const stored = { stored: { name: 'deepseek', model: 'deepseek-pro' }, key_in: null };
    shell.setup.answer = () => ({ out: `${JSON.stringify(stored)}\n`, code: 0 });
    const asked = { service: 'deepseek', model: 'deepseek-pro', api_key: 'sk-never', base_url: 'https://elsewhere' };
    await expect(shell.invoke('choose-model', asked)).resolves.toEqual(stored);
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'setup', '--from-stdin']);
    expect(JSON.parse(run.input!)).toEqual({ service: 'deepseek', model: 'deepseek-pro' });
    // A name of a model as services write them.
    for (const model of ['anthropic/claude-sonnet-4:beta', 'qwen2.5:7b', `model-${'x'.repeat(194)}`]) {
      await shell.invoke('choose-model', { service: 'ollama', model });
      expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ service: 'ollama', model });
    }
  });

  it('keeps no model for a service that the window was not told of, and nothing that is no name of a model', async () => {
    const shell = await told();
    const runs = shell.setups.length;
    await expect((async () => shell.invoke('choose-model', { service: 'openai', model: 'gpt' }))()).rejects.toThrow('not among those that are set up');
    for (const model of [
      '', ' ', 'two words', 'tab\there', 'line\nbreak', 'bell\u0007', 7, undefined,
      // One character too long, and like no key: the length alone refuses it.
      `model-${'x'.repeat(195)}`,
      'sk-0123456789abcdef', 'SK-abc', 'A'.repeat(40), 'a_b'.repeat(14),
    ]) {
      await expect((async () => shell.invoke('choose-model', { service: 'deepseek', model }))(), JSON.stringify(model))
        .rejects.toThrow('no name of a model');
    }
    await expect((async () => shell.invoke('choose-model', undefined))()).rejects.toThrow('not among those that are set up');
    expect(shell.setups).toHaveLength(runs);
  });

  it('keeps a level of thinking for a service that the window was told of, and with it nothing else', async () => {
    const shell = await told();
    const stored = { effort: { name: 'deepseek', effort: 'max' } };
    shell.setup.answer = () => ({ out: `${JSON.stringify(stored)}\n`, code: 0 });
    const asked = { service: 'deepseek', effort: 'max', model: 'deepseek-pro', api_key: 'sk-never' };
    await expect(shell.invoke('choose-effort', asked)).resolves.toEqual(stored);
    const run = shell.setups.at(-1)!;
    expect(run.args).toEqual(['-m', 'columbina', 'setup', '--from-stdin']);
    expect(JSON.parse(run.input!)).toEqual({ service: 'deepseek', effort: 'max' });
    for (const effort of ['default', 'off', 'low', 'high']) {
      await shell.invoke('choose-effort', { service: 'ollama', effort });
      expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ service: 'ollama', effort });
    }
  });

  it('keeps no level for a service that the window was not told of, nor ultra, nor anything that is no level', async () => {
    const shell = await told();
    const runs = shell.setups.length;
    await expect((async () => shell.invoke('choose-effort', { service: 'openai', effort: 'max' }))()).rejects.toThrow('not among those that are set up');
    for (const effort of ['ultra', 'MAX', ' max', 'loud', '', 3, undefined, ['max']]) {
      await expect((async () => shell.invoke('choose-effort', { service: 'deepseek', effort }))(), JSON.stringify(effort))
        .rejects.toThrow('no level of thinking that is kept');
    }
    await expect((async () => shell.invoke('choose-effort', undefined))()).rejects.toThrow('not among those that are set up');
    expect(shell.setups).toHaveLength(runs);
  });

  it('writes what is chosen one thing at a time, in the order in which it was chosen', async () => {
    const shell = await told();
    const before = shell.setups.length;
    const written = () => shell.setups.slice(before).map(run => JSON.parse(run.input!));
    const low = shell.invoke('choose-effort', { service: 'deepseek', effort: 'low' });
    const model = shell.invoke('choose-model', { service: 'deepseek', model: 'deepseek-pro' });
    const max = shell.invoke('choose-effort', { service: 'deepseek', effort: 'max' });
    await settled();
    // Each write reads all that is stored and writes it back: the others wait for the first.
    expect(written()).toEqual([{ service: 'deepseek', effort: 'low' }]);
    answer(shell.setups.at(-1)!, { effort: { name: 'deepseek', effort: 'low' } });
    await expect(low).resolves.toEqual({ effort: { name: 'deepseek', effort: 'low' } });
    await settled();
    expect(written()).toEqual([{ service: 'deepseek', effort: 'low' }, { service: 'deepseek', model: 'deepseek-pro' }]);
    answer(shell.setups.at(-1)!, { stored: { name: 'deepseek', model: 'deepseek-pro' }, key_in: null });
    await model;
    await settled();
    expect(written().at(-1)).toEqual({ service: 'deepseek', effort: 'max' });
    answer(shell.setups.at(-1)!, { effort: { name: 'deepseek', effort: 'max' } });
    await expect(max).resolves.toEqual({ effort: { name: 'deepseek', effort: 'max' } });
  });

  it('writes what is chosen next where what was chosen before could not be written', async () => {
    const shell = await told();
    const before = shell.setups.length;
    const failed = shell.invoke('choose-effort', { service: 'deepseek', effort: 'low' });
    const next = shell.invoke('choose-effort', { service: 'deepseek', effort: 'max' });
    await settled();
    shell.setups.at(-1)!.child.emit('error', new Error('spawn ENOENT'));
    await expect(failed).rejects.toThrow('could not be started: spawn ENOENT');
    await settled();
    expect(shell.setups).toHaveLength(before + 2);
    answer(shell.setups.at(-1)!, { effort: { name: 'deepseek', effort: 'max' } });
    await expect(next).resolves.toEqual({ effort: { name: 'deepseek', effort: 'max' } });
  });

  it('answers none of this to anything but the page of the application', async () => {
    const shell = await told();
    const runs = shell.setups.length;
    await expect((async () => shell.handlers.get('columbina:models')!(shell.unrelated, 'deepseek'))()).rejects.toThrow('IPC sender');
    await expect((async () => shell.handlers.get('columbina:choose-model')!(shell.unrelated, { service: 'deepseek', model: 'deepseek-pro' }))())
      .rejects.toThrow('IPC sender');
    await expect((async () => shell.handlers.get('columbina:choose-effort')!(shell.unrelated, { service: 'deepseek', effort: 'max' }))())
      .rejects.toThrow('IPC sender');
    expect(shell.setups).toHaveLength(runs);
  });
});

describe('files that go with a message', () => {
  const MOST = 25 * 1024 * 1024;
  /** The shell, with the service at work in a project, unless a test says that it replays. */
  const working = async (env: Record<string, string> = { COLUMBINA_SERVICE_EXECUTABLE: python }) => {
    const shell = await shellHarness(undefined, env);
    const cwd = await shell.select('/work/project');
    await shell.invoke('start', { cwd });
    return { shell, cwd };
  };
  type Shell = Awaited<ReturnType<typeof shellHarness>>;
  type Told = { attached: { id: string; name: string; size: number }[]; refused: { name: string; why: string }[] };
  /** A file of the user, outside the project. */
  const at = (...names: string[]) => path.resolve('/home/me', ...names);
  const put = (shell: Shell, name: string, size = 100) => {
    shell.drive.set(at(name), { size });
    return at(name);
  };
  /** Choose files in the dialog of the system. */
  const choose = async (shell: Shell, ...chosen: string[]) => {
    shell.dialog.showOpenDialog.mockResolvedValueOnce({ canceled: false, filePaths: chosen });
    return await shell.invoke('choose-attachments', { language: 'ja' }) as Told;
  };
  const kept = (cwd: string, ...names: string[]) => path.join(cwd, '.columbina', 'attachments', ...names);

  it('are chosen in the dialog of the system, titled in the language of the window, and told of by name and size', async () => {
    const { shell } = await working();
    const told = await choose(shell, put(shell, 'notes.md', 1200), put(shell, 'photo.png', 3_000_000));
    const [over, options] = shell.dialog.showOpenDialog.mock.calls.at(-1)!;
    expect(over).toBe(shell.window);
    expect(options).toEqual({ title: '添付するファイルを選択', properties: ['openFile', 'multiSelections'] });
    expect(told).toEqual({
      attached: [{ id: expect.any(String), name: 'notes.md', size: 1200 }, { id: expect.any(String), name: 'photo.png', size: 3_000_000 }],
      refused: [],
    });
    expect(new Set(told.attached.map(each => each.id)).size).toBe(2);
    // Where the files lie stays with the shell.
    expect(JSON.stringify(told)).not.toContain('home');
    const titled = async (asked: unknown) => {
      shell.dialog.showOpenDialog.mockResolvedValueOnce({ canceled: true, filePaths: [] });
      expect(await shell.invoke('choose-attachments', asked)).toBeNull();
      return shell.dialog.showOpenDialog.mock.calls.at(-1)![1].title;
    };
    expect(await titled({ language: 'en' })).toBe('Choose files to attach');
    expect(await titled({ language: 'zh-CN' })).toBe('选择要附上的文件');
    expect(await titled({ language: 'de' })).toBe('Choose files to attach');
    expect(await titled(undefined)).toBe('Choose files to attach');
  });

  it('take a file up to 25 MB, and say of every other one why it is not taken', async () => {
    const { shell } = await working();
    shell.drive.set(at('photos'), 'folder');
    const told = await choose(shell, at('photos'), put(shell, 'film.mp4', MOST + 1), at('gone.txt'), put(shell, 'exactly.bin', MOST));
    expect(told.attached.map(each => [each.name, each.size])).toEqual([['exactly.bin', MOST]]);
    expect(told.refused).toEqual([
      { name: 'photos', why: 'notAFile' }, { name: 'film.mp4', why: 'tooBig' }, { name: 'gone.txt', why: 'gone' },
    ]);
  });

  it('take the files that were dropped, as the preload names them, and nothing that names none', async () => {
    const { shell } = await working();
    const notes = put(shell, 'notes.md');
    expect(await shell.invoke('attach-paths', [notes])).toEqual({ attached: [{ id: expect.any(String), name: 'notes.md', size: 100 }], refused: [] });
    for (const given of [undefined, notes, [7], ['notes.md'], Array<string>(201).fill(notes)]) {
      await expect((async () => shell.invoke('attach-paths', given))(), String(given).slice(0, 40)).rejects.toThrow('no files that were dropped');
    }
  });

  it('hold two hundred files at most, for all the messages that are being written', async () => {
    const { shell } = await working();
    const notes = put(shell, 'notes.md');
    const first = await shell.invoke('attach-paths', Array<string>(200).fill(notes)) as Told;
    expect(first.attached).toHaveLength(200);
    expect(await shell.invoke('attach-paths', [notes])).toEqual({ attached: [], refused: [{ name: 'notes.md', why: 'tooMany' }] });
    // One that is let go of makes room for one more.
    await shell.invoke('forget-attachments', [first.attached[0].id]);
    expect((await shell.invoke('attach-paths', [notes]) as Told).attached).toHaveLength(1);
  });

  it('are copied into the project when the message goes, beside a file that keeps them out of the history', async () => {
    vi.useFakeTimers({ now: new Date(2026, 9, 16, 15, 30, 12), toFake: ['Date'] });
    try {
      const { shell, cwd } = await working();
      const told = await choose(shell, put(shell, 'notes.md'), put(shell, 'data.csv', 2048));
      const ids = told.attached.map(each => each.id);
      expect(await shell.invoke('keep-attachments', ids)).toEqual({
        kept: ['.columbina/attachments/20261016-153012/notes.md', '.columbina/attachments/20261016-153012/data.csv'],
      });
      expect(shell.drive.get(kept(cwd, '20261016-153012', 'data.csv'))).toEqual({ size: 2048 });
      expect(shell.disk.mkdir).toHaveBeenCalledWith(kept(cwd, '20261016-153012'), { recursive: true });
      expect(shell.files.get(kept(cwd, '.gitignore'))).toBe('*\n');
      // A copy is made only where nothing is there yet.
      expect(shell.disk.copyFile.mock.calls.map(called => called[2])).toEqual([1, 1]);
      // The files are held until the page lets go of them, so that a message that did not go
      // can be sent again with them.
      expect(await shell.invoke('keep-attachments', ids)).toEqual({
        kept: ['.columbina/attachments/20261016-153012/notes (2).md', '.columbina/attachments/20261016-153012/data (2).csv'],
      });
      await shell.invoke('forget-attachments', ids);
      await expect((async () => shell.invoke('keep-attachments', ids))()).rejects.toThrow('was not chosen');
    } finally {
      vi.useRealTimers();
    }
  });

  it('leave a file that keeps them out of the history as it is, and give a name that is there a number', async () => {
    vi.useFakeTimers({ now: new Date(2026, 0, 2, 3, 4, 5), toFake: ['Date'] });
    try {
      const { shell, cwd } = await working();
      shell.drive.set(kept(cwd, '.gitignore'), { size: 8 });
      shell.files.set(kept(cwd, '.gitignore'), 'written\n');
      shell.drive.set(at('other', 'notes.md'), { size: 1 });
      shell.drive.set(at('other', 'Makefile'), { size: 1 });
      shell.drive.set(at('other', '.env'), { size: 1 });
      const told = await choose(
        shell, put(shell, 'notes.md'), at('other', 'notes.md'), put(shell, 'Makefile'), at('other', 'Makefile'),
        put(shell, '.env'), at('other', '.env'), put(shell, 'odd`name\u0007.txt'),
      );
      const answer = await shell.invoke('keep-attachments', told.attached.map(each => each.id)) as { kept: string[] };
      // A name that begins with a dot has no ending: the number comes at its end.
      expect(answer.kept.map(each => each.split('/').slice(2).join('/'))).toEqual([
        '20260102-030405/notes.md', '20260102-030405/notes (2).md', '20260102-030405/Makefile', '20260102-030405/Makefile (2)',
        '20260102-030405/.env', '20260102-030405/.env (2)', '20260102-030405/odd`name_.txt',
      ]);
      expect(shell.files.get(kept(cwd, '.gitignore'))).toBe('written\n');
    } finally {
      vi.useRealTimers();
    }
  });

  it('copy nothing where a file is gone or no longer fits, or where no project is there to keep them', async () => {
    const { shell } = await working();
    const ids = async (name: string, size = 100) => (await choose(shell, put(shell, name, size))).attached.map(each => each.id);
    const refused = async (given: unknown, said: string) => {
      await expect((async () => shell.invoke('keep-attachments', given))(), JSON.stringify(given)).rejects.toThrow(said);
    };
    const notes = await ids('notes.md');
    const gone = await ids('gone.md');
    shell.drive.delete(at('gone.md'));
    await refused([...notes, ...gone], 'gone.md is not there any more');
    const grown = await ids('grown.md');
    shell.drive.set(at('grown.md'), { size: MOST + 1 });
    await refused(grown, 'grown.md is no longer a file');
    const folder = await ids('folder.md');
    shell.drive.set(at('folder.md'), 'folder');
    await refused(folder, 'folder.md is no longer a file');
    for (const given of [undefined, 'attachment-1', [], [7], ['attachment-999']]) await refused(given, given === undefined || typeof given === 'string' || (Array.isArray(given) && given.length === 0) ? 'no set of files' : 'was not chosen');
    await refused(Array<string>(11).fill(notes[0]), 'no set of files');
    expect(shell.disk.copyFile).not.toHaveBeenCalled();
    // What was refused is still held, and goes once the project is there for it.
    expect(await shell.invoke('keep-attachments', notes)).toMatchObject({ kept: [expect.stringMatching(/notes\.md$/)] });
    const later = await ids('later.md');
    await shell.invoke('stop');
    await refused(later, 'No project is open');
  });

  it('are kept nowhere in a replay, which has no project of its own', async () => {
    const { shell } = await working({});
    const told = await choose(shell, put(shell, 'notes.md'));
    await expect((async () => shell.invoke('keep-attachments', told.attached.map(each => each.id)))()).rejects.toThrow('No project is open');
    expect(shell.disk.copyFile).not.toHaveBeenCalled();
  });

  it('let go of files that are not to go with a message after all', async () => {
    const { shell } = await working();
    const told = await choose(shell, put(shell, 'one.md'), put(shell, 'two.md'));
    const [one, two] = told.attached.map(each => each.id);
    await shell.invoke('forget-attachments', [one, 7, 'attachment-999']);
    await shell.invoke('forget-attachments', 'nothing that is a list');
    await expect((async () => shell.invoke('keep-attachments', [one]))()).rejects.toThrow('was not chosen');
    expect(await shell.invoke('keep-attachments', [two])).toMatchObject({ kept: [expect.stringMatching(/two\.md$/)] });
  });

  it('answer none of this to anything but the page of the application', async () => {
    const { shell } = await working();
    const notes = put(shell, 'notes.md');
    const told = await choose(shell, notes);
    const from = (name: string, ...given: unknown[]) => (async () => shell.handlers.get(`columbina:${name}`)!(shell.unrelated, ...given))();
    await expect(from('choose-attachments', { language: 'en' })).rejects.toThrow('IPC sender');
    await expect(from('attach-paths', [notes])).rejects.toThrow('IPC sender');
    await expect(from('forget-attachments', [told.attached[0].id])).rejects.toThrow('IPC sender');
    await expect(from('keep-attachments', [told.attached[0].id])).rejects.toThrow('IPC sender');
    expect(shell.dialog.showOpenDialog).toHaveBeenCalledTimes(2);
    expect(shell.disk.copyFile).not.toHaveBeenCalled();
    // The file is still held: nothing that was asked from elsewhere let go of it.
    expect(await shell.invoke('keep-attachments', [told.attached[0].id])).toMatchObject({ kept: [expect.any(String)] });
  });
});

describe('tasks at set times, worktrees, the groups of QQ and pasted pictures', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const answering = async (out: unknown = {}) => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: `${JSON.stringify(out)}\n`, code: 0 });
    return shell;
  };
  const program = (shell: Awaited<ReturnType<typeof shellHarness>>) => shell.setups.at(-1)!.args.slice(2);

  it('has the program list, add and change the tasks, with every value in one word with its option', async () => {
    const shell = await answering({ schedules: [] });
    await expect(shell.invoke('schedules', { what: 'list' })).resolves.toEqual({ schedules: [] });
    expect(program(shell)).toEqual(['schedules', 'list', '--json']);
    const cwd = await shell.select('/work/project');
    const task = {
      name: '--every=x', cwd, prompt: '-p Sum up.', every: 'week', weekday: 'fri', at: '17:00', missed: 'skip', minutes: 30,
    };
    await shell.invoke('schedules', { what: 'add', task });
    expect(program(shell)).toEqual([
      'schedules', 'add', '--name=--every=x', `--cwd=${cwd}`, '--prompt=-p Sum up.', '--every=week', '--minutes=30',
      '--at=17:00', '--weekday=fri', '--missed=skip', '--json',
    ]);
    await shell.invoke('schedules', { what: 'add', task: { name: 'n', cwd, prompt: 'p', every: 'day', at: 'noon', weekday: 'xyz' } });
    expect(program(shell)).toEqual(['schedules', 'add', '--name=n', `--cwd=${cwd}`, '--prompt=p', '--every=day', '--json']);
    await shell.invoke('schedules', { what: 'disable', id: 'a1b2' });
    expect(program(shell)).toEqual(['schedules', 'disable', 'a1b2', '--json']);
  });

  it('adds no task in a folder that was not chosen, or that is no task at all', async () => {
    const shell = await answering();
    const cwd = await shell.select('/work/project');
    const runs = shell.setups.length;
    for (const asked of [
      { what: 'add', task: { name: 'n', cwd: path.resolve('/elsewhere'), prompt: 'p', every: 'day' } },
      { what: 'add', task: { name: ' ', cwd, prompt: 'p', every: 'day' } },
      { what: 'add', task: { name: 'n', cwd, prompt: 'a\u0007b', every: 'day' } },
      { what: 'add', task: { name: 'n', cwd, prompt: 'p', every: 'fortnight' } },
      { what: 'remove', id: '--all' },
      { what: 'run', id: 'a1' },
      undefined,
    ]) {
      await expect((async () => shell.invoke('schedules', asked))(), JSON.stringify(asked)).rejects.toThrow();
    }
    expect(shell.setups).toHaveLength(runs);
    shell.setup.answer = () => ({ out: '{"error": "there is no task a1"}\n', code: 2 });
    await expect(shell.invoke('schedules', { what: 'remove', id: 'a1' })).rejects.toThrow('there is no task a1');
    await expect((async () => shell.handlers.get('columbina:schedules')!(shell.unrelated, { what: 'list' }))()).rejects.toThrow('IPC sender');
  });

  it('has the program make, list and end worktrees in a project that was chosen, and opens what it made', async () => {
    const shell = await answering();
    const cwd = await shell.select('/work/project');
    const made = path.join(cwd, '.columbina', 'worktrees', 'fix');
    shell.setup.answer = () => ({ out: `${JSON.stringify({ name: 'fix', path: made, branch: 'columbina/fix', done: 'add' })}\n`, code: 0 });
    await shell.invoke('worktrees', { what: 'add', cwd, name: ' -fix ' });
    expect(program(shell)).toEqual(['worktrees', 'add', `--cwd=${cwd}`, '--json', '--', '-fix']);
    // The worktree that was made may be started in, as a folder that was chosen is.
    await expect(shell.invoke('start', { cwd: made })).resolves.toMatchObject({ cwd: made });
    const listed = path.join(cwd, '.columbina', 'worktrees', 'other');
    shell.setup.answer = () => ({ out: `${JSON.stringify({ worktrees: [{ name: 'other', path: listed }, 'nothing'] })}\n`, code: 0 });
    await shell.invoke('worktrees', { what: 'list', cwd });
    expect(program(shell)).toEqual(['worktrees', 'list', `--cwd=${cwd}`, '--json']);
    await expect(shell.invoke('start', { cwd: listed })).resolves.toMatchObject({ cwd: listed });
    shell.setup.answer = () => ({ out: '{"name": "fix", "done": "remove"}\n', code: 0 });
    await shell.invoke('worktrees', { what: 'remove', cwd, name: 'fix', force: true });
    expect(program(shell)).toEqual(['worktrees', 'remove', `--cwd=${cwd}`, '--json', '--force', '--', 'fix']);
    await shell.invoke('worktrees', { what: 'merge', cwd, name: 'fix', force: true });
    expect(program(shell)).toEqual(['worktrees', 'merge', `--cwd=${cwd}`, '--json', '--', 'fix']);
  });

  it('ends no worktree outside a project that was chosen, nor one without a name, and says what the program said', async () => {
    const shell = await answering();
    const cwd = await shell.select('/work/project');
    const runs = shell.setups.length;
    for (const asked of [
      { what: 'list', cwd: path.resolve('/elsewhere') },
      { what: 'prune', cwd },
      { what: 'add', cwd, name: ' ' },
      { what: 'add', cwd, name: 'a\nb' },
      { what: 'add', cwd, name: 'x'.repeat(101) },
    ]) {
      await expect((async () => shell.invoke('worktrees', asked))(), JSON.stringify(asked)).rejects.toThrow();
    }
    expect(shell.setups).toHaveLength(runs);
    shell.setup.answer = () => ({ out: '{"error": "there is no worktree x"}\n', code: 2 });
    await expect(shell.invoke('worktrees', { what: 'merge', cwd, name: 'x' })).rejects.toThrow('there is no worktree x');
    // A path that the program names and that is gone is passed over.
    shell.setup.answer = () => ({ out: `${JSON.stringify({ name: 'gone', path: path.resolve('/gone') })}\n`, code: 0 });
    shell.realpathSync.mockImplementation((value: string) => {
      if (path.resolve(value) === path.resolve('/gone')) throw new Error('ENOENT');
      return path.resolve(value);
    });
    await expect(shell.invoke('worktrees', { what: 'add', cwd, name: 'gone' })).resolves.toMatchObject({ name: 'gone' });
  });

  it('has the program list, name, show and forget the groups of QQ', async () => {
    const shell = await answering({ groups: [] });
    await shell.invoke('qq-groups', { what: 'list' });
    expect(program(shell)).toEqual(['qq', '--groups', '--json']);
    await shell.invoke('qq-groups', { what: 'name', id: 'G1', name: ' 家人群 ' });
    expect(program(shell)).toEqual(['qq', '--name-group', 'G1', '家人群', '--json']);
    await shell.invoke('qq-groups', { what: 'name', id: 'G1', name: '' });
    expect(program(shell)).toEqual(['qq', '--name-group', 'G1', ' ', '--json']);
    await shell.invoke('qq-groups', { what: 'memory', id: 'G1' });
    expect(program(shell)).toEqual(['qq', '--memory=G1', '--json']);
    await shell.invoke('qq-groups', { what: 'forget', id: 'G1' });
    expect(program(shell)).toEqual(['qq', '--forget-memory=G1', '--json']);
    const runs = shell.setups.length;
    for (const asked of [
      { what: 'name', id: '--all', name: 'x' },
      { what: 'name', id: 'G1', name: '-x' },
      { what: 'name', id: 'G1', name: 'x'.repeat(65) },
      { what: 'name', id: 'G1', name: 7 },
      { what: 'forget' },
      { what: 'rename', id: 'G1' },
    ]) {
      await expect((async () => shell.invoke('qq-groups', asked))(), JSON.stringify(asked)).rejects.toThrow();
    }
    expect(shell.setups).toHaveLength(runs);
    shell.setup.answer = () => ({ out: '{"error": "no bot of QQ is set up"}\n', code: 2 });
    await expect(shell.invoke('qq-groups', { what: 'list' })).rejects.toThrow('no bot of QQ is set up');
  });

  it('writes pasted pictures into files of its own, and holds them as chosen files', async () => {
    const shell = await shellHarness(undefined, real);
    const bytes = new Uint8Array([137, 80, 78, 71]).buffer;
    const told = await shell.invoke('attach-pasted', [{ type: 'image/png', bytes }, { type: 'image/jpeg', bytes }]) as {
      attached: { name: string; size: number }[];
    };
    expect(told.attached.map(each => each.name)).toEqual([
      expect.stringMatching(/^pasted-\d{8}-\d{6}-1\.png$/), expect.stringMatching(/^pasted-\d{8}-\d{6}-2\.jpg$/),
    ]);
    expect(shell.disk.mkdir).toHaveBeenCalledWith(path.resolve('/columbina-test/temp/columbina-pasted'), { recursive: true });
    for (const given of [
      undefined, [], [{ type: 'image/svg+xml', bytes }], [{ type: 'image/png', bytes: new ArrayBuffer(0) }],
      [{ type: 'image/png', bytes: 'not bytes' }], Array(11).fill({ type: 'image/png', bytes }),
    ]) {
      await expect((async () => shell.invoke('attach-pasted', given))(), String(given)).rejects.toThrow();
    }
  });
});

describe('signing in to GitHub', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const status = {
    signed_in: { login: 'octocat', name: 'The Octocat', avatar: null },
    install_url: 'https://github.com/apps/columbina/installations/new',
    repositories: ['octocat/hello'],
  };
  const toldOfGitHub = (shell: Awaited<ReturnType<typeof shellHarness>>) => shell.sent
    .filter(([channel]) => channel === 'columbina:github').map(([, value]) => value);
  const readerOf = (shell: Awaited<ReturnType<typeof shellHarness>>, child: any) =>
    shell.lineReaders.find((reader: any) => reader.input === child.stdout);

  it('asks who is signed in, with room for a picture, and says whether GitHub was to wait', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: `${JSON.stringify(status)}\n`, code: 0 });
    await expect(shell.invoke('github-status')).resolves.toEqual({ ...status, skipped: false });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'login', '--status', '--json']);
    // A picture as an address of data is longer than the answer of setup may be.
    const picture = `data:image/png;base64,${'A'.repeat(1_400_000)}`;
    const pictured = { ...status, signed_in: { ...status.signed_in, avatar: picture } };
    shell.setup.answer = () => ({ out: `${JSON.stringify(pictured)}\n`, code: 0 });
    await expect(shell.invoke('github-status')).resolves.toEqual({ ...pictured, skipped: false });
  });

  it('has nothing to say of GitHub in a replay', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('github-status')).resolves.toBeNull();
    expect(shell.setups).toEqual([]);
    await expect((async () => shell.invoke('github-sign-in'))()).rejects.toThrow('No service program');
    await expect((async () => shell.invoke('github-sign-out'))()).rejects.toThrow('No service program');
    expect(shell.signIns).toEqual([]);
  });

  it('keeps whether GitHub is to wait, from one start to the next', async () => {
    const shell = await shellHarness(undefined, real);
    shell.setup.answer = () => ({ out: '{"signed_in": null, "install_url": "https://github.com/apps/x/installations/new"}\n', code: 0 });
    await shell.invoke('skip-github', true);
    expect(shell.preferences().githubSkipped).toBe(true);
    await expect(shell.invoke('github-status')).resolves.toMatchObject({ skipped: true });
    // Only a yes is a yes.
    await shell.invoke('skip-github', 'yes');
    expect(shell.preferences().githubSkipped).toBe(false);

    const again = await shellHarness(JSON.stringify({ theme: 'moonveil', recent: [], githubSkipped: true }), real);
    again.setup.answer = shell.setup.answer;
    await expect(again.invoke('github-status')).resolves.toMatchObject({ skipped: true });
    const odd = await shellHarness(JSON.stringify({ githubSkipped: 'true' }), real);
    odd.setup.answer = shell.setup.answer;
    await expect(odd.invoke('github-status')).resolves.toMatchObject({ skipped: false });
  });

  it('tells the page each document that a sign-in writes as it comes, and then that it ended', async () => {
    const shell = await shellHarness(undefined, real);
    await expect(shell.invoke('github-sign-in')).resolves.toBeUndefined();
    const [child] = shell.signIns;
    expect(child.args).toEqual(['-m', 'columbina', 'login', 'github', '--json']);
    expect(child.options.cwd).toBe(path.resolve('/columbina-test/home'));
    // On Windows a tree of processes is ended by its first one. Elsewhere it is a group of its own.
    expect(child.options).toMatchObject({ detached: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    const reader = readerOf(shell, child);
    reader.emit('line', '{"code": "WDJB-MJHT", "url": "https://github.com/login/device", "expires_in": 899}');
    // What is no document of JSON is told of to nobody.
    for (const odd of ['not JSON', '[1, 2]', 'null', '7', `{"long": "${'x'.repeat(2 * 1024 * 1024)}"}`]) reader.emit('line', odd);
    reader.emit('line', '{"signed_in": {"login": "octocat", "name": null, "avatar": null}}');
    child.emit('close', 0, null);
    expect(toldOfGitHub(shell)).toEqual([
      { code: 'WDJB-MJHT', url: 'https://github.com/login/device', expires_in: 899 },
      { signed_in: { login: 'octocat', name: null, avatar: null } },
      { ended: true },
    ]);
    expect(reader.close).toHaveBeenCalledOnce();
  });

  it('runs one sign-in at a time: one that is begun takes the place of the one that runs', async () => {
    const shell = await shellHarness(undefined, real);
    await shell.invoke('github-sign-in');
    await shell.invoke('github-sign-in');
    expect(shell.signIns).toHaveLength(2);
    const [first, second] = shell.signIns;
    expect(first.stdin.end).toHaveBeenCalledOnce();
    expect(second.stdin.end).not.toHaveBeenCalled();
    // What the first one writes from then on, and its end, are told of to nobody.
    readerOf(shell, first).emit('line', '{"error": "stopped", "code": "STOPPED"}');
    first.emit('close', 130, null);
    readerOf(shell, second).emit('line', '{"code": "WDJB-MJHT", "url": "https://github.com/login/device", "expires_in": 899}');
    second.emit('close', 0, null);
    expect(toldOfGitHub(shell)).toEqual([
      { code: 'WDJB-MJHT', url: 'https://github.com/login/device', expires_in: 899 },
      { ended: true },
    ]);
    // Another begins once that one has ended.
    await shell.invoke('github-sign-in');
    expect(shell.signIns).toHaveLength(3);
    expect(second.stdin.end).not.toHaveBeenCalled();
  });

  it('stops a sign-in by closing its input, and ends it where it has not ended a few seconds later', async () => {
    vi.useFakeTimers();
    try {
      const shell = await shellHarness(undefined, real);
      await shell.invoke('github-sign-in');
      const [child] = shell.signIns;
      await shell.invoke('github-cancel');
      expect(child.stdin.end).toHaveBeenCalledOnce();
      // It is heard no more: the page that stopped it knows.
      readerOf(shell, child).emit('line', '{"error": "stopped", "code": "STOPPED"}');
      await vi.advanceTimersByTimeAsync(2_900);
      expect(shell.spawn.mock.calls.some(call => call[0] === 'taskkill.exe')).toBe(false);
      await vi.advanceTimersByTimeAsync(200);
      expect(shell.spawn.mock.calls.at(-1)!.slice(0, 2)).toEqual(['taskkill.exe', ['/pid', String(child.pid), '/T', '/F']]);
      child.emit('close', null, 'SIGKILL');
      expect(toldOfGitHub(shell)).toEqual([]);
      // A stop where none runs stops nothing.
      await shell.invoke('github-cancel');
      expect(child.stdin.end).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it('ends a sign-in that it put in the place of another, where that one has not ended a few seconds later', async () => {
    vi.useFakeTimers();
    try {
      const shell = await shellHarness(undefined, real);
      await shell.invoke('github-sign-in');
      await shell.invoke('github-sign-in');
      const [first, second] = shell.signIns;
      await vi.advanceTimersByTimeAsync(3_100);
      const killed = shell.spawn.mock.calls.filter(call => call[0] === 'taskkill.exe').map(call => call[1][1]);
      expect(killed).toEqual([String(first.pid)]);
      expect(second.stdin.end).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('ends nothing where the sign-in ended once its input was closed, and stops none where none runs', async () => {
    vi.useFakeTimers();
    try {
      const shell = await shellHarness(undefined, real);
      await shell.invoke('github-cancel');
      await shell.invoke('github-sign-in');
      const [child] = shell.signIns;
      await shell.invoke('github-cancel');
      child.emit('close', 0, null);
      await vi.advanceTimersByTimeAsync(5_000);
      expect(shell.spawn.mock.calls.some(call => call[0] === 'taskkill.exe')).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops a sign-in when the page is gone, and when the windows are closed', async () => {
    const shell = await shellHarness(undefined, real);
    await shell.invoke('github-sign-in');
    shell.window.webContents.emit('render-process-gone');
    expect(shell.signIns[0].stdin.end).toHaveBeenCalledOnce();
    shell.signIns[0].emit('close', 0, null);
    await shell.invoke('github-sign-in');
    shell.app.emit('window-all-closed');
    expect(shell.signIns[1].stdin.end).toHaveBeenCalledOnce();
  });

  it('says why a sign-in could not be started, and lets another begin', async () => {
    const shell = await shellHarness(undefined, real);
    const failed = Object.assign(new EventEmitter(), {
      pid: 0, exitCode: null as number | null, signalCode: null,
      stdout: new EventEmitter(), stderr: new EventEmitter(), stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
    });
    shell.spawn.mockImplementationOnce(() => {
      queueMicrotask(() => failed.emit('error', new Error('spawn ENOENT')));
      return failed;
    });
    await expect((async () => shell.invoke('github-sign-in'))()).rejects.toThrow('The sign-in could not be started: spawn ENOENT');
    // It is no sign-in that runs: its end is told of to nobody, and there is nothing of it to stop.
    failed.emit('close', -2, null);
    await shell.invoke('github-cancel');
    expect(toldOfGitHub(shell)).toEqual([]);
    expect(failed.stdin.end).not.toHaveBeenCalled();
    await expect(shell.invoke('github-sign-in')).resolves.toBeUndefined();
    // An error of a program that started, such as one of a stop, leaves it to end by itself.
    const [child] = shell.signIns;
    child.emit('error', new Error('kill EPERM'));
    child.emit('error', new Error('kill EPERM'));
    readerOf(shell, child).emit('line', '{"code": "WDJB-MJHT", "url": "https://github.com/login/device", "expires_in": 899}');
    child.emit('close', 0, null);
    expect(toldOfGitHub(shell)).toEqual([
      { code: 'WDJB-MJHT', url: 'https://github.com/login/device', expires_in: 899 },
      { ended: true },
    ]);
  });

  it('signs out with the program, and answers what it says', async () => {
    const shell = await shellHarness(undefined, real);
    const said = { signed_out: true, revoke: 'https://github.com/settings/apps/authorizations' };
    shell.setup.answer = () => ({ out: `${JSON.stringify(said)}\n`, code: 0 });
    await expect(shell.invoke('github-sign-out')).resolves.toEqual(said);
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'logout', 'github', '--json']);
  });

  it('answers none of this to anything but the page of the application', async () => {
    const shell = await shellHarness(undefined, real);
    for (const name of ['github-status', 'github-sign-in', 'github-cancel', 'github-sign-out', 'skip-github']) {
      await expect((async () => shell.handlers.get(`columbina:${name}`)!(shell.unrelated, true))()).rejects.toThrow('IPC sender');
    }
    expect(shell.signIns).toEqual([]);
    expect(shell.setups).toEqual([]);
  });
});

describe('newer versions of the application', () => {
  const RELEASES = 'https://api.github.com/repos/Yueran-ML/agent-harness/releases?per_page=30';
  const pageOf = (version: string) => `https://github.com/Yueran-ML/agent-harness/releases/tag/desktop-v${version}`;
  /** A release as GitHub lists it: published, and not one that is tried before it is released. */
  const release = (tag: string, given: Record<string, unknown> = {}) => ({ tag_name: tag, draft: false, prerelease: false, ...given });
  /** What GitHub answers, with a status, and a body as it is sent. */
  const answered = (body: unknown, status = 200) => ({
    status, ok: status >= 200 && status < 300, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)),
  });
  const none = { newer: null, page: null, failure: null };
  /** A package whose GitHub lists these releases. */
  const packageWith = async (releases: unknown[], system: System = {}) => {
    const shell = await shellHarness(undefined, {}, { packaged: true, ...system });
    shell.net.fetch.mockResolvedValue(answered(releases));
    return shell;
  };

  it('says the version that runs, and looks for newer ones only in a package', async () => {
    const source = await shellHarness();
    expect(source.invoke('about')).toEqual({ version: '0.1.2', checks: false });
    await expect(source.invoke('updates', { now: true })).rejects.toThrow('does not look for newer versions');
    expect(source.net.fetch).not.toHaveBeenCalled();
    const packaged = await packageWith([], { version: '0.3.1' });
    expect(packaged.invoke('about')).toEqual({ version: '0.3.1', checks: true });
  });

  it('asks where the environment names from source, which the walks through the window use, and only there', async () => {
    const feed = 'http://127.0.0.1:8123/releases';
    const named = await shellHarness(undefined, { COLUMBINA_UPDATE_FEED: feed });
    named.net.fetch.mockResolvedValue(answered([release('desktop-v0.1.3')]));
    expect(named.invoke('about')).toEqual({ version: '0.1.2', checks: true });
    await expect(named.invoke('updates', { now: false })).resolves.toEqual({ newer: '0.1.3', page: pageOf('0.1.3'), failure: null });
    expect(named.net.fetch.mock.calls[0][0]).toBe(feed);
    for (const other of ['', 'file:///C:/releases.json', 'ftp://127.0.0.1/releases', 'http://', 'http://127.0.0.1/a b', 'releases']) {
      const shell = await shellHarness(undefined, { COLUMBINA_UPDATE_FEED: other });
      expect(shell.invoke('about')).toEqual({ version: '0.1.2', checks: false });
    }
    // A package asks GitHub, whatever the environment names.
    const packaged = await shellHarness(undefined, { COLUMBINA_UPDATE_FEED: feed }, { packaged: true });
    packaged.net.fetch.mockResolvedValue(answered([]));
    await packaged.invoke('updates', { now: false });
    expect(packaged.net.fetch.mock.calls[0][0]).toBe(RELEASES);
  });

  it('asks GitHub for its releases, and tells of the newest release of the application that comes after the one that runs', async () => {
    const timeout = vi.spyOn(AbortSignal, 'timeout');
    const shell = await packageWith([
      release('v0.9.5'), release('desktop-v0.1.3'), release('desktop-v0.2.0', { draft: true }),
      release('desktop-v0.3.5', { prerelease: true }), release('desktop-v0.1.4-beta'), release('desktop-v0.1.10'),
      null, 'desktop-v9.9.9', release('desktop-v0.1.11', { draft: null }), { tag_name: 'desktop-v3.0.0' },
      { tag_name: 5, draft: false, prerelease: false }, release('desktop-v1234567.0.0'), release('Desktop-v0.3.0'),
      release(' desktop-v0.4.0'), release('desktop-v0.5'), release('desktop-0.6.0'),
    ]);
    await expect(shell.invoke('updates', { now: false })).resolves.toEqual({ newer: '0.1.10', page: pageOf('0.1.10'), failure: null });
    expect(shell.net.fetch).toHaveBeenCalledTimes(1);
    const [address, init] = shell.net.fetch.mock.calls[0];
    expect(address).toBe(RELEASES);
    expect(init.headers).toEqual({ Accept: 'application/vnd.github+json', 'User-Agent': 'Columbina/0.1.2' });
    expect(init.signal).toBeInstanceOf(AbortSignal);
    expect(timeout.mock.calls).toEqual([[15_000]]);
    // Through a session of its own, kept in memory only: that of the window lets nothing out.
    // It is made once.
    await shell.invoke('updates', { now: true });
    expect(shell.partitions).toEqual(['columbina-releases']);
  });

  it('goes by the numbers of a version, the first of them first', async () => {
    const newestOf = async (tags: string[], version = '0.1.2') => {
      const shell = await packageWith(tags.map(tag => release(tag)), { version });
      return ((await shell.invoke('updates', { now: false })) as { newer: string | null }).newer;
    };
    expect(await newestOf(['desktop-v0.9.9', 'desktop-v1.0.0', 'desktop-v0.10.0'])).toBe('1.0.0');
    expect(await newestOf(['desktop-v0.2.0', 'desktop-v0.10.0', 'desktop-v0.9.0'])).toBe('0.10.0');
    expect(await newestOf(['desktop-v0.1.3', 'desktop-v0.1.20', 'desktop-v0.1.4'])).toBe('0.1.20');
    expect(await newestOf(['desktop-v0.2.0'])).toBe('0.2.0');
    expect(await newestOf(['desktop-v1.0.0'])).toBe('1.0.0');
    // A release that is the one that runs, or comes before it, is none to tell of.
    expect(await newestOf(['desktop-v0.1.2'])).toBeNull();
    expect(await newestOf(['desktop-v0.1.1', 'desktop-v0.0.9'])).toBeNull();
    expect(await newestOf(['desktop-v0.9.9', 'desktop-v0.10.0'], '1.0.0')).toBeNull();
    expect(await newestOf(['desktop-v1.0.0'], '1.0.1')).toBeNull();
    expect(await newestOf([])).toBeNull();
    // A version that runs and is no release, as one from source can be, has none after it.
    expect(await newestOf(['desktop-v0.1.3'], '0.1.2-dev')).toBeNull();
  });

  it('says why it is not known: GitHub was not reached, turned the question away, or gave no list', async () => {
    const said = async (answer: () => Promise<unknown>) => {
      const shell = await packageWith([]);
      shell.net.fetch.mockImplementation(answer);
      return ((await shell.invoke('updates', { now: false })) as { failure: string | null }).failure;
    };
    expect(await said(async () => { throw new Error('getaddrinfo ENOTFOUND api.github.com'); })).toBe('network');
    expect(await said(async () => ({ status: 200, ok: true, text: async () => { throw new Error('The connection was reset.'); } }))).toBe('network');
    expect(await said(async () => answered({ message: 'API rate limit exceeded' }, 403))).toBe('busy');
    expect(await said(async () => answered({ message: 'Too many requests' }, 429))).toBe('busy');
    expect(await said(async () => answered({ message: 'Not Found' }, 404))).toBe('unreadable');
    expect(await said(async () => answered('', 500))).toBe('unreadable');
    // What comes with a status of failure is not read, though it were a list.
    expect(await said(async () => answered([release('desktop-v0.1.3')], 500))).toBe('unreadable');
    expect(await said(async () => answered('<html>Unicorn!</html>'))).toBe('unreadable');
    expect(await said(async () => answered({ message: 'Moved' }))).toBe('unreadable');
    expect(await said(async () => answered('null'))).toBe('unreadable');
    // A list beyond what is read is none. One of that length is read.
    const most = 4 * 1024 * 1024;
    expect(await said(async () => answered(`[${' '.repeat(most - 1)}]`))).toBe('unreadable');
    expect(await said(async () => answered(`[${' '.repeat(most - 2)}]`))).toBeNull();
  });

  it('takes an answer of the last hour as it is, unless it is asked now, and asks again after a failure', async () => {
    vi.useFakeTimers({ now: 1_000_000, toFake: ['Date'] });
    try {
      const shell = await packageWith([release('desktop-v0.1.3')]);
      const found = { newer: '0.1.3', page: pageOf('0.1.3'), failure: null };
      await expect(shell.invoke('updates', { now: false })).resolves.toEqual(found);
      shell.net.fetch.mockResolvedValue(answered([release('desktop-v0.1.4')]));
      vi.setSystemTime(1_000_000 + 60 * 60_000 - 1);
      await expect(shell.invoke('updates', { now: false })).resolves.toEqual(found);
      await expect(shell.invoke('updates', {})).resolves.toEqual(found);
      expect(shell.net.fetch).toHaveBeenCalledTimes(1);
      // Asked now, GitHub is asked now, and its answer is the one that is kept from then on.
      await expect(shell.invoke('updates', { now: true })).resolves.toEqual({ newer: '0.1.4', page: pageOf('0.1.4'), failure: null });
      expect(shell.net.fetch).toHaveBeenCalledTimes(2);
      vi.setSystemTime(1_000_000 + 2 * 60 * 60_000 - 2);
      await shell.invoke('updates', { now: false });
      expect(shell.net.fetch).toHaveBeenCalledTimes(2);
      vi.setSystemTime(1_000_000 + 2 * 60 * 60_000);
      shell.net.fetch.mockResolvedValue(answered([]));
      await expect(shell.invoke('updates', { now: false })).resolves.toEqual(none);
      expect(shell.net.fetch).toHaveBeenCalledTimes(3);
      // What failed is not kept: the next question asks GitHub again.
      shell.net.fetch.mockRejectedValue(new Error('offline'));
      await expect(shell.invoke('updates', { now: true })).resolves.toEqual({ ...none, failure: 'network' });
      shell.net.fetch.mockResolvedValue(answered([release('desktop-v0.1.5')]));
      await expect(shell.invoke('updates', { now: false })).resolves.toEqual({ newer: '0.1.5', page: pageOf('0.1.5'), failure: null });
      expect(shell.net.fetch).toHaveBeenCalledTimes(5);
    } finally {
      vi.useRealTimers();
    }
  });

  it('asks one question at a time', async () => {
    const shell = await packageWith([]);
    let answer!: (value: unknown) => void;
    shell.net.fetch.mockReturnValueOnce(new Promise(resolve => { answer = resolve; }));
    const first = shell.invoke('updates', { now: false });
    const second = shell.invoke('updates', { now: true });
    expect(shell.net.fetch).toHaveBeenCalledTimes(1);
    answer(answered([release('desktop-v0.1.3')]));
    const found = { newer: '0.1.3', page: pageOf('0.1.3'), failure: null };
    await expect(first).resolves.toEqual(found);
    await expect(second).resolves.toEqual(found);
    // Once it is answered, the next question that is asked now asks GitHub again.
    await shell.invoke('updates', { now: true });
    expect(shell.net.fetch).toHaveBeenCalledTimes(2);
  });

  it('answers only the page of the application', async () => {
    const shell = await packageWith([release('desktop-v0.1.3')]);
    expect(() => shell.handlers.get('columbina:about')!(shell.unrelated)).toThrow('IPC sender');
    await expect((async () => shell.handlers.get('columbina:updates')!(shell.unrelated, { now: true }))()).rejects.toThrow('IPC sender');
    expect(shell.net.fetch).not.toHaveBeenCalled();
  });
});

describe('the assistant in Feishu', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const app = {
    set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
    running: false, state: null, code: null, error: null,
  };
  const feishuPage = pathToFileURL(path.resolve(path.dirname(mainPath), '../dist/feishu.html')).href;
  const logOfFeishu = path.resolve('/columbina-test/logs/columbina-feishu.log');
  const toldOfFeishu = (shell: Awaited<ReturnType<typeof shellHarness>>) => shell.sent
    .filter(([channel]) => channel === 'columbina:feishu').map(([, value]) => value);
  const readerOf = (shell: Awaited<ReturnType<typeof shellHarness>>, child: any) =>
    shell.lineReaders.find((reader: any) => reader.input === child.stdout);
  /** The shell with an app set up, which the program says of whenever it is asked. */
  const withApp = async (kept?: string) => {
    const shell = await shellHarness(kept, real);
    shell.setup.answer = (args) => (args.includes('--status')
      ? { out: `${JSON.stringify(app)}\n`, code: 0 }
      : args.includes('--forget') ? { out: '{"forgotten": true}\n', code: 0 }
        : { out: '{"stored": {"domain": "feishu", "app_id": "cli_a1b2c3d4"}, "secret_in": "keyring"}\n', code: 0 });
    return shell;
  };
  /** End an assistant that runs, as it ends once its input was closed. */
  const ended = (child: any, code = 0) => {
    child.exitCode = code;
    child.emit('close', code, null);
  };

  it('has nothing to say of it in a replay', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('feishu-status')).resolves.toBeNull();
    await expect((async () => shell.invoke('feishu-start'))()).rejects.toThrow('No service program');
    await expect((async () => shell.invoke('set-up-feishu', { language: 'en' }))()).rejects.toThrow('No service program');
    expect(shell.bots).toEqual([]);
    expect(shell.setups).toEqual([]);
  });

  it('asks the program how the assistant is doing, and adds what the shell knows of it', async () => {
    const shell = await withApp();
    await expect(shell.invoke('feishu-status')).resolves.toEqual({ ...app, own: false, on: false, ended: null });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'feishu', '--status', '--json']);
    expect(shell.setups.at(-1)!.input).toBe('');
  });

  it('starts the assistant, tells the page what it says of itself, and that it ended', async () => {
    const shell = await withApp();
    await expect(shell.invoke('feishu-start')).resolves.toBeUndefined();
    const [child] = shell.bots;
    expect(child.args).toEqual(['-m', 'columbina', 'feishu', '--json']);
    expect(child.options.cwd).toBe(path.resolve('/columbina-test/home'));
    expect(child.options).toMatchObject({ detached: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    expect(child.stderr.pipe).toHaveBeenCalledOnce();
    // It runs again at the next start of the application, until the user stops it.
    expect(shell.preferences().feishuOn).toBe(true);
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ own: true, on: true });

    const reader = readerOf(shell, child);
    reader.emit('line', '{"state": "connecting", "paired": false, "code": "123456", "error": null}');
    // What is no document of JSON, or no object, or longer than such a line is, is passed over.
    for (const line of ['not json', '[1, 2]', 'null', `{"state": "${'x'.repeat(70_000)}"}`]) reader.emit('line', line);
    reader.emit('line', '{"state": "connected", "paired": false, "code": "123456", "error": null}');
    child.stderr.emit('data', Buffer.from('columbina: connected to Feishu\ncolumbina: the session of the assistant cannot be opened: it is open elsewhere\n'));
    ended(child, 1);
    expect(toldOfFeishu(shell)).toEqual([
      { state: 'connecting', paired: false, code: '123456', error: null },
      { state: 'connected', paired: false, code: '123456', error: null },
      { ended: true, code: 1, said: 'the session of the assistant cannot be opened: it is open elsewhere' },
    ]);
    // A page that asks later is told how it ended, until another one starts.
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({
      own: false, ended: { code: 1, said: 'the session of the assistant cannot be opened: it is open elsewhere' },
    });
    await shell.invoke('feishu-start');
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ own: true, ended: null });
  });

  it('tells nothing that an assistant writes once it has ended', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    const reader = readerOf(shell, child);
    ended(child);
    reader.emit('line', '{"state": "connected", "paired": true, "code": null, "error": null}');
    expect(toldOfFeishu(shell)).toEqual([{ ended: true, code: 0, said: '' }]);
  });

  it('runs one assistant at most, and tells nothing of one that ended well', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    await shell.invoke('feishu-start');
    expect(shell.bots).toHaveLength(1);
    // A line that it wrote last and did not end is told as far as it came.
    shell.bots[0].stderr.emit('data', Buffer.from('columbina: stopped'));
    ended(shell.bots[0]);
    expect(toldOfFeishu(shell)).toEqual([{ ended: true, code: 0, said: 'stopped' }]);
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ ended: null });
  });

  it('stops the assistant by closing its input, and ends it where it has not ended a few seconds later', async () => {
    vi.useFakeTimers();
    try {
      const shell = await withApp();
      await shell.invoke('feishu-start');
      const [child] = shell.bots;
      const stopped = shell.invoke('feishu-stop') as Promise<void>;
      expect(child.stdin.end).toHaveBeenCalledOnce();
      // It is not started with the application any more.
      expect(shell.preferences().feishuOn).toBe(false);
      await vi.advanceTimersByTimeAsync(5_900);
      expect(shell.spawn.mock.calls.some(call => call[0] === 'taskkill.exe')).toBe(false);
      await vi.advanceTimersByTimeAsync(200);
      expect(shell.spawn.mock.calls.at(-1)!.slice(0, 2)).toEqual(['taskkill.exe', ['/pid', String(child.pid), '/T', '/F']]);
      // It has stopped once it has ended, and not before.
      let done = false;
      void stopped.then(() => { done = true; });
      await vi.advanceTimersByTimeAsync(1_000);
      expect(done).toBe(false);
      child.emit('close', null, 'SIGKILL');
      await expect(stopped).resolves.toBeUndefined();
      // A stop where none runs stops nothing.
      await shell.invoke('feishu-stop');
      expect(child.stdin.end).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it('says that the assistant did not stop where it is still there after it was ended', async () => {
    vi.useFakeTimers();
    try {
      const shell = await withApp();
      await shell.invoke('feishu-start');
      const [child] = shell.bots;
      const stopped = shell.invoke('feishu-stop') as Promise<void>;
      const refused = expect(stopped).rejects.toThrow('The assistant in Feishu did not end.');
      await vi.advanceTimersByTimeAsync(6_000 + 5_000);
      await refused;
      // It runs still, as far as the shell knows: no other is started beside it.
      await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ own: true });
      await shell.invoke('feishu-start');
      expect(shell.bots).toHaveLength(1);
      // Nor is an app stored or deleted while it may run.
      const forgetting = expect((async () => shell.invoke('feishu-forget'))()).rejects.toThrow('did not end');
      await vi.advanceTimersByTimeAsync(6_000 + 5_000);
      await forgetting;
      expect(shell.setups.filter(run => run.args.includes('--forget'))).toEqual([]);
      ended(child);
    } finally {
      vi.useRealTimers();
    }
  });

  it('starts nothing once the application is closing', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    const stopping = shell.invoke('feishu-stop') as Promise<void>;
    // A start that waits for the stop to end, while the windows are closed meanwhile.
    const starting = shell.invoke('feishu-start') as Promise<void>;
    shell.app.emit('window-all-closed');
    ended(child);
    await stopping;
    await starting;
    await settled();
    expect(shell.bots).toHaveLength(1);
    expect(shell.app.quit).toHaveBeenCalled();
  });

  it('keeps the window for the app open while an app is stored from it', async () => {
    const shell = await withApp();
    shell.setup.answer = () => null;
    void shell.invoke('set-up-feishu', { language: 'en' });
    await Promise.resolve();
    const storing = shell.asFeishuWindow('store', { domain: 'feishu', app_id: 'cli_a1b2c3d4', app_secret: 'made up' }) as Promise<unknown>;
    await settled();
    const closing = { preventDefault: vi.fn() };
    shell.feishuWindow().emit('close', closing);
    expect(closing.preventDefault).toHaveBeenCalledOnce();
    answer(shell.setups.at(-1)!, { stored: { domain: 'feishu', app_id: 'cli_a1b2c3d4' }, secret_in: 'keyring' });
    await storing;
    const later = { preventDefault: vi.fn() };
    shell.feishuWindow().emit('close', later);
    expect(later.preventDefault).not.toHaveBeenCalled();
  });

  it('ends nothing where the assistant ended once its input was closed', async () => {
    vi.useFakeTimers();
    try {
      const shell = await withApp();
      await shell.invoke('feishu-start');
      const [child] = shell.bots;
      const stopped = shell.invoke('feishu-stop') as Promise<void>;
      // Two stops at once wait for the same end.
      const again = shell.invoke('feishu-stop') as Promise<void>;
      ended(child);
      await expect(Promise.all([stopped, again])).resolves.toEqual([undefined, undefined]);
      await vi.advanceTimersByTimeAsync(10_000);
      expect(shell.spawn.mock.calls.some(call => call[0] === 'taskkill.exe')).toBe(false);
      expect(child.stdin.end).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it('deletes the app once the assistant has stopped', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    const forgetting = shell.invoke('feishu-forget') as Promise<unknown>;
    await settled();
    expect(child.stdin.end).toHaveBeenCalledOnce();
    expect(shell.setups.filter(run => run.args.includes('--forget'))).toEqual([]);
    ended(child);
    await expect(forgetting).resolves.toEqual({ forgotten: true });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'feishu', '--forget', '--json']);
    expect(shell.preferences().feishuOn).toBe(false);
  });

  it('says why the assistant could not be started, and lets another start', async () => {
    const shell = await withApp();
    const failed = Object.assign(new EventEmitter(), {
      pid: 0, exitCode: null as number | null, signalCode: null,
      stdout: new EventEmitter(), stderr: Object.assign(new EventEmitter(), { pipe: vi.fn() }),
      stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
    });
    shell.spawn.mockImplementationOnce(() => {
      queueMicrotask(() => failed.emit('error', new Error('spawn ENOENT')));
      return failed;
    });
    await expect((async () => shell.invoke('feishu-start'))()).rejects.toThrow('The assistant in Feishu could not be started: spawn ENOENT');
    failed.emit('close', -2, null);
    expect(toldOfFeishu(shell)).toEqual([]);
    expect(shell.preferences()).toBeNull();
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ own: false, ended: { code: null, said: 'spawn ENOENT' } });
    await shell.invoke('feishu-stop');
    expect(failed.stdin.end).not.toHaveBeenCalled();
    await expect(shell.invoke('feishu-start')).resolves.toBeUndefined();
    // An error of an assistant that started leaves it to end by itself.
    shell.bots[0].emit('error', new Error('kill EPERM'));
    ended(shell.bots[0]);
    expect(toldOfFeishu(shell)).toEqual([{ ended: true, code: 0, said: '' }]);
  });

  it('has the assistant say things by itself in the language of the page that starts it', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start', { language: 'zh-CN' });
    expect(shell.bots[0].args).toEqual(['-m', 'columbina', 'feishu', '--json', '--language', 'zh']);
    // It is kept for the next start of the application.
    expect(shell.preferences().feishuLanguage).toBe('zh');
    ended(shell.bots[0]);
    await shell.invoke('feishu-start', { language: 'en' });
    expect(shell.bots[1].args).toEqual(['-m', 'columbina', 'feishu', '--json', '--language', 'en']);
    // A page in a language that the assistant does not speak leaves it to the platform of the app.
    for (const language of ['ja', 'zh', undefined]) {
      ended(shell.bots.at(-1));
      await shell.invoke('feishu-start', { language });
      expect(shell.bots.at(-1).args).toEqual(['-m', 'columbina', 'feishu', '--json']);
      expect(shell.preferences().feishuLanguage).toBeNull();
    }
    ended(shell.bots.at(-1));
    await shell.invoke('feishu-start');
    expect(shell.bots.at(-1).args).toEqual(['-m', 'columbina', 'feishu', '--json']);
  });

  it('starts the assistant with the application where it ran when the application was left', async () => {
    const shell = await shellHarness(JSON.stringify({ feishuOn: true }), real);
    await settled();
    expect(shell.bots).toHaveLength(1);
    expect(shell.bots[0].args).toEqual(['-m', 'columbina', 'feishu', '--json']);
    // In the language that it spoke when it was started, as far as the assistant speaks it.
    const speaking = await shellHarness(JSON.stringify({ feishuOn: true, feishuLanguage: 'zh' }), real);
    const unknown = await shellHarness(JSON.stringify({ feishuOn: true, feishuLanguage: 'fr' }), real);
    await settled();
    expect(speaking.bots[0].args).toEqual(['-m', 'columbina', 'feishu', '--json', '--language', 'zh']);
    expect(unknown.bots[0].args).toEqual(['-m', 'columbina', 'feishu', '--json']);
    // And not where it did not, nor in a replay.
    const off = await shellHarness(JSON.stringify({ feishuOn: false }), real);
    const replay = await shellHarness(JSON.stringify({ feishuOn: true }));
    await settled();
    expect([...off.bots, ...replay.bots]).toEqual([]);
  });

  it('lets the assistant stop before the application leaves from the tray', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    // Closed, the window is put away in the tray, and the assistant goes on.
    const putAway = vi.fn();
    shell.window.emit('close', { preventDefault: putAway });
    expect(putAway).toHaveBeenCalledOnce();
    expect(shell.window.hide).toHaveBeenCalledOnce();
    expect(child.stdin.end).not.toHaveBeenCalled();
    // Quitting from there closes it as where nothing keeps it: the page is asked to end first.
    const [, , quit] = shell.menu.buildFromTemplate.mock.calls.at(-1)![0] as { click?: () => void }[];
    quit.click!();
    expect(shell.window.close).toHaveBeenCalledOnce();
    const preventDefault = vi.fn();
    shell.window.emit('close', { preventDefault });
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(shell.sent.at(-1)?.[0]).toBe('columbina:shutdown');
    await shell.invoke('stop');
    expect(shell.window.destroy).toHaveBeenCalledOnce();
    expect(child.stdin.end).toHaveBeenCalledOnce();
    expect(shell.app.quit).not.toHaveBeenCalled();
    ended(child);
    await settled();
    expect(shell.app.quit).toHaveBeenCalled();
  });

  it('lets the assistant stop when the windows are all closed', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    shell.app.emit('window-all-closed');
    await settled();
    expect(child.stdin.end).toHaveBeenCalledOnce();
    expect(shell.app.quit).not.toHaveBeenCalled();
    ended(child);
    await settled();
    expect(shell.app.quit).toHaveBeenCalledOnce();
  });

  it('shows the log of the assistant in its folder', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-log');
    expect(shell.system.showItemInFolder).toHaveBeenCalledWith(logOfFeishu);
  });

  it('answers the calls of the assistant to the main window alone', async () => {
    const shell = await withApp();
    for (const name of ['feishu-status', 'feishu-start', 'feishu-stop', 'feishu-forget', 'feishu-log', 'set-up-feishu']) {
      await expect((async () => shell.handlers.get(`columbina:${name}`)!(shell.unrelated))()).rejects.toThrow('IPC sender');
    }
    expect(shell.bots).toEqual([]);
    expect(shell.setups).toEqual([]);
  });

  describe('and the window for its app', () => {
    const opened = async (kept?: string) => {
      const shell = await withApp(kept);
      const closed = shell.invoke('set-up-feishu', { language: 'zh-CN', then: 'more' }) as Promise<unknown>;
      await Promise.resolve();
      return { shell, closed };
    };

    it('is a window of its own under the main one, as closed to the world as the main one', async () => {
      const { shell } = await opened();
      const window = shell.feishuWindow();
      expect(shell.windows).toHaveLength(2);
      expect(window.options).toMatchObject({ parent: shell.window, modal: true, resizable: false });
      expect(window.options.webPreferences).toEqual({
        preload: path.join(path.dirname(mainPath), 'feishu-preload.cjs'),
        contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true,
      });
      expect(window.loadFile).toHaveBeenCalledWith(path.resolve(path.dirname(mainPath), '../dist/feishu.html'));
      expect(window.webContents.mainFrame.url).toBe(feishuPage);
      expect(window.webContents.setWindowOpenHandler.mock.calls[0][0]()).toEqual({ action: 'deny' });
      for (const happening of ['will-navigate', 'will-attach-webview']) {
        const event = { preventDefault: vi.fn() };
        window.webContents.emit(happening, event);
        expect(event.preventDefault).toHaveBeenCalledOnce();
      }
      window.emit('ready-to-show');
      expect(window.show).toHaveBeenCalledOnce();
    });

    it('opens once, and says when it has closed whether an app was stored there', async () => {
      const { shell, closed } = await opened();
      const again = shell.invoke('set-up-feishu', { language: 'en' }) as Promise<unknown>;
      expect(shell.windows).toHaveLength(2);
      expect(shell.feishuWindow().focus).toHaveBeenCalledOnce();
      await shell.asFeishuWindow('close');
      await expect(closed).resolves.toEqual({ changed: false });
      await expect(again).resolves.toEqual({ changed: false });
      // Another one opens once that one has closed.
      const next = shell.invoke('set-up-feishu', { language: 'en' }) as Promise<unknown>;
      await Promise.resolve();
      expect(shell.windows).toHaveLength(3);
      await shell.asFeishuWindow('store', { domain: 'feishu', app_id: 'cli_a1b2c3d4', app_secret: 'made up' });
      await shell.asFeishuWindow('close');
      await expect(next).resolves.toEqual({ changed: true });
    });

    it('tells its page the language, the theme and what is set up', async () => {
      const { shell } = await opened(JSON.stringify({ theme: 'moonwhite' }));
      await expect(shell.asFeishuWindow('context')).resolves.toEqual({ language: 'zh-CN', theme: 'moonwhite', status: app });
      expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'feishu', '--status', '--json']);
      // A language that the shell does not speak is English.
      await shell.asFeishuWindow('close');
      void shell.invoke('set-up-feishu', { language: 'xx' });
      await Promise.resolve();
      await expect(shell.asFeishuWindow('context')).resolves.toMatchObject({ language: 'en' });
    });

    it('hands the program the app on its standard input, and nothing else that came with it', async () => {
      const { shell } = await opened();
      const answer = await shell.asFeishuWindow('store', {
        domain: 'lark', app_id: 'cli_a1b2c3d4', app_secret: 'made up', then: 'more', owner: 'ou_somebody', number: 3,
      });
      expect(answer).toEqual({ stored: { domain: 'feishu', app_id: 'cli_a1b2c3d4' }, secret_in: 'keyring' });
      const run = shell.setups.at(-1)!;
      expect(run.args).toEqual(['-m', 'columbina', 'feishu', '--from-stdin']);
      expect(JSON.parse(run.input!)).toEqual({ domain: 'lark', app_id: 'cli_a1b2c3d4', app_secret: 'made up' });
      // What is not a text is left out, and the program says what is missing.
      await shell.asFeishuWindow('store', { domain: 3, app_id: null, app_secret: { secret: true } });
      expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({});
      // No secret goes onto a command line.
      expect(shell.setups.flatMap(each => each.args).join(' ')).not.toContain('made up');
    });

    it('stops the assistant that runs while the app is stored, and starts it again then', async () => {
      const { shell } = await opened();
      await shell.invoke('feishu-start');
      const [first] = shell.bots;
      const storing = shell.asFeishuWindow('store', { domain: 'feishu', app_id: 'cli_a1b2c3d4', app_secret: 'made up' }) as Promise<unknown>;
      await settled();
      expect(first.stdin.end).toHaveBeenCalledOnce();
      expect(shell.setups.filter(run => run.args.includes('--from-stdin'))).toEqual([]);
      ended(first);
      await expect(storing).resolves.toMatchObject({ secret_in: 'keyring' });
      await settled();
      expect(shell.bots).toHaveLength(2);
      // One that does not run is not started by a store.
      const stopping = shell.invoke('feishu-stop') as Promise<void>;
      ended(shell.bots[1]);
      await stopping;
      await shell.asFeishuWindow('store', { domain: 'feishu', app_id: 'cli_a1b2c3d4', app_secret: 'made up' });
      await settled();
      expect(shell.bots).toHaveLength(2);
    });

    it('opens the consoles of the two platforms, and no other page', async () => {
      const { shell } = await opened();
      await shell.asFeishuWindow('open-page', 'https://open.feishu.cn/app');
      await shell.asFeishuWindow('open-page', 'https://open.larksuite.com/app');
      expect(shell.system.openExternal.mock.calls).toEqual([['https://open.feishu.cn/app'], ['https://open.larksuite.com/app']]);
      for (const address of ['https://open.feishu.cn/app/cli_a1b2c3d4', 'http://open.feishu.cn/app', 'file:///C:/', 7]) {
        await expect((async () => shell.asFeishuWindow('open-page', address))()).rejects.toThrow('not the console');
      }
      expect(shell.system.openExternal).toHaveBeenCalledTimes(2);
    });

    it('answers its calls to it alone', async () => {
      const { shell } = await opened();
      const main = { sender: shell.window.webContents, senderFrame: shell.window.webContents.mainFrame };
      for (const name of ['context', 'store', 'open-page', 'close']) {
        await expect((async () => shell.handlers.get(`columbina-feishu:${name}`)!(main, {}))()).rejects.toThrow('the window for the app of Feishu');
      }
      // And none of the main window, nor of the window for the key.
      const asIt = { sender: shell.feishuWindow().webContents, senderFrame: shell.feishuWindow().webContents.mainFrame };
      await expect((async () => shell.handlers.get('columbina:feishu-start')!(asIt))()).rejects.toThrow('IPC sender');
      await expect((async () => shell.handlers.get('columbina-key:context')!(asIt))()).rejects.toThrow('IPC sender');
      // Nor once it has closed.
      await shell.asFeishuWindow('close');
      const gone = shell.windows[1];
      await expect((async () => shell.handlers.get('columbina-feishu:context')!(
        { sender: gone.webContents, senderFrame: gone.webContents.mainFrame }))()).rejects.toThrow('the window for the app of Feishu');
    });
  });
});


describe('the assistant in WeCom', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const bot = {
    set_up: true, bot_id: 'aibTestBot0123', paired: false, folder: path.resolve('/home/columbina-test/Columbina/wecom'),
    writable: false, running: false, state: null, code: null, error: null,
  };
  const feishuApp = {
    set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', paired: false, running: false, state: null, code: null, error: null,
  };
  const wecomPage = pathToFileURL(path.resolve(path.dirname(mainPath), '../dist/wecom.html')).href;
  const told = (shell: Awaited<ReturnType<typeof shellHarness>>, kind: string) => shell.sent
    .filter(([channel]) => channel === `columbina:${kind}`).map(([, value]) => value);
  const readerOf = (shell: Awaited<ReturnType<typeof shellHarness>>, child: any) =>
    shell.lineReaders.find((reader: any) => reader.input === child.stdout);
  /** The shell with a bot set up, and an app of Feishu, which the program says of whenever it is asked. */
  const withBot = async (kept?: string) => {
    const shell = await shellHarness(kept, real);
    shell.setup.answer = (args) => (args.includes('--status')
      ? { out: `${JSON.stringify(args.includes('wecom') ? bot : feishuApp)}\n`, code: 0 }
      : args.includes('--forget') ? { out: '{"forgotten": true}\n', code: 0 }
        : { out: '{"stored": {"bot_id": "aibTestBot0123"}, "secret_in": "keyring"}\n', code: 0 });
    return shell;
  };
  const ended = (child: any, code = 0) => {
    child.exitCode = code;
    child.emit('close', code, null);
  };

  it('has nothing to say of it in a replay', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('wecom-status')).resolves.toBeNull();
    await expect((async () => shell.invoke('wecom-start'))()).rejects.toThrow('No service program');
    await expect((async () => shell.invoke('set-up-wecom', { language: 'en' }))()).rejects.toThrow('No service program');
    expect(shell.bots).toEqual([]);
  });

  it('asks the program how the assistant is doing, and adds what the shell knows of it', async () => {
    const shell = await withBot();
    await expect(shell.invoke('wecom-status')).resolves.toEqual({ ...bot, own: false, on: false, ended: null });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'wecom', '--status', '--json']);
  });

  it('starts the assistant, tells the page what it says of itself on a channel of its own, and that it ended', async () => {
    const shell = await withBot();
    await shell.invoke('wecom-start', { language: 'zh-CN' });
    const [child] = shell.bots;
    expect(child.args).toEqual(['-m', 'columbina', 'wecom', '--json', '--language', 'zh']);
    expect(child.options.cwd).toBe(path.resolve('/columbina-test/home'));
    expect(shell.preferences()).toMatchObject({ wecomOn: true, wecomLanguage: 'zh', feishuOn: false, feishuLanguage: null });
    await expect(shell.invoke('wecom-status')).resolves.toMatchObject({ own: true, on: true });
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ own: false, on: false });

    readerOf(shell, child).emit('line', '{"state": "connected", "paired": false, "code": "123456", "error": null, "bot_id": "aibTestBot0123"}');
    child.stderr.emit('data', Buffer.from('columbina: WeCom refused the bot\n'));
    ended(child, 1);
    expect(told(shell, 'wecom')).toEqual([
      { state: 'connected', paired: false, code: '123456', error: null, bot_id: 'aibTestBot0123' },
      { ended: true, code: 1, said: 'WeCom refused the bot' },
    ]);
    expect(told(shell, 'feishu')).toEqual([]);
    await expect(shell.invoke('wecom-status')).resolves.toMatchObject({ own: false, ended: { code: 1, said: 'WeCom refused the bot' } });
    // The assistant in Feishu has nothing to say of it.
    await expect(shell.invoke('feishu-status')).resolves.toMatchObject({ ended: null });
  });

  it('runs beside the assistant in Feishu, and each is stopped alone', async () => {
    const shell = await withBot();
    await shell.invoke('feishu-start');
    await shell.invoke('wecom-start');
    expect(shell.bots.map(child => child.args[2])).toEqual(['feishu', 'wecom']);
    await shell.invoke('wecom-start');
    expect(shell.bots).toHaveLength(2);
    const [feishu, wecom] = shell.bots;
    const stopping = shell.invoke('wecom-stop') as Promise<void>;
    expect(wecom.stdin.end).toHaveBeenCalledOnce();
    expect(feishu.stdin.end).not.toHaveBeenCalled();
    ended(wecom);
    await stopping;
    expect(shell.preferences()).toMatchObject({ wecomOn: false, feishuOn: true });
  });

  it('says that the assistant did not stop where it is still there after it was ended', async () => {
    vi.useFakeTimers();
    try {
      const shell = await withBot();
      await shell.invoke('wecom-start');
      const stopping = (shell.invoke('wecom-stop') as Promise<void>).catch((problem: Error) => problem);
      await vi.advanceTimersByTimeAsync(11_000);
      await expect(stopping).resolves.toEqual(new Error('The assistant in WeCom did not end.'));
    } finally {
      vi.useRealTimers();
    }
  });

  it('says why the assistant could not be started', async () => {
    const shell = await withBot();
    const failed = Object.assign(new EventEmitter(), {
      pid: 0, exitCode: null as number | null, signalCode: null,
      stdout: new EventEmitter(), stderr: Object.assign(new EventEmitter(), { pipe: vi.fn() }),
      stdin: Object.assign(new EventEmitter(), { end: vi.fn() }),
    });
    shell.spawn.mockImplementationOnce(() => {
      queueMicrotask(() => failed.emit('error', new Error('spawn ENOENT')));
      return failed;
    });
    await expect((async () => shell.invoke('wecom-start'))()).rejects.toThrow('The assistant in WeCom could not be started: spawn ENOENT');
    await expect(shell.invoke('wecom-status')).resolves.toMatchObject({ own: false, ended: { code: null, said: 'spawn ENOENT' } });
  });

  it('deletes the bot once the assistant has stopped, and changes what its agent may do with the program', async () => {
    const shell = await withBot();
    await shell.invoke('wecom-start');
    const forgetting = shell.invoke('wecom-forget') as Promise<unknown>;
    await settled();
    ended(shell.bots[0]);
    await expect(forgetting).resolves.toEqual({ forgotten: true });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'wecom', '--forget', '--json']);
    expect(shell.preferences().wecomOn).toBe(false);
    await shell.invoke('wecom-writable', true);
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'wecom', '--from-stdin']);
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ writable: true });
    await expect((async () => shell.invoke('wecom-writable', 'yes'))()).rejects.toThrow('true or false');
  });

  it('changes the model of the assistant and how hard it thinks with the program', async () => {
    const shell = await withBot();
    await shell.invoke('wecom-thinking', { model: ' deepseek-v4-pro ', effort: 'high', more: 'never' });
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'wecom', '--from-stdin']);
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ model: 'deepseek-v4-pro', effort: 'high' });
    await shell.invoke('wecom-thinking', {});
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ model: null, effort: null });
    const runs = shell.setups.length;
    for (const asked of [{ model: '  ' }, { model: 7 }, { model: 'x'.repeat(201) }, { effort: 'ultra' }]) {
      await expect((async () => shell.invoke('wecom-thinking', asked))(), JSON.stringify(asked)).rejects.toThrow(/model|thinks/);
    }
    expect(shell.setups).toHaveLength(runs);
    await expect((async () => shell.handlers.get('columbina:wecom-thinking')!(shell.unrelated, {}))()).rejects.toThrow('IPC sender');
  });

  it('shows the log and the folder of the assistant', async () => {
    const shell = await withBot();
    await shell.invoke('wecom-log');
    expect(shell.system.showItemInFolder).toHaveBeenCalledWith(path.resolve('/columbina-test/logs/columbina-wecom.log'));
    await shell.invoke('wecom-folder');
    expect(shell.system.openPath).toHaveBeenCalledWith(bot.folder);
  });

  it('starts the assistant with the application where it ran when the application was left, beside the one in Feishu', async () => {
    const shell = await shellHarness(JSON.stringify({ wecomOn: true, wecomLanguage: 'en' }), real);
    await settled();
    expect(shell.bots.map(child => child.args)).toEqual([['-m', 'columbina', 'wecom', '--json', '--language', 'en']]);
    const both = await shellHarness(JSON.stringify({ feishuOn: true, wecomOn: true, wecomLanguage: 'fr' }), real);
    await settled();
    expect(both.bots.map(child => child.args)).toEqual([['-m', 'columbina', 'feishu', '--json'], ['-m', 'columbina', 'wecom', '--json']]);
    // And so does the one in QQ.
    const qq = await shellHarness(JSON.stringify({ qqOn: true, qqLanguage: 'zh' }), real);
    await settled();
    expect(qq.bots.map(child => child.args)).toEqual([['-m', 'columbina', 'qq', '--json', '--language', 'zh']]);
  });

  it('answers no question of whether the agents of the assistant in QQ may change files', async () => {
    const shell = await shellHarness();
    expect(shell.handlers.has('columbina:qq-writable')).toBe(false);
    expect(shell.handlers.has('columbina:wecom-writable')).toBe(true);
  });

  it('keeps the window in the tray while it runs, and lets it stop before the application leaves', async () => {
    const shell = await withBot();
    await shell.invoke('wecom-start');
    const [child] = shell.bots;
    const putAway = vi.fn();
    shell.window.emit('close', { preventDefault: putAway });
    expect(putAway).toHaveBeenCalledOnce();
    expect(shell.window.hide).toHaveBeenCalledOnce();
    const [, , quit] = shell.menu.buildFromTemplate.mock.calls.at(-1)![0] as { click?: () => void }[];
    quit.click!();
    shell.window.emit('close', { preventDefault: vi.fn() });
    await shell.invoke('stop');
    expect(child.stdin.end).toHaveBeenCalledOnce();
    expect(shell.app.quit).not.toHaveBeenCalled();
    ended(child);
    await settled();
    expect(shell.app.quit).toHaveBeenCalled();
  });

  it('waits for every assistant before the application quits', async () => {
    const shell = await withBot();
    await shell.invoke('feishu-start');
    await shell.invoke('wecom-start');
    const [feishu, wecom] = shell.bots;
    const leaving = { preventDefault: vi.fn() };
    shell.app.emit('before-quit', leaving);
    expect(leaving.preventDefault).toHaveBeenCalledOnce();
    shell.app.emit('window-all-closed');
    await settled();
    expect(feishu.stdin.end).toHaveBeenCalledOnce();
    expect(wecom.stdin.end).toHaveBeenCalledOnce();
    ended(feishu);
    await settled();
    expect(shell.app.quit).not.toHaveBeenCalled();
    ended(wecom);
    await settled();
    expect(shell.app.quit).toHaveBeenCalledOnce();
  });

  it('answers the calls of the assistant to the main window alone', async () => {
    const shell = await withBot();
    for (const name of ['wecom-status', 'wecom-start', 'wecom-stop', 'wecom-forget', 'wecom-log', 'wecom-writable', 'wecom-folder', 'set-up-wecom']) {
      await expect((async () => shell.handlers.get(`columbina:${name}`)!(shell.unrelated))()).rejects.toThrow('IPC sender');
    }
    expect(shell.bots).toEqual([]);
  });

  describe('and the window for its bot', () => {
    const opened = async (kept?: string) => {
      const shell = await withBot(kept);
      const closed = shell.invoke('set-up-wecom', { language: 'ja' }) as Promise<unknown>;
      await Promise.resolve();
      return { shell, closed };
    };

    it('is a window of its own under the main one, with a page and a bridge of its own', async () => {
      const { shell } = await opened();
      const window = shell.wecomWindow();
      expect(window.options).toMatchObject({ parent: shell.window, modal: true, resizable: false });
      expect(window.options.webPreferences).toEqual({
        preload: path.join(path.dirname(mainPath), 'wecom-preload.cjs'),
        contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true,
      });
      expect(window.loadFile).toHaveBeenCalledWith(path.resolve(path.dirname(mainPath), '../dist/wecom.html'));
      expect(window.webContents.mainFrame.url).toBe(wecomPage);
      expect(shell.feishuWindow()).toBeUndefined();
    });

    it('tells its page the language, the theme and the bot, and says when it has closed whether a bot was stored there', async () => {
      const { shell, closed } = await opened(JSON.stringify({ theme: 'moonwhite' }));
      await expect(shell.asWecomWindow('context')).resolves.toEqual({ language: 'ja', theme: 'moonwhite', status: bot });
      expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'wecom', '--status', '--json']);
      await shell.asWecomWindow('store', { bot_id: 'aibTestBot0123', secret: 'made up' });
      await shell.asWecomWindow('close');
      await expect(closed).resolves.toEqual({ changed: true });
    });

    it('hands the program the bot on its standard input, and nothing else that came with it', async () => {
      const { shell } = await opened();
      const answer = await shell.asWecomWindow('store', {
        bot_id: 'aibTestBot0123', secret: 'made up', domain: 'lark', app_id: 'cli_a1b2c3d4', app_secret: 'other', writable: true,
      });
      expect(answer).toEqual({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'keyring' });
      const run = shell.setups.at(-1)!;
      expect(run.args).toEqual(['-m', 'columbina', 'wecom', '--from-stdin']);
      expect(JSON.parse(run.input!)).toEqual({ bot_id: 'aibTestBot0123', secret: 'made up' });
      expect(shell.setups.flatMap(each => each.args).join(' ')).not.toContain('made up');
    });

    it('stops the assistant that runs while the bot is stored, and starts it again then', async () => {
      const { shell } = await opened();
      await shell.invoke('wecom-start');
      const storing = shell.asWecomWindow('store', { bot_id: 'aibTestBot0123', secret: 'made up' }) as Promise<unknown>;
      await settled();
      expect(shell.bots[0].stdin.end).toHaveBeenCalledOnce();
      ended(shell.bots[0]);
      await storing;
      await settled();
      expect(shell.bots.map(child => child.args[2])).toEqual(['wecom', 'wecom']);
    });

    it('opens the admin console of WeCom, and no other page', async () => {
      const { shell } = await opened();
      await shell.asWecomWindow('open-page', 'https://work.weixin.qq.com/wework_admin/frame');
      expect(shell.system.openExternal.mock.calls).toEqual([['https://work.weixin.qq.com/wework_admin/frame']]);
      for (const address of ['https://open.feishu.cn/app', 'https://work.weixin.qq.com/', 'http://work.weixin.qq.com/wework_admin/frame', 7]) {
        await expect((async () => shell.asWecomWindow('open-page', address))()).rejects.toThrow('not the admin console of WeCom');
      }
    });

    it('answers its calls to it alone, and the window for the app of Feishu answers none of them', async () => {
      const { shell } = await opened();
      const main = { sender: shell.window.webContents, senderFrame: shell.window.webContents.mainFrame };
      for (const name of ['context', 'store', 'open-page', 'close']) {
        await expect((async () => shell.handlers.get(`columbina-wecom:${name}`)!(main, {}))()).rejects.toThrow('the window for the bot of WeCom');
      }
      const asIt = { sender: shell.wecomWindow().webContents, senderFrame: shell.wecomWindow().webContents.mainFrame };
      for (const name of ['columbina-feishu:context', 'columbina-feishu:store', 'columbina:wecom-start']) {
        await expect((async () => shell.handlers.get(name)!(asIt, {}))()).rejects.toThrow(/IPC sender/);
      }
      await shell.asWecomWindow('close');
      // Once it has closed, the window for the app of Feishu opens beside nothing.
      void shell.invoke('set-up-feishu', { language: 'en' });
      await Promise.resolve();
      const feishu = { sender: shell.feishuWindow().webContents, senderFrame: shell.feishuWindow().webContents.mainFrame };
      await expect((async () => shell.handlers.get('columbina-wecom:context')!(feishu))()).rejects.toThrow('the window for the bot of WeCom');
    });
  });
});

describe('the tray, the start with the system, and one application at a time', () => {
  const real = { COLUMBINA_SERVICE_EXECUTABLE: python };
  const app = {
    set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: true,
    running: false, state: null, code: null, error: null, folder: path.resolve('/home/columbina-test/Columbina/feishu'), writable: false,
  };
  /** The shell with an app set up, which the program says of whenever it is asked. */
  const withApp = async (system: System = {}, said: Record<string, unknown> = app) => {
    const shell = await shellHarness(undefined, real, system);
    shell.setup.answer = (args) => (args.includes('--status')
      ? { out: `${JSON.stringify(said)}\n`, code: 0 }
      : { out: `${JSON.stringify({ writable: true })}\n`, code: 0 });
    return shell;
  };
  /** The words of the menu of the tray that the shell made last. */
  const menuOf = (shell: Awaited<ReturnType<typeof shellHarness>>) =>
    (shell.menu.buildFromTemplate.mock.calls.at(-1)![0] as { label?: string; type?: string }[]).map(item => item.label ?? item.type);

  it('puts the window away in the tray while the assistant runs, and brings it back from there', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const putAway = vi.fn();
    shell.window.emit('close', { preventDefault: putAway });
    expect(putAway).toHaveBeenCalledOnce();
    expect(shell.window.hide).toHaveBeenCalledOnce();
    // What plays in the page is not heard from the tray.
    expect(shell.window.webContents.setAudioMuted).toHaveBeenLastCalledWith(true);
    const [tray] = shell.trays;
    expect(tray.image).toBe(path.join(path.dirname(mainPath), 'icon.ico'));
    expect(tray.setToolTip).toHaveBeenCalledWith('Columbina');
    expect(menuOf(shell)).toEqual(['Open Columbina', 'separator', 'Quit Columbina']);
    // A click on the icon, or on what the menu opens with, brings the window back.
    shell.window.minimized = true;
    tray.emit('click');
    expect(shell.window.restore).toHaveBeenCalledOnce();
    expect(shell.window.show).toHaveBeenCalledOnce();
    expect(shell.window.focus).toHaveBeenCalled();
    expect(shell.window.webContents.setAudioMuted).toHaveBeenLastCalledWith(false);
    const [open] = shell.menu.buildFromTemplate.mock.calls.at(-1)![0] as { click?: () => void }[];
    open.click!();
    expect(shell.window.show).toHaveBeenCalledTimes(2);
    // Put away again, it has the same icon.
    shell.window.emit('close', { preventDefault: vi.fn() });
    expect(shell.trays).toHaveLength(1);
    expect(shell.window.hide).toHaveBeenCalledTimes(2);
  });

  it('closes as it did where the assistant does not run', async () => {
    const shell = await withApp();
    const preventDefault = vi.fn();
    shell.window.emit('close', { preventDefault });
    expect(preventDefault).not.toHaveBeenCalled();
    expect(shell.trays).toEqual([]);
    // Nor once the application is closing.
    await shell.invoke('feishu-start');
    shell.app.emit('before-quit', { preventDefault: vi.fn() });
    const later = vi.fn();
    shell.window.emit('close', { preventDefault: later });
    expect(later).not.toHaveBeenCalled();
    expect(shell.trays).toEqual([]);
  });

  it('draws the icon of the tray from the picture of the application on macOS', async () => {
    const shell = await withApp({ platform: 'darwin' });
    await shell.invoke('feishu-start');
    shell.window.emit('close', { preventDefault: vi.fn() });
    expect(shell.pictures.createFromPath).toHaveBeenCalledWith(path.resolve(path.dirname(mainPath), '../dist/columbina-icon.png'));
    expect(shell.trays[0].image).toEqual({ file: path.resolve(path.dirname(mainPath), '../dist/columbina-icon.png'), size: { width: 16, height: 16 } });
    // A click on its icon in the Dock brings the window back.
    shell.app.emit('activate');
    expect(shell.window.show).toHaveBeenCalledOnce();
  });

  it('speaks in the language of the page in the tray, and of the system before the page has said it', async () => {
    const chinese = await withApp({ locale: 'zh-CN' });
    await chinese.invoke('feishu-start');
    chinese.window.emit('close', { preventDefault: vi.fn() });
    expect(menuOf(chinese)).toEqual(['打开 Columbina', 'separator', '退出 Columbina']);
    await chinese.invoke('set-language', 'ja');
    expect(menuOf(chinese)).toEqual(['Columbina を開く', 'separator', 'Columbina を終了']);
    expect(chinese.preferences().language).toBe('ja');
    await expect((async () => chinese.invoke('set-language', 'fr'))()).rejects.toThrow('Unknown language of the page');
    // What the page said last holds at the next start too, whatever the system speaks.
    const kept = await shellHarness(JSON.stringify({ language: 'zh-CN' }), real, { locale: 'ja-JP' });
    kept.setup.answer = () => ({ out: `${JSON.stringify(app)}\n`, code: 0 });
    await kept.invoke('feishu-start');
    kept.window.emit('close', { preventDefault: vi.fn() });
    expect(menuOf(kept)).toEqual(['打开 Columbina', 'separator', '退出 Columbina']);
    // A language that the shell does not speak is not kept, and the page has not changed it before the tray.
    const odd = await shellHarness(JSON.stringify({ language: 'fr' }), real, { locale: 'ja-JP' });
    odd.setup.answer = () => ({ out: `${JSON.stringify(app)}\n`, code: 0 });
    await odd.invoke('feishu-start');
    odd.window.emit('close', { preventDefault: vi.fn() });
    expect(menuOf(odd)).toEqual(['Columbina を開く', 'separator', 'Columbina を終了']);
    const writes = () => odd.writeFileSync.mock.calls.filter(([file]) => String(file).startsWith(preferencesFile)).length;
    const before = writes();
    await odd.invoke('set-language', 'en');
    expect(odd.preferences().language).toBe('en');
    // The same language again is not written down again.
    await odd.invoke('set-language', 'en');
    expect(writes()).toBe(before + 1);
  });

  it('starts at login in the tray, with its window put away', async () => {
    const shell = await withApp({ argv: ['--hidden'] });
    expect(shell.window.options.show).toBe(false);
    expect(shell.trays).toHaveLength(1);
    expect(shell.window.webContents.setAudioMuted).toHaveBeenLastCalledWith(true);
    // Started otherwise, it shows its window.
    const shown = await withApp();
    expect(shown.window.options.show).toBe(true);
    expect(shown.trays).toEqual([]);
  });

  it('shows its window where it was started again while it started at login', async () => {
    const shell = await withApp({ argv: ['--hidden'], beforeReady: (started) => started.emit('second-instance') });
    expect(shell.window.options.show).toBe(false);
    expect(shell.window.show).toHaveBeenCalledOnce();
    expect(shell.window.webContents.setAudioMuted).toHaveBeenLastCalledWith(false);
    expect(shell.trays).toEqual([]);
  });

  it('shows the window of the application that runs where it is started again, and the second ends', async () => {
    const first = await withApp();
    first.app.emit('second-instance');
    expect(first.window.show).toHaveBeenCalledOnce();
    expect(first.app.quit).not.toHaveBeenCalled();
    const second = await shellHarness(undefined, real, { alone: false });
    await settled();
    expect(second.app.quit).toHaveBeenCalledOnce();
    expect(second.windows).toEqual([]);
  });

  it('stops what runs where Windows ends the session', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-start');
    const [child] = shell.bots;
    shell.window.emit('session-end');
    await settled();
    expect(child.stdin.end).toHaveBeenCalledOnce();
    child.exitCode = 0;
    child.emit('close', 0, null);
    await settled();
    expect(shell.app.quit).toHaveBeenCalled();
  });

  it('starts with the system in the tray where it is a package on Windows', async () => {
    const shell = await withApp({ packaged: true });
    expect(shell.invoke('login-item')).toEqual({ offered: true, on: false });
    expect(shell.invoke('set-login-item', true)).toEqual({ offered: true, on: true });
    expect(shell.app.setLoginItemSettings).toHaveBeenLastCalledWith({ openAtLogin: true, path: 'Columbina.exe', args: ['--hidden'] });
    expect(shell.app.getLoginItemSettings).toHaveBeenLastCalledWith({ path: 'Columbina.exe', args: ['--hidden'] });
    expect(shell.invoke('set-login-item', false)).toEqual({ offered: true, on: false });
    await expect((async () => shell.invoke('set-login-item', 'yes'))()).rejects.toThrow('on or off');
    // Not from the checkout, nor on another system.
    for (const system of [{}, { packaged: true, platform: 'darwin' as const }]) {
      const other = await withApp(system);
      expect(other.invoke('login-item')).toEqual({ offered: false, on: false });
      await expect((async () => other.invoke('set-login-item', true))()).rejects.toThrow('does not start with the system');
      expect(other.app.setLoginItemSettings).not.toHaveBeenCalled();
    }
  });

  it('says whether the assistant may change its files, stopping it first and starting it again', async () => {
    const shell = await withApp();
    await expect(shell.invoke('feishu-writable', true)).resolves.toEqual({ writable: true });
    const asked = shell.setups.at(-1)!;
    expect(asked.args).toEqual(['-m', 'columbina', 'feishu', '--from-stdin']);
    expect(JSON.parse(asked.input!)).toEqual({ writable: true });
    expect(shell.bots).toEqual([]);
    // One that runs is stopped first, and started again after.
    await shell.invoke('feishu-start');
    const [running] = shell.bots;
    const changing = shell.invoke('feishu-writable', false);
    await settled();
    expect(running.stdin.end).toHaveBeenCalledOnce();
    running.exitCode = 0;
    running.emit('close', 0, null);
    await expect(changing).resolves.toEqual({ writable: true });
    expect(JSON.parse(shell.setups.at(-1)!.input!)).toEqual({ writable: false });
    expect(shell.bots).toHaveLength(2);
    await expect((async () => shell.invoke('feishu-writable', 'yes'))()).rejects.toThrow('true or false');
  });

  it('starts the assistant again even where what was asked did not go through', async () => {
    const shell = await withApp();
    shell.setup.answer = (args) => (args.includes('--from-stdin') ? { out: '', code: 1 } : { out: `${JSON.stringify(app)}\n`, code: 0 });
    await shell.invoke('feishu-start');
    const [running] = shell.bots;
    const changing = shell.invoke('feishu-writable', true);
    await settled();
    running.exitCode = 0;
    running.emit('close', 0, null);
    await expect(changing).rejects.toThrow();
    expect(shell.bots).toHaveLength(2);
  });

  it('opens the folder of the assistant that the program names, and no other', async () => {
    const shell = await withApp();
    await shell.invoke('feishu-folder');
    expect(shell.system.openPath).toHaveBeenCalledWith(app.folder);
    expect(shell.setups.at(-1)!.args).toEqual(['-m', 'columbina', 'feishu', '--status', '--json']);
    shell.system.openPath.mockResolvedValueOnce('No application opens it.');
    await expect((async () => shell.invoke('feishu-folder'))()).rejects.toThrow('No application opens it.');
    for (const folder of [undefined, 'relative/folder']) {
      const odd = await withApp({}, { ...app, folder });
      await expect((async () => odd.invoke('feishu-folder'))()).rejects.toThrow('named no folder');
      expect(odd.system.openPath).not.toHaveBeenCalled();
    }
    const replay = await shellHarness();
    await expect((async () => replay.invoke('feishu-folder'))()).rejects.toThrow('No service program');
  });

  it('answers these calls to the main window alone', async () => {
    const shell = await withApp({ packaged: true });
    for (const [name, given] of [['set-language', 'en'], ['login-item'], ['set-login-item', true], ['feishu-writable', true], ['feishu-folder']] as [string, unknown?][]) {
      await expect((async () => shell.handlers.get(`columbina:${name}`)!(shell.unrelated, given))()).rejects.toThrow('IPC sender');
    }
    expect(shell.app.setLoginItemSettings).not.toHaveBeenCalled();
    expect(shell.setups).toEqual([]);
  });
});


describe('the pet', () => {
  const userData = path.resolve('/columbina-test/userData');
  const stateFile = path.join(userData, 'pet-state.json');
  const builtIn = path.resolve(path.dirname(mainPath), '../dist/pets');
  const petPage = path.resolve(path.dirname(mainPath), '../dist/pet.html');
  const found = (key: string, origin = 'builtin') => ({
    key, id: key.split('/')[1], name: `The ${key}`, description: 'A pet.', origin, sheet: `/sheets/${key}.png`, animations: null,
  });
  const told = (state: string, label = '', pid = 4242) => JSON.stringify({ version: 1, state, label, at: 1, pid });

  it('shows where it showed at the last start, where it stood then, over the other windows', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true, petAt: { x: 5, y: 6 } }));
    const pet = shell.petWindow();
    expect(pet.options).toMatchObject({
      width: 220, height: 240, x: 5, y: 6, transparent: true, frame: false, alwaysOnTop: true, skipTaskbar: true, show: false,
      webPreferences: { preload: path.join(path.dirname(mainPath), 'pet-preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true },
    });
    expect(pet.loadFile).toHaveBeenCalledWith(petPage);
    pet.emit('ready-to-show');
    expect(pet.showInactive).toHaveBeenCalledTimes(1);
    // It is never focused by showing, and it opens no other page.
    expect(pet.show).not.toHaveBeenCalled();
    expect(pet.webContents.setWindowOpenHandler).toHaveBeenCalled();
    expect((await shellHarness()).petWindow()).toBeUndefined();
  });

  it('keeps what was chosen of it as far as it can be, and nothing else', async () => {
    const kept = await shellHarness(JSON.stringify({ petOn: true, pet: 'user/mine', petAt: { x: -40, y: 7 } }));
    kept.invoke('set-theme', 'moonwhite');
    expect(kept.preferences()).toMatchObject({ petOn: true, pet: 'user/mine', petAt: { x: -40, y: 7 } });
    const refused = await shellHarness(JSON.stringify({ petOn: 'yes', pet: 'builtin/../../secret', petAt: { x: 1.5, y: 2 } }));
    refused.invoke('set-theme', 'moonwhite');
    expect(refused.preferences()).toMatchObject({ petOn: false, pet: 'builtin/moon', petAt: null });
    for (const pet of ['elsewhere/moon', 'user/', 'builtin/a\\b', 7]) {
      const shell = await shellHarness(JSON.stringify({ pet }));
      shell.invoke('set-theme', 'moonwhite');
      expect(shell.preferences().pet, String(pet)).toBe('builtin/moon');
    }
  });

  it('tells the main window the pets that can be chosen, from the application and the user, and which one is', async () => {
    const shell = await shellHarness(JSON.stringify({ pet: 'user/mochi' }));
    shell.petsFound.push(found('builtin/moon'), found('user/mochi', 'user'));
    expect(await shell.invoke('pets')).toEqual({
      on: false, chosen: 'user/mochi',
      pets: [
        { key: 'builtin/moon', name: 'The builtin/moon', description: 'A pet.', origin: 'builtin' },
        { key: 'user/mochi', name: 'The user/mochi', description: 'A pet.', origin: 'user' },
      ],
      folder: path.join(userData, 'pets'), stateFile,
    });
    expect(shell.pets.listPets).toHaveBeenLastCalledWith([{ folder: builtIn, origin: 'builtin' }, { folder: path.join(userData, 'pets'), origin: 'user' }]);
    // The one that was chosen is gone: the first one stands in for it, and none where there is none.
    shell.petsFound.splice(1);
    expect((await shell.invoke('pets')).chosen).toBe('builtin/moon');
    shell.petsFound.splice(0);
    expect((await shell.invoke('pets')).chosen).toBeNull();
  });

  it('shows and hides as it is asked, one at a time, and keeps that for the next start', async () => {
    const shell = await shellHarness();
    shell.petsFound.push(found('builtin/moon'), found('builtin/cat'));
    await shell.invoke('set-pet', { on: true });
    await shell.invoke('set-pet', { on: true });
    expect(shell.windows.filter(each => String(each.options?.webPreferences?.preload).endsWith('pet-preload.cjs'))).toHaveLength(1);
    expect(shell.preferences().petOn).toBe(true);
    const pet = shell.petWindow();
    await shell.invoke('set-pet', { chosen: 'builtin/cat' });
    expect(shell.preferences().pet).toBe('builtin/cat');
    expect(shell.sent.at(-1)).toEqual(['columbina-pet:sheet', { sheet: 'data:image/png;base64,builtin/cat', animations: null, name: 'The builtin/cat' }]);
    // A pet that is not there is not chosen.
    await shell.invoke('set-pet', { chosen: 'builtin/dog' });
    expect(shell.preferences().pet).toBe('builtin/cat');
    await shell.invoke('set-pet', { on: false });
    expect(pet.destroy).toHaveBeenCalledTimes(1);
    expect(shell.preferences().petOn).toBe(false);
    await shell.invoke('set-pet', {});
    expect(pet.destroy).toHaveBeenCalledTimes(1);
  });

  it('has the service process write what its task does to the file that the pet reads', async () => {
    const shell = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python });
    const work = path.resolve('/work');
    await shell.select(work);
    await shell.invoke('start', { cwd: work });
    expect(shell.children[0]).toBeDefined();
    const started = shell.spawn.mock.calls.find(([, args]) => (args as string[]).includes('serve'))!;
    expect(started[2].env.COLUMBINA_PET_STATE).toBe(stateFile);
  });

  it('looks at the task while it shows, and tells its window what changed, in the words of the page', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true, language: 'zh-CN' }));
    const shown = () => shell.sent.filter(([channel]) => channel === 'columbina-pet:shown').map(([, value]) => value);
    // Nothing was told yet: the task rests, as the pet does already.
    expect(shown()).toEqual([]);
    shell.files.set(stateFile, told('thinking'));
    shell.again(1000);
    shell.again(1000);
    expect(shown()).toEqual([{ state: 'thinking', label: '', bubble: '' }]);
    shell.files.set(stateFile, told('waiting', 'powershell'));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'waiting', label: 'powershell', bubble: '等你点头：powershell。' });
    shell.files.set(stateFile, told('waiting'));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'waiting', label: '', bubble: '等你点头。' });
    shell.files.set(stateFile, told('failed', 'AUTH'));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'failed', label: 'AUTH', bubble: '遇到点难题。' });
    // Done is said where the window is not looked at.
    shell.files.set(stateFile, told('done'));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'done', label: '', bubble: '' });
    shell.window.focused = false;
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'done', label: '', bubble: '做完了。' });
    // A file that is half written changes nothing, and one of a process that is gone is at rest.
    const before = shown().length;
    shell.files.set(stateFile, '{"version": 1, "sta');
    shell.again(1000);
    expect(shown()).toHaveLength(before);
    shell.files.set(stateFile, told('review', 'read', 99));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'idle', label: '', bubble: '' });
    shell.running.add(99);
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'review', label: 'read', bubble: '' });
    // Another tool, and the same bubble: the pet is told.
    shell.files.set(stateFile, told('review', 'grep', 99));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'review', label: 'grep', bubble: '' });
    // A process of somebody else is there, as the system says of it.
    shell.files.set(stateFile, told('thinking', '', 1));
    shell.again(1000);
    expect(shown().at(-1)).toEqual({ state: 'thinking', label: '', bubble: '' });
    // Its window is given what was told last when it is ready.
    expect(await shell.asPetWindow('ready')).toMatchObject({ shown: { state: 'thinking', label: '', bubble: '' } });
  });

  it('says what the task does in English and in Japanese too', async () => {
    for (const [language, bubble] of [['en', 'Waiting for your yes to shell.'], ['ja', 'shell の承認を待っています。']] as const) {
      const shell = await shellHarness(JSON.stringify({ petOn: true, language }));
      shell.files.set(stateFile, told('waiting', 'shell'));
      shell.again(1000);
      expect(shell.sent.at(-1), language).toEqual(['columbina-pet:shown', { state: 'waiting', label: 'shell', bubble }]);
    }
    const words = { en: ['Waiting for your yes.', 'Something went wrong.', 'Done.'], ja: ['承認を待っています。', 'うまくいきませんでした。', '終わりました。'] };
    for (const [language, [waiting, failed, done]] of Object.entries(words)) {
      const shell = await shellHarness(JSON.stringify({ petOn: true, language }));
      shell.window.focused = false;
      for (const [state, bubble] of [['waiting', waiting], ['failed', failed], ['done', done]]) {
        shell.files.set(stateFile, told(state));
        shell.again(1000);
        expect(shell.sent.at(-1), `${language} ${state}`).toEqual(['columbina-pet:shown', { state, label: '', bubble }]);
      }
    }
  });

  it('looks at the task at once when it shows', async () => {
    const shell = await shellHarness();
    shell.files.set(stateFile, told('thinking'));
    await shell.invoke('set-pet', { on: true });
    expect(shell.sent.at(-1)).toEqual(['columbina-pet:shown', { state: 'thinking', label: '', bubble: '' }]);
  });

  it('stops looking at the task when it is hidden, and begins again when it shows', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    expect([...shell.repeated.values()].map(one => one.ms)).toEqual([1000]);
    await shell.invoke('set-pet', { on: false });
    expect(shell.repeated.size).toBe(0);
    await shell.invoke('set-pet', { on: true });
    expect(shell.repeated.size).toBe(1);
    shell.petWindow().emit('closed');
    expect(shell.repeated.size).toBe(0);
  });

  it('gives its window the picture of the pet that is chosen, and nothing where it cannot', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    expect(await shell.asPetWindow('ready')).toEqual({ pet: null, shown: { state: 'idle', label: '', bubble: '' } });
    shell.petsFound.push(found('builtin/moon'));
    expect((await shell.asPetWindow('ready')).pet).toEqual({ sheet: 'data:image/png;base64,builtin/moon', animations: null, name: 'The builtin/moon' });
    shell.pets.sheetOf.mockImplementationOnce(() => { throw new Error('the sheet is too large'); });
    expect((await shell.asPetWindow('ready')).pet).toBeNull();
  });

  it('answers its own window only, and the main window only what is the main window', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    for (const name of ['ready', 'click', 'drag', 'menu']) {
      expect(() => shell.handlers.get(`columbina-pet:${name}`)!(shell.event), name).toThrow('IPC sender is not the window of the pet.');
    }
    const pet = shell.petWindow();
    const asPet = { sender: pet.webContents, senderFrame: pet.webContents.mainFrame };
    for (const name of ['pets', 'set-pet', 'open-pets-folder']) {
      await expect(Promise.resolve().then(() => shell.handlers.get(`columbina:${name}`)!(asPet)), name).rejects.toThrow('IPC sender is not the application window.');
    }
    // A frame of the window that went to another page is not the pet.
    pet.webContents.mainFrame.url = 'file:///elsewhere/pet.html';
    expect(() => shell.handlers.get('columbina-pet:ready')!(asPet)).toThrow('IPC sender is not the window of the pet.');
    pet.webContents.mainFrame.url = pathToFileURL(petPage).href;
    expect(shell.handlers.get('columbina-pet:ready')!(asPet)).toMatchObject({ pet: null });
    // Once it is gone, nothing answers as its window.
    await shell.invoke('set-pet', { on: false });
    expect(() => shell.handlers.get('columbina-pet:ready')!(asPet)).toThrow('IPC sender is not the window of the pet.');
  });

  it('shows the main window at a click', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    shell.window.minimized = true;
    await shell.asPetWindow('click');
    expect(shell.window.restore).toHaveBeenCalled();
    expect(shell.window.show).toHaveBeenCalled();
    expect(shell.window.focus).toHaveBeenCalled();
  });

  it('is moved with the pointer, and keeps where it was put', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    const pet = shell.petWindow();
    pet.position = [300, 400];
    // A drag that has not begun moves nothing.
    await shell.asPetWindow('drag', 'move');
    expect(pet.setPosition).not.toHaveBeenCalled();
    Object.assign(shell.pointer, { x: 1000, y: 500 });
    await shell.asPetWindow('drag', 'start');
    Object.assign(shell.pointer, { x: 1010, y: 480 });
    await shell.asPetWindow('drag', 'move');
    expect(pet.setPosition).toHaveBeenLastCalledWith(310, 380);
    expect(shell.preferences()?.petAt ?? null).toBeNull();
    await shell.asPetWindow('drag', 'sideways');
    expect(pet.setPosition).toHaveBeenCalledTimes(1);
    Object.assign(shell.pointer, { x: 900, y: 520 });
    await shell.asPetWindow('drag', 'end');
    expect(pet.setPosition).toHaveBeenLastCalledWith(200, 420);
    expect(shell.preferences().petAt).toEqual({ x: 200, y: 420 });
    // The drag is over.
    await shell.asPetWindow('drag', 'move');
    expect(pet.setPosition).toHaveBeenCalledTimes(2);
  });

  it('opens a menu that shows Columbina or hides the pet, in the language of the page', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true, language: 'zh-CN' }));
    const pet = shell.petWindow();
    await shell.asPetWindow('menu');
    const built = shell.menu.buildFromTemplate.mock.results.at(-1)!.value;
    expect(built.template.map((item: { label: string }) => item.label)).toEqual(['打开 Columbina', '收起桌宠']);
    expect(built.popup).toHaveBeenCalledWith({ window: pet });
    built.template[0].click();
    expect(shell.window.show).toHaveBeenCalled();
    built.template[1].click();
    expect(pet.destroy).toHaveBeenCalledTimes(1);
    expect(shell.preferences().petOn).toBe(false);
    expect(shell.sent.at(-1)).toEqual(['columbina:pet', { on: false }]);
    for (const [language, words] of [['en', ['Open Columbina', 'Hide the pet']], ['ja', ['Columbina を開く', 'ペットをしまう']]] as const) {
      const other = await shellHarness(JSON.stringify({ petOn: true, language }));
      await other.asPetWindow('menu');
      expect(other.menu.buildFromTemplate.mock.results.at(-1)!.value.template.map((item: { label: string }) => item.label)).toEqual(words);
    }
  });

  it('goes with the main window', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    const pet = shell.petWindow();
    shell.window.emit('closed');
    expect(pet.destroy).toHaveBeenCalledTimes(1);
    expect(shell.repeated.size).toBe(0);
  });

  it('keeps the window that it opened last, where one that was put away says late that it closed', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    const first = shell.petWindow();
    await shell.invoke('set-pet', { on: false });
    await shell.invoke('set-pet', { on: true });
    first.emit('closed');
    expect(await shell.asPetWindow('ready')).toMatchObject({ shown: { state: 'idle' } });
    expect(shell.repeated.size).toBe(1);
  });

  it('forgets its window where the window closed by itself, and makes another when it is asked', async () => {
    const shell = await shellHarness(JSON.stringify({ petOn: true }));
    const pet = shell.petWindow();
    pet.emit('closed');
    await shell.invoke('set-pet', { on: true });
    expect(shell.windows.filter(each => String(each.options?.webPreferences?.preload).endsWith('pet-preload.cjs'))).toHaveLength(2);
    const again = shell.petWindow();
    // An old window that closes late takes nothing of the new one.
    pet.emit('closed');
    again.isDestroyed = () => true;
    shell.window.emit('closed');
    expect(again.destroy).not.toHaveBeenCalled();
    expect(pet.destroy).not.toHaveBeenCalled();
  });

  it('opens the folder for pets of the user, made where it is not there', async () => {
    const shell = await shellHarness();
    await shell.invoke('open-pets-folder');
    expect(shell.mkdirSync).toHaveBeenCalledWith(path.join(userData, 'pets'), { recursive: true });
    expect(shell.system.openPath).toHaveBeenCalledWith(path.join(userData, 'pets'));
    shell.system.openPath.mockResolvedValueOnce('No application opens folders.');
    await expect(shell.invoke('open-pets-folder')).rejects.toThrow('No application opens folders.');
  });
});

describe('the engine of the studio', () => {
  /** The project of the studio in a folder, and the engine that development finds in the repository. */
  const studioOf = (cwd: string) => path.join(cwd, '.columbina', 'studio');
  const builtEngine = (name = 'columbina-engine.exe') => path.resolve(path.dirname(mainPath), '../../engine/target/release', name);
  /** What the page was told of the engine, in order. */
  const told = (shell: { sent: [string, unknown][] }) => shell.sent.filter(([channel]) => channel === 'columbina:engine-event').map(([, value]) => value as any);
  // Turns of the loop, not of the clock: a timer of Windows waits some 15 ms.
  const settledAll = async () => { for (let at = 0; at < 20; at += 1) await new Promise(resolve => setImmediate(resolve)); };
  type Shell = Awaited<ReturnType<typeof shellHarness>>;
  /** Choose a folder in the dialog and open it in the studio. */
  async function openIn(shell: Shell, folder = '/film') {
    const cwd = await shell.select(folder);
    const answer = await shell.invoke('engine-open', { cwd });
    await settledAll();
    return { cwd, answer, studio: studioOf(cwd) };
  }
  /** The connections that reached an engine: the main one, then the one of the view, of each engine in turn. */
  const connections = (shell: Shell) => shell.engines.sockets.filter(socket => socket.server !== null);

  it('opens only a folder that was chosen, in a project of its own there, with an engine that it starts', async () => {
    const shell = await shellHarness();
    await expect(shell.invoke('engine-open', { cwd: path.resolve('/unchosen') })).rejects.toThrow('native folder dialog');
    await expect(shell.invoke('engine-open', {})).rejects.toThrow('Choose a project folder before opening it in the studio.');
    expect(shell.engines.children).toEqual([]);
    const { answer, studio } = await openIn(shell);
    expect(shell.mkdirSync).toHaveBeenCalledWith(studio, { recursive: true });
    // What the engine makes of its own there is kept out of the history of the folder, and the
    // project is not; a file that is there already is the user's, and stays.
    expect(shell.writeFileSync).toHaveBeenCalledWith(path.join(studio, '.gitignore'), 'cache/\nengine.lock\n', { flag: 'wx' });
    const [child] = shell.engines.children;
    // Not headless: the engine is the application's, and ends when its stdin is closed.
    expect(child.program).toBe(builtEngine());
    expect(child.args).toEqual(['--project', studio]);
    expect(child.options).toMatchObject({ cwd: path.dirname(builtEngine()), windowsHide: true, detached: false, stdio: ['pipe', 'pipe', 'pipe'] });
    expect(answer).toEqual({ session: 1, project: studio, owned: true, pid: child.pid, revision: 0 });
    expect(told(shell)).toEqual([{ kind: 'opened', session: 1, project: studio, owned: true, pid: child.pid, revision: 0 }]);
    // What it logs is read as it comes, into a log of its own.
    expect(child.stderr.listenerCount('data')).toBe(1);
    // Two connections, each of which said hello as the desktop.
    const [main, view] = connections(shell);
    for (const socket of [main, view]) expect(socket.written[0]).toMatchObject({ method: 'hello', params: { client: 'desktop', protocol: 1 } });
  });

  it('keeps the engine of a folder that is open, and closes it before it opens another', async () => {
    const shell = await shellHarness();
    const { cwd } = await openIn(shell);
    await expect(shell.invoke('engine-open', { cwd })).resolves.toMatchObject({ session: 1 });
    expect(shell.engines.children).toHaveLength(1);
    // A page that was loaded again is told again, and fetches the project anew.
    expect(told(shell).map(event => [event.kind, event.session])).toEqual([['opened', 1], ['opened', 1]]);
    await openIn(shell, '/another-film');
    const [first, second] = shell.engines.children;
    expect(first.stdin.end).toHaveBeenCalledOnce();
    expect(first.exitCode).toBe(0);
    expect(second.args).toEqual(['--project', studioOf(path.resolve('/another-film'))]);
    expect(told(shell).slice(2).map(event => [event.kind, event.session, event.expected])).toEqual([
      ['exited', 1, true], ['opened', 2, undefined],
    ]);
  });

  it('lets the page call the methods that it may, and no other', async () => {
    const shell = await shellHarness();
    expect(await shell.invoke('engine-call', { method: 'project.get' })).toMatchObject({ ok: false, error: { code: null } });
    await openIn(shell);
    const [main, view] = connections(shell);
    const allowed = [
      'project.get', 'transaction.apply', 'history.undo', 'history.redo', 'media.make_proxy', 'transport.load', 'transport.play',
      'transport.pause', 'transport.seek', 'transport.step', 'transport.state', 'transport.proxies',
    ];
    for (const method of allowed) {
      const params = method === 'transaction.apply' ? { ops: [{ op: 'remove_clip', timeline: 't1', id: 'c1' }] } : { frame: 3 };
      expect(await shell.invoke('engine-call', { method, params })).toEqual({ ok: true, result: {} });
      expect(main.written.at(-1)).toMatchObject({ method, params });
    }
    const sent = main.written.length + view.written.length;
    for (const method of ['view.attach', 'view.detach', 'media.import', 'hello', 'project.changed', 'transport', '', 7, undefined, 'constructor']) {
      await expect(shell.invoke('engine-call', { method, params: {} })).rejects.toThrow('no method of the engine');
    }
    for (const params of [[1], 'ops', 3]) {
      await expect(shell.invoke('engine-call', { method: 'transaction.apply', params })).rejects.toThrow('is an object');
    }
    expect(main.written.length + view.written.length).toBe(sent);
    // What the engine refuses comes to the page whole, with its code and data.
    shell.engines.servers.get(main.address)!.answers.set('transaction.apply', () => ({
      error: { code: -32001, message: 'the clip covers frames that clip c1 covers', data: { reason: 'overlap', with: 'c1', index: 0 } },
    }));
    expect(await shell.invoke('engine-call', { method: 'transaction.apply', params: { ops: [] } })).toEqual({
      ok: false, error: { code: -32001, message: 'the clip covers frames that clip c1 covers', data: { reason: 'overlap', with: 'c1', index: 0 } },
    });
  });

  it('lets a transaction of the page name no file: no media that it adds, and no proxy that it sets', async () => {
    const shell = await shellHarness();
    await openIn(shell);
    const [main] = connections(shell);
    const sent = main.written.length;
    const media = { id: '', path: '\\\\127.0.0.1\\share\\x.mp4', identity: { size: 1, modified_us: 0, digest: '0'.repeat(64) }, facts: { duration: { ticks: 1, scale: 1 } } };
    const proxy = { path: 'C:\\Users\\me\\precious.mp4', width: 1, height: 1, codec: 'hevc', digest_of_source: '0'.repeat(64) };
    for (const ops of [
      [{ op: 'add_media', media }],
      [{ op: 'remove_clip', timeline: 't1', id: 'c1' }, { op: 'add_media', media, index: 0 }],
      [{ op: 'set_proxy', media: 'm1', proxy }],
    ]) {
      await expect(shell.invoke('engine-call', { method: 'transaction.apply', params: { ops } })).rejects.toThrow(/adds no media|sets no proxy/);
    }
    for (const params of [{}, { ops: 'all' }, { ops: [null] }, { ops: [[]] }, null]) {
      await expect(shell.invoke('engine-call', { method: 'transaction.apply', params })).rejects.toThrow(/list of operations|is an object/);
    }
    expect(main.written.length).toBe(sent);
    // A proxy may be taken away: the engine makes it again at its next start.
    for (const op of [{ op: 'set_proxy', media: 'm1', proxy: null }, { op: 'set_proxy', media: 'm1' }]) {
      expect(await shell.invoke('engine-call', { method: 'transaction.apply', params: { ops: [op] } })).toMatchObject({ ok: true });
      expect(main.written.at(-1)).toMatchObject({ method: 'transaction.apply', params: { ops: [op] } });
    }
  });

  it('hands the page what the engine tells, in order, once', async () => {
    const shell = await shellHarness();
    await openIn(shell);
    const server = shell.engines.servers.values().next().value!;
    server.notify('project.changed', { revision: 1, by: 'agent', made_by: 'agent', done: [], next_id: 2 });
    server.notify('transport.state', { media: null });
    server.notify('media.proxy', { media: 'm1', state: 'queued' });
    server.notify('project.changed', { revision: 2, by: 'desktop', made_by: 'desktop', done: [], next_id: 3 });
    await settledAll();
    // Both connections hear every change; the page hears each once.
    expect(told(shell).filter(event => event.kind === 'notification')).toEqual([
      { kind: 'notification', session: 1, method: 'project.changed', params: { revision: 1, by: 'agent', made_by: 'agent', done: [], next_id: 2 } },
      { kind: 'notification', session: 1, method: 'transport.state', params: { media: null } },
      { kind: 'notification', session: 1, method: 'media.proxy', params: { media: 'm1', state: 'queued' } },
      { kind: 'notification', session: 1, method: 'project.changed', params: { revision: 2, by: 'desktop', made_by: 'desktop', done: [], next_id: 3 } },
    ]);
  });

  it('imports only files that the user chose in the dialog of the system', async () => {
    const shell = await shellHarness();
    expect(await shell.invoke('engine-import', { language: 'en' })).toMatchObject({ ok: false });
    expect(shell.dialog.showOpenDialog).not.toHaveBeenCalled();
    await openIn(shell);
    const [main] = connections(shell);
    shell.engines.servers.get(main.address)!.answers.set('media.import', params => ({
      result: { revision: 1, added: params.paths.map((_: string, at: number) => `m${at + 1}`), next_id: 3 },
    }));
    const chosen = [path.resolve('/media/a.mp4'), path.resolve('/media/b.wav')];
    shell.dialog.showOpenDialog.mockResolvedValueOnce({ canceled: false, filePaths: [...chosen, 'relative.mp4'] });
    // Whatever paths the page names are no part of it.
    expect(await shell.invoke('engine-import', { language: 'ja', paths: ['C:\\secret.mp4'] }))
      .toEqual({ ok: true, result: { revision: 1, added: ['m1', 'm2'], next_id: 3 } });
    expect(main.written.at(-1)).toMatchObject({ method: 'media.import', params: { paths: chosen } });
    const [over, options] = shell.dialog.showOpenDialog.mock.calls.at(-1)!;
    expect(over).toBe(shell.window);
    expect(options.title).toBe('読み込むメディアを選択');
    expect(options.properties).toEqual(['openFile', 'multiSelections']);
    expect(options.filters[0].extensions).toEqual(expect.arrayContaining(['mp4', 'mov', 'mkv', 'wav', 'mp3', 'png', 'jpg']));
    expect(options.filters[1].extensions).toEqual(['*']);
    // Nothing chosen is nothing imported.
    const asked = main.written.length;
    expect(await shell.invoke('engine-import', { language: 'en' })).toEqual({ ok: true, result: null });
    expect(shell.dialog.showOpenDialog.mock.calls.at(-1)![1].title).toBe('Choose media to import');
    expect(main.written.length).toBe(asked);
  });

  it('makes the view where the page keeps the viewer, in the window, on a connection of its own', async () => {
    const shell = await shellHarness();
    // Before an engine is open, the viewer is only remembered.
    await shell.invoke('engine-view-rect', { x: 10, y: 20, width: 640, height: 360 });
    await openIn(shell);
    const [main, view] = connections(shell);
    // The handle as the engine reads it: the 8 bytes of the window, in decimal, whole.
    expect(shell.window.getNativeWindowHandle).toHaveBeenCalled();
    expect(view.written.at(-1)).toMatchObject({ method: 'view.attach', params: { parent: '18446744069414588980', x: 10, y: 20, width: 640, height: 360 } });
    expect(told(shell).at(-1)).toEqual({ kind: 'attached', session: 1, adapter: null, backend: null });
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 800, height: 450 });
    expect(view.written.at(-1)).toMatchObject({ method: 'view.place', params: { x: 0, y: 0, width: 800, height: 450 } });
    await shell.invoke('engine-view-show', { shown: false });
    expect(view.written.at(-1)).toMatchObject({ method: 'view.show', params: { shown: false } });
    const sent = view.written.length;
    await shell.invoke('engine-view-show', { shown: false });
    expect(view.written.length).toBe(sent);
    // The page may move the view with a call too, which the shell remembers as well.
    expect(await shell.invoke('engine-call', { method: 'view.place', params: { x: 1, y: 2, width: 3, height: 4 } })).toEqual({ ok: true, result: {} });
    expect(view.written.at(-1)).toMatchObject({ method: 'view.place', params: { x: 1, y: 2, width: 3, height: 4 } });
    // Nothing of the view goes on the main connection.
    expect(main.methods().filter(method => method?.startsWith('view.'))).toEqual([]);
    // Whole pixels of the screen, at or after the corner, and no larger than the engine draws.
    for (const rect of [
      { x: -1, y: 0, width: 1, height: 1 }, { x: 0.5, y: 0, width: 1, height: 1 }, { x: 0, y: 0, width: 16_385, height: 1 },
      { x: 0, y: 0, width: 1, height: '1' }, { x: 0, y: 0, width: 1 }, null, 'rect', { x: 40_000, y: 0, width: 1, height: 1 },
    ]) {
      expect(() => shell.invoke('engine-view-rect', rect)).toThrow('whole pixels');
    }
    for (const asked of [{ shown: 'no' }, {}, null]) expect(() => shell.invoke('engine-view-show', asked)).toThrow('shown or not');
    expect(view.written.length).toBe(sent + 1);
  });

  it('makes the view where the viewer is once it is made, and hides it there if it was hidden meanwhile', async () => {
    const shell = await shellHarness();
    await openIn(shell);
    const [, view] = connections(shell);
    const server = shell.engines.servers.get(view.address)!;
    server.answers.set('view.attach', () => 'hold');
    await shell.invoke('engine-view-rect', { x: 1, y: 1, width: 100, height: 100 });
    await shell.invoke('engine-view-rect', { x: 2, y: 2, width: 200, height: 200 });
    await shell.invoke('engine-view-show', { shown: false });
    expect(view.methods().filter(method => method === 'view.attach')).toHaveLength(1);
    server.answerHeld('view.attach', { result: { adapter: 'NVIDIA GeForce RTX 4080', backend: 'Dx12' } });
    await settledAll();
    expect(view.methods().slice(-3)).toEqual(['view.attach', 'view.show', 'view.place']);
    expect(view.written.at(-1)).toMatchObject({ params: { x: 2, y: 2, width: 200, height: 200 } });
    expect(told(shell).at(-1)).toEqual({ kind: 'attached', session: 1, adapter: 'NVIDIA GeForce RTX 4080', backend: 'Dx12' });
  });

  it('makes the view again for an engine that is opened anew, and where its window was let go', async () => {
    const shell = await shellHarness();
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 320, height: 180 });
    const { cwd } = await openIn(shell);
    await shell.invoke('engine-close');
    await shell.invoke('engine-open', { cwd });
    await settledAll();
    const views = connections(shell).filter((_, at) => at % 2 === 1);
    expect(views.map(socket => socket.methods().filter(method => method === 'view.attach').length)).toEqual([1, 1]);
    // A view whose window is gone is made again where the page keeps the viewer.
    const [, view] = connections(shell).slice(2);
    shell.engines.servers.get(view.address)!.answers.set('view.place', () => ({ error: { code: -32003, message: 'the view is gone', data: { reason: 'gone' } } }));
    await shell.invoke('engine-view-rect', { x: 5, y: 5, width: 320, height: 180 });
    await settledAll();
    expect(view.methods().slice(-2)).toEqual(['view.place', 'view.attach']);
    expect(view.written.at(-1)).toMatchObject({ params: { x: 5, y: 5 } });
  });

  it('says on macOS that the engine makes no view there, and keeps the menu of the application', async () => {
    const socket = '/tmp/columbina-0/0123456789abcdef.sock';
    const mac = await shellHarness(undefined, {}, { platform: 'darwin' });
    expect(mac.window.removeMenu).not.toHaveBeenCalled();
    mac.engines.starts.push({ listen: socket });
    const { answer } = await openIn(mac);
    expect(answer.owned).toBe(true);
    expect(mac.engines.children[0].program).toBe(builtEngine('columbina-engine'));
    expect(mac.engines.children[0].options.detached).toBe(true);
    const [, view] = connections(mac);
    mac.engines.servers.get(socket)!.answers.set('view.attach', () => ({
      error: { code: -32003, message: 'views are not made on this system', data: { reason: 'unsupported' } },
    }));
    await mac.invoke('engine-view-rect', { x: 0, y: 0, width: 400, height: 180 });
    await settledAll();
    expect(told(mac).at(-1)).toEqual({ kind: 'unsupported', session: 1, reason: 'unsupported', message: 'views are not made on this system' });
    const sent = view.written.length;
    await mac.invoke('engine-view-rect', { x: 0, y: 0, width: 500, height: 180 });
    await mac.invoke('engine-view-show', { shown: false });
    expect(view.written.length).toBe(sent);
    // On Windows the window has no menu, which would take the top of its client area.
    const windows = await shellHarness();
    expect(windows.window.removeMenu).toHaveBeenCalledOnce();
  });

  it('reaches the engine of an agent that has the project open, and says that it has no view', async () => {
    const shell = await shellHarness();
    const cwd = await shell.select('/film');
    const agents = shell.engines.serve('\\\\.\\pipe\\columbina-engine-fedcba9876543210', studioOf(cwd), 31);
    agents.answers.set('view.attach', () => ({ error: { code: -32601, message: 'no such method' } }));
    shell.engines.starts.push({ held: agents.address });
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 320, height: 180 });
    await expect(shell.invoke('engine-open', { cwd })).resolves.toMatchObject({ owned: false, pid: 31 });
    await settledAll();
    expect(told(shell).at(-1)).toMatchObject({ kind: 'unsupported', reason: 'headless' });
    // Closing lets go of its connections, and leaves that engine to its agent.
    await shell.invoke('engine-close');
    await settledAll();
    expect(agents.sockets.every(socket => socket.destroyed)).toBe(true);
    expect(shell.engines.servers.has(agents.address)).toBe(true);
    expect(told(shell).at(-1)).toMatchObject({ kind: 'exited', expected: true });
  });

  it('tells the page why the engine could not start', async () => {
    const shell = await shellHarness();
    shell.engines.starts.push({ error: 'project.json cannot be read: expected value at line 1' });
    const cwd = await shell.select('/film');
    await expect(shell.invoke('engine-open', { cwd })).rejects.toThrow('project.json cannot be read');
    expect(told(shell)).toEqual([{ kind: 'error', session: 1, message: 'project.json cannot be read: expected value at line 1' }]);
    // A COLUMBINA_ENGINE that is no whole path is no engine.
    const relative = await shellHarness(undefined, { COLUMBINA_ENGINE: 'columbina-engine.exe' });
    await expect(relative.invoke('engine-open', { cwd: await relative.select('/film') }))
      .rejects.toThrow('COLUMBINA_ENGINE names the engine program by a path that is not whole, which is not taken: columbina-engine.exe');
    const none = await shellHarness();
    none.absent.add(builtEngine());
    none.absent.add(builtEngine().replace(`${path.sep}release${path.sep}`, `${path.sep}debug${path.sep}`));
    await expect(none.invoke('engine-open', { cwd: await none.select('/film') })).rejects.toThrow('There is no engine program');
    expect(relative.engines.children).toEqual([]);
  });

  it('tells the page that the engine ended, and opens it again when it is asked', async () => {
    const shell = await shellHarness();
    const { cwd } = await openIn(shell);
    shell.engines.children[0].end(1);
    await settledAll();
    expect(told(shell).at(-1)).toEqual({ kind: 'exited', session: 1, expected: false, code: 1, signal: null });
    expect(await shell.invoke('engine-call', { method: 'project.get' })).toMatchObject({ ok: false });
    await expect(shell.invoke('engine-open', { cwd })).resolves.toMatchObject({ session: 2, owned: true });
    expect(shell.engines.children).toHaveLength(2);
  });

  it('closes the engine with the window, and before the application quits', async () => {
    const shell = await shellHarness();
    await openIn(shell);
    const [child] = shell.engines.children;
    shell.window.emit('closed');
    await settledAll();
    expect(child.stdin.end).toHaveBeenCalledOnce();
    expect(told(shell).at(-1)).toMatchObject({ kind: 'exited', expected: true });
    const quitting = await shellHarness();
    await openIn(quitting);
    quitting.app.emit('window-all-closed');
    expect(quitting.app.quit).not.toHaveBeenCalled();
    await settledAll();
    expect(quitting.engines.children[0].exitCode).toBe(0);
    expect(quitting.app.quit).toHaveBeenCalled();
    // However the application ends, the stdin of its engine is closed.
    const ending = await shellHarness();
    await openIn(ending);
    ending.app.emit('will-quit');
    expect(ending.engines.children[0].stdin.end).toHaveBeenCalled();
  });

  it('kills an engine that does not end when its stdin is closed', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const shell = await shellHarness();
      const cwd = await shell.select('/film');
      const opening = shell.invoke('engine-open', { cwd });
      await vi.advanceTimersByTimeAsync(10);
      await opening;
      const [child] = shell.engines.children;
      child.stubborn = true;
      const closing = shell.invoke('engine-close');
      await vi.advanceTimersByTimeAsync(4_900);
      expect(child.exitCode).toBeNull();
      expect(shell.spawn.mock.calls.some(([, args]) => args.includes('/pid'))).toBe(false);
      await vi.advanceTimersByTimeAsync(200);
      // As the shell ends what it starts: taskkill, with what the engine started.
      expect(shell.spawn.mock.calls.find(([, args]) => args.includes('/pid')))
        .toEqual(['taskkill.exe', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' }]);
      await vi.advanceTimersByTimeAsync(10);
      await closing;
      expect(child.exitCode).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('gives the service the engine of the window, and the engine the libraries of FFmpeg', async () => {
    const ffmpeg = path.resolve('/ffmpeg');
    const named = path.resolve('/builds/columbina-engine.exe');
    const shell = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python, COLUMBINA_ENGINE: named, FFMPEG_DIR: ffmpeg, Path: 'C:\\Windows' });
    await shell.invoke('start', { cwd: await shell.select('/film') });
    expect(shell.spawn.mock.calls.at(-1)![2].env).toMatchObject({ COLUMBINA_ENGINE: named, FFMPEG_DIR: ffmpeg });
    await openIn(shell);
    const [child] = shell.engines.children;
    expect(child.program).toBe(named);
    // Windows writes it Path, and the libraries go first, before any other build of libav on it.
    expect(child.options.env.Path).toBe(`${path.join(ffmpeg, 'bin')}${path.delimiter}C:\\Windows`);
    expect(child.options.env).not.toHaveProperty('PATH');
    // Without a name, the newest build of the repository.
    const built = await shellHarness(undefined, { COLUMBINA_SERVICE_EXECUTABLE: python });
    await built.invoke('start', { cwd: await built.select('/film') });
    expect(built.spawn.mock.calls.at(-1)![2].env.COLUMBINA_ENGINE).toBe(builtEngine());
    // A package brings its engine, with its libraries beside it.
    const packaged = await shellHarness(undefined, { PATH: 'C:\\Windows' }, { packaged: true });
    await openIn(packaged);
    const brought = path.join(resources, 'engine', 'columbina-engine.exe');
    expect(packaged.engines.children[0].program).toBe(brought);
    expect(packaged.engines.children[0].options.env.PATH).toBe(`${path.dirname(brought)}${path.delimiter}C:\\Windows`);
  });

  it('tells a page that was loaded again how the view stands, and hides the view until it says where the viewer is', async () => {
    const shell = await shellHarness();
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 320, height: 180 });
    const { cwd } = await openIn(shell);
    const [, view] = connections(shell);
    await shell.invoke('engine-view-show', { shown: false });
    // The page is loaded again: the native view would stay over it.
    shell.window.webContents.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
    // Hidden already; nothing more is sent.
    expect(view.methods().filter(method => method === 'view.show')).toHaveLength(1);
    await shell.invoke('engine-view-show', { shown: true });
    shell.window.webContents.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
    shell.window.webContents.emit('did-start-navigation', { isMainFrame: false, isSameDocument: false });
    shell.window.webContents.emit('did-start-navigation', { isMainFrame: true, isSameDocument: true });
    expect(view.methods().filter(method => method === 'view.show')).toHaveLength(1);
    // The new page opens its project again, and hears how the view stands.
    await shell.invoke('engine-open', { cwd });
    expect(told(shell).slice(-2)).toEqual([
      expect.objectContaining({ kind: 'opened', session: 1 }), { kind: 'attached', session: 1, adapter: null, backend: null },
    ]);
    // Shown again only once it says where the viewer is.
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 330, height: 180 });
    expect(view.methods().slice(-2)).toEqual(['view.place', 'view.show']);
    expect(view.written.at(-1)).toMatchObject({ params: { shown: true } });
    // A page that crashed is the same.
    shell.window.webContents.emit('render-process-gone', {}, { reason: 'crashed' });
    expect(view.written.at(-1)).toMatchObject({ method: 'view.show', params: { shown: false } });
  });

  it('takes no late word of a view that was made before for the one there is', async () => {
    const shell = await shellHarness();
    await shell.invoke('engine-view-rect', { x: 0, y: 0, width: 320, height: 180 });
    await openIn(shell);
    const [, view] = connections(shell);
    const server = shell.engines.servers.get(view.address)!;
    // Two moves go out; the first is answered that the view is gone, which makes it again.
    server.answers.set('view.place', () => 'hold');
    await shell.invoke('engine-view-rect', { x: 1, y: 0, width: 320, height: 180 });
    await shell.invoke('engine-view-rect', { x: 2, y: 0, width: 320, height: 180 });
    const gone = { error: { code: -32003, message: 'the view is gone', data: { reason: 'gone' } } };
    server.answerHeld('view.place', gone);
    await settledAll();
    expect(view.methods().filter(method => method === 'view.attach')).toHaveLength(2);
    // The second, about the view before, says so late: no third view is made for it.
    server.answerHeld('view.place', gone);
    await settledAll();
    expect(view.methods().filter(method => method === 'view.attach')).toHaveLength(2);
  });

  it('takes the project over from the headless engine of an agent, and tells the page', async () => {
    const shell = await shellHarness();
    const cwd = await shell.select('/film');
    const agents = shell.engines.headless('\\\\.\\pipe\\columbina-engine-fedcba9876543210', studioOf(cwd), 31, shell.running);
    shell.engines.starts.push({ held: agents.address });
    await expect(shell.invoke('engine-open', { cwd })).resolves.toMatchObject({ session: 1, owned: true });
    expect(agents.sockets[0].methods()).toEqual(['hello', 'engine.yield']);
    // What that engine tells its clients as it yields comes too, before the project is opened.
    expect(told(shell).map(event => event.method ?? event.kind)).toEqual(['taking-over', 'engine.yielding', 'opened']);
    expect(told(shell)[0]).toEqual({ kind: 'taking-over', session: 1, pid: 31 });
  });

  it('gives back the keys that the menu gave: zoom and full screen, and from source the tools of the page', async () => {
    const press = (shell: Shell, input: Record<string, unknown>) => {
      const event = { preventDefault: vi.fn() };
      shell.window.webContents.emit('before-input-event', event, { type: 'keyDown', control: false, shift: false, alt: false, meta: false, ...input });
      return event.preventDefault.mock.calls.length === 1;
    };
    const shell = await shellHarness();
    const page = shell.window.webContents;
    expect(press(shell, { control: true, key: '=' })).toBe(true);
    expect(press(shell, { control: true, shift: true, key: '+' })).toBe(true);
    expect(page.setZoomLevel.mock.calls).toEqual([[0.5], [1]]);
    // The page measures the viewer again: the pixels of the screen that it covers changed.
    expect(told(shell).at(-1)).toEqual({ kind: 'measure', session: null });
    expect(press(shell, { control: true, key: '-' })).toBe(true);
    expect(press(shell, { control: true, code: 'Numpad0', key: 'Insert' })).toBe(true);
    expect(page.setZoomLevel.mock.calls.slice(2)).toEqual([[0.5], [0]]);
    expect(press(shell, { key: 'F11' })).toBe(true);
    expect(shell.window.setFullScreen).toHaveBeenCalledWith(true);
    expect(press(shell, { control: true, key: 'r' })).toBe(true);
    expect(page.reload).toHaveBeenCalledOnce();
    expect(press(shell, { control: true, shift: true, key: 'I' })).toBe(true);
    expect(press(shell, { key: 'F12' })).toBe(true);
    expect(page.toggleDevTools).toHaveBeenCalledTimes(2);
    // Other keys are the page's, and so is a key let go.
    for (const input of [{ key: 'a', control: true }, { key: '=', control: true, alt: true }, { key: 'F11', type: 'keyUp' }, { key: '0' }]) {
      expect(press(shell, input)).toBe(false);
    }
    // A package does not load its page again, nor show its tools.
    const packaged = await shellHarness(undefined, {}, { packaged: true });
    expect(press(packaged, { control: true, key: 'r' })).toBe(false);
    expect(press(packaged, { key: 'F12' })).toBe(false);
    expect(press(packaged, { control: true, key: '=' })).toBe(true);
    // macOS keeps its menu, and the keys are its own.
    const mac = await shellHarness(undefined, {}, { platform: 'darwin' });
    expect(mac.window.webContents.listenerCount('before-input-event')).toBe(0);
  });

  it('answers only its own window', async () => {
    const shell = await shellHarness();
    for (const name of ['engine-open', 'engine-call', 'engine-import', 'engine-view-rect', 'engine-view-show', 'engine-close']) {
      await expect(Promise.resolve().then(() => shell.handlers.get(`columbina:${name}`)!(shell.unrelated, { cwd: path.resolve('/film') })))
        .rejects.toThrow('IPC sender');
    }
    expect(shell.engines.children).toEqual([]);
  });
});
