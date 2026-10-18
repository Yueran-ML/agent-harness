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
}

/** What a login shell writes by itself, before it says its PATH. */
const SAID_PATH = 'Last login: Thu Oct 15 on ttys000\n_COLUMBINA_PATH_/opt/homebrew/bin:/usr/bin:/bin_COLUMBINA_PATH_';

/** Where a package keeps what it brings beside the application, such as its Python. */
const resources = path.resolve('/columbina-test/resources');

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
  const app = Object.assign(new EventEmitter(), {
    setName: vi.fn(), setAppUserModelId: vi.fn(), isPackaged: system.packaged ?? false,
    getVersion: () => system.version ?? '0.1.2',
    dock: { setIcon: vi.fn() },
    whenReady: () => Promise.resolve(), quit: vi.fn(),
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
    focus = vi.fn();
    restore = vi.fn();
    isDestroyed = () => false;
    setMenuBarVisibility = vi.fn();
    destroy = vi.fn();
    setBackgroundColor = vi.fn();
    setTitleBarOverlay = vi.fn();
    setSheetOffset = vi.fn();
  }
  let live = 0;
  let peakLive = 0;
  const children: any[] = [];
  // The runs of `columbina setup`: what each was started with and given, and what it answers.
  const setups: { args: string[]; cwd: string; input: string | null; child: any }[] = [];
  // The runs of `columbina login github`, which write their lines through a reader of lines.
  const signIns: any[] = [];
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
    if (args.includes('setup') || args.includes('--status') || args.includes('logout')) {
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
    if (args.includes('/pid')) return Object.assign(new EventEmitter(), { killed: args });
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
  const world = { openExternal: vi.fn(), showItemInFolder: vi.fn() };
  // The network, which answers nothing unless a test says what, and the sessions of their own that
  // the shell asks it through, by the names of their partitions.
  const net = { fetch: vi.fn(async (_address: string, _init?: any): Promise<any> => { throw new Error('No network in the tests.'); }) };
  const partitions: string[] = [];
  const session = { fromPartition: vi.fn((name: string) => { partitions.push(name); return { fetch: net.fetch }; }) };
  const writeFileSync = vi.fn((file: string, text: string) => { files.set(path.resolve(file), String(text)); });
  const menu = { buildFromTemplate: vi.fn((template: unknown[]) => ({ template })), setApplicationMenu: vi.fn() };
  const modules: Record<string, unknown> = {
    electron: {
      app, BrowserWindow: TestWindow, Menu: menu, dialog, shell: world, Notification: TestNotification, session,
      ipcMain: { handle: (name: string, callback: (...args: any[]) => any) => handlers.set(name, callback), on: vi.fn() },
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
      existsSync: (file: string) => !absent.has(path.resolve(file)), mkdirSync: vi.fn(), realpathSync,
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
      createWriteStream: () => Object.assign(new EventEmitter(), { end: vi.fn() }),
    },
    'node:fs/promises': disk,
    'node:path': path,
    'node:url': { pathToFileURL, fileURLToPath },
    'node:string_decoder': { StringDecoder },
  };
  runInNewContext(source, {
    require: (name: string) => { if (!(name in modules)) throw new Error(`Unexpected module: ${name}`); return modules[name]; },
    __dirname: path.dirname(mainPath), process: { platform, env, execPath: 'Columbina.exe', resourcesPath: resources },
    // The clock of the tests, so that a test that sets it sets it for the shell too.
    URL, setTimeout, clearTimeout, Date, AbortSignal,
  }, { filename: mainPath });
  await Promise.resolve();
  const event = { sender: window.webContents, senderFrame: window.webContents.mainFrame };
  const invoke = (name: string, ...args: unknown[]) => handlers.get(`columbina:${name}`)!(event, ...args);
  const select = async (folder: string) => {
    dialog.showOpenDialog.mockResolvedValueOnce({ canceled: false, filePaths: [folder] });
    return invoke('select-folder') as Promise<string>;
  };
  const unrelated = { sender: window.webContents, senderFrame: { url: window.webContents.mainFrame.url } };
  const preferences = () => JSON.parse(files.get(preferencesFile) ?? 'null');
  // The window for the key, once the shell has opened it, and a call as that window makes it.
  const keyWindow = () => windows[1];
  const asKeyWindow = (name: string, ...args: unknown[]) => handlers.get(`columbina-key:${name}`)!(
    { sender: keyWindow().webContents, senderFrame: keyWindow().webContents.mainFrame }, ...args);
  return {
    handlers, invoke, select, app, window, windows, event, unrelated, children, spawn, realpathSync, disk, drive, file, sent,
    dialog, files, gone, absent, preferences, writeFileSync, notifications, Notification: TestNotification, system: world, net,
    partitions,
    setups, setup, keyWindow, asKeyWindow, menu, logins, signIns, lineReaders,
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
    for (const method of ['select-folder', 'start', 'local-music']) {
      await expect(shell.handlers.get(`columbina:${method}`)!(unrelatedFrame, { cwd: '/selected-project' })).rejects.toThrow('IPC sender');
    }
    expect(shell.spawn).not.toHaveBeenCalled();
    expect(shell.disk.open).not.toHaveBeenCalled();
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

  it('runs the Python that the package brings, and adds nothing for that Python to the environment', async () => {
    const env = { PATH: 'C:\\Windows', PYTHONPATH: 'C:\\elsewhere', USERPROFILE: 'C:\\Users\\a' };
    const shell = await shellHarness(undefined, env, { packaged: true });
    const cwd = await shell.select('/selected-project');
    await expect(shell.invoke('start', { cwd })).resolves.toEqual({ mode: 'service', cwd });
    const [executable, args, options] = shell.spawn.mock.calls[0];
    expect(executable).toBe(windowsPython);
    expect(args).toEqual(['-I', '-B', '-m', 'columbina', 'serve']);
    // The commands of the agent inherit this environment. What keeps that Python to itself are its flags.
    expect(options.env).toEqual(env);
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
    expect(served.env).toEqual({ ...env, PATH: '/opt/homebrew/bin:/usr/bin:/bin' });
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
      PATH: '/usr/bin', https_proxy: 'http://given:1', HTTP_PROXY: '', http_proxy: 'http://shell:3',
    });
  });

  it('on macOS takes a proxy from the login shell also where it says no PATH', async () => {
    const shell = await shellHarness(undefined, { PATH: finder }, {
      platform: 'darwin', packaged: true, loginShell: '_COLUMBINA_https_proxy_http://127.0.0.1:7890_COLUMBINA_https_proxy_',
    });
    const cwd = await shell.select('/selected-project');
    await shell.invoke('start', { cwd });
    expect(shell.spawn.mock.calls.at(-1)![2].env).toEqual({ PATH: added, https_proxy: 'http://127.0.0.1:7890' });
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
    expect(shell.preferences()).toEqual({ theme: 'moonwhite', recent: [], githubSkipped: false });
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

  it('answers none of this to anything but the page of the application', async () => {
    const shell = await told();
    const runs = shell.setups.length;
    await expect((async () => shell.handlers.get('columbina:models')!(shell.unrelated, 'deepseek'))()).rejects.toThrow('IPC sender');
    await expect((async () => shell.handlers.get('columbina:choose-model')!(shell.unrelated, { service: 'deepseek', model: 'deepseek-pro' }))())
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
