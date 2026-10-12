import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { describe, expect, it, vi } from 'vitest';
import { themes } from '../src/themes';
import { themeColour } from './stylesheet';

const mainPath = fileURLToPath(new URL('../electron/main.cjs', import.meta.url));
const source = readFileSync(mainPath, 'utf8');

async function shellHarness() {
  const handlers = new Map<string, (...args: any[]) => any>();
  const app = Object.assign(new EventEmitter(), {
    setName: vi.fn(), setAppUserModelId: vi.fn(), isPackaged: true,
    whenReady: () => Promise.resolve(), quit: vi.fn(),
    getPath: vi.fn((name: string) => {
      if (name === 'documents') throw new Error('Documents must never be used as a project.');
      return path.resolve(`/columbina-test/${name}`);
    }),
  });
  const dialog = { showOpenDialog: vi.fn().mockResolvedValue({ canceled: true, filePaths: [] }) };
  const sent: [string, unknown][] = [];
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
    constructor(options: unknown) { super(); this.options = options; window = this; }
    isDestroyed = () => false;
    setMenuBarVisibility = vi.fn();
    loadFile = vi.fn();
    destroy = vi.fn();
    setBackgroundColor = vi.fn();
    setTitleBarOverlay = vi.fn();
  }
  let live = 0;
  let peakLive = 0;
  const children: any[] = [];
  const spawn = vi.fn((_executable, _args, options) => {
    live += 1;
    peakLive = Math.max(peakLive, live);
    const child = Object.assign(new EventEmitter(), {
      pid: 100 + children.length, cwd: options.cwd, exitCode: null as number | null, signalCode: null,
      stdout: new EventEmitter(), stderr: { pipe: vi.fn() },
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
  const disk = {
    realpath: vi.fn(async (value: string) => path.resolve(value)),
    open: vi.fn().mockResolvedValue(file),
  };
  const modules: Record<string, unknown> = {
    electron: {
      app, BrowserWindow: TestWindow, dialog, shell: { openExternal: vi.fn() },
      ipcMain: { handle: (name: string, callback: (...args: any[]) => any) => handlers.set(name, callback), on: vi.fn() },
    },
    'node:child_process': { spawn },
    'node:readline': { createInterface: () => Object.assign(new EventEmitter(), { close: vi.fn() }) },
    'node:fs': {
      existsSync: () => true, mkdirSync: vi.fn(), realpathSync,
      statSync: () => ({ isDirectory: () => true }),
      createWriteStream: () => Object.assign(new EventEmitter(), { end: vi.fn() }),
    },
    'node:fs/promises': disk,
    'node:path': path,
    'node:url': { pathToFileURL, fileURLToPath },
  };
  runInNewContext(source, {
    require: (name: string) => { if (!(name in modules)) throw new Error(`Unexpected module: ${name}`); return modules[name]; },
    __dirname: path.dirname(mainPath), process: { platform: 'win32', env: {}, execPath: 'Columbina.exe' },
    URL, setTimeout, clearTimeout,
  }, { filename: mainPath });
  await Promise.resolve();
  const event = { sender: window.webContents, senderFrame: window.webContents.mainFrame };
  const invoke = (name: string, ...args: unknown[]) => handlers.get(`columbina:${name}`)!(event, ...args);
  const select = async (folder: string) => {
    dialog.showOpenDialog.mockResolvedValueOnce({ canceled: false, filePaths: [folder] });
    return invoke('select-folder') as Promise<string>;
  };
  return { handlers, invoke, select, app, window, event, children, spawn, realpathSync, disk, file, sent, live: () => live, peakLive: () => peakLive };
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
