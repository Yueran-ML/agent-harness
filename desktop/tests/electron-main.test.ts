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
import { themeColour } from './stylesheet';

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

/**
 * The shell, with everything around it made up.
 *
 * Args:
 *   kept: What the file of preferences holds when the shell starts, as text. Without it there is no such file.
 *   env: The environment of the shell. Without the variable that names the service, the shell replays.
 */
async function shellHarness(kept?: string, env: Record<string, string> = {}) {
  const handlers = new Map<string, (...args: any[]) => any>();
  // The files that the shell reads and writes whole, by their paths.
  const files = new Map<string, string>();
  if (kept !== undefined) files.set(preferencesFile, kept);
  // The folders that are not there any more.
  const gone = new Set<string>();
  const missing = (file: string) => Object.assign(new Error(`ENOENT: ${file}`), { code: 'ENOENT' });
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
  }
  let live = 0;
  let peakLive = 0;
  const children: any[] = [];
  // The runs of `columbina setup`: what each was started with and given, and what it answers.
  const setups: { args: string[]; cwd: string; input: string | null; child: any }[] = [];
  const setup = {
    answer: ((): SetupRun => ({ out: '{"use": null, "services": []}\n', code: 0 })) as (args: string[], input: string) => SetupRun | null,
  };
  const spawn = vi.fn((_executable, args: string[], options) => {
    if (args.includes('setup')) {
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
  const disk = {
    realpath: vi.fn(async (value: string) => path.resolve(value)),
    open: vi.fn().mockResolvedValue(file),
  };
  const notifications: any[] = [];
  class TestNotification extends EventEmitter {
    static supported = true;
    static isSupported = () => TestNotification.supported;
    show = vi.fn();
    constructor(public options: unknown) { super(); notifications.push(this); }
  }
  const system = { openExternal: vi.fn(), showItemInFolder: vi.fn() };
  const writeFileSync = vi.fn((file: string, text: string) => { files.set(path.resolve(file), String(text)); });
  const modules: Record<string, unknown> = {
    electron: {
      app, BrowserWindow: TestWindow, dialog, shell: system, Notification: TestNotification,
      ipcMain: { handle: (name: string, callback: (...args: any[]) => any) => handlers.set(name, callback), on: vi.fn() },
    },
    'node:child_process': { spawn },
    'node:readline': { createInterface: () => Object.assign(new EventEmitter(), { close: vi.fn() }) },
    'node:fs': {
      existsSync: () => true, mkdirSync: vi.fn(), realpathSync,
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
    __dirname: path.dirname(mainPath), process: { platform: 'win32', env, execPath: 'Columbina.exe' },
    URL, setTimeout, clearTimeout,
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
    handlers, invoke, select, app, window, windows, event, unrelated, children, spawn, realpathSync, disk, file, sent,
    dialog, files, gone, preferences, writeFileSync, notifications, Notification: TestNotification, system,
    setups, setup, keyWindow, asKeyWindow,
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
    expect(shell.preferences()).toEqual({ theme: 'moonwhite', recent: [] });
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
    for (const method of ['context', 'store', 'close']) {
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
    await expect(closed).resolves.toEqual({ stored: true });
    expect(JSON.stringify(shell.sent)).not.toContain(KEY);
  });

  it('says that nothing was stored where nothing was, or where the program refused', async () => {
    const first = await opened();
    first.shell.keyWindow().close();
    await expect(first.closed).resolves.toEqual({ stored: false });

    const second = await opened();
    second.shell.setup.answer = () => ({ out: '{"error": "what the document says under \'api_key\' cannot be a key"}\n', code: 2 });
    await second.shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash', api_key: 'a b' });
    second.shell.keyWindow().close();
    await expect(second.closed).resolves.toEqual({ stored: false });
  });

  it('is one window, however often it is asked for, and everyone who asked is told how it ended', async () => {
    const { shell, closed } = await opened();
    const again = shell.invoke('set-up-service', {}) as Promise<unknown>;
    await Promise.resolve();
    expect(shell.windows).toHaveLength(2);
    expect(shell.keyWindow().focus).toHaveBeenCalledOnce();
    await shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash' });
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ stored: true });
    await expect(again).resolves.toEqual({ stored: true });
  });

  it('can be opened again after it was closed, and begins anew', async () => {
    const { shell, closed } = await opened();
    await shell.asKeyWindow('store', { service: 'deepseek', model: 'deepseek-flash' });
    shell.keyWindow().close();
    await expect(closed).resolves.toEqual({ stored: true });
    const second = shell.invoke('set-up-service', {}) as Promise<unknown>;
    await Promise.resolve();
    expect(shell.windows).toHaveLength(3);
    shell.windows[2].close();
    await expect(second).resolves.toEqual({ stored: false });
  });
});
