import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const read = (file: string) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');

type Call = (...given: any[]) => any;
type Listener = (...given: unknown[]) => void;

/** A script of a bridge, run with an Electron that writes down what is asked of it, on a system. */
function bridgeOf(file: string, platform = 'win32') {
  const invoked: unknown[][] = [];
  const sent: unknown[][] = [];
  const listening = new Map<string, Set<Listener>>();
  const exposed: Record<string, Record<string, Call>> = {};
  const electron = {
    // Where a file lies, as Electron knows it of a file that was dropped: what the test wrote on it.
    // A file that a page makes up has no place, and one that is no file at all is refused.
    webUtils: {
      getPathForFile: (file: unknown) => {
        if (file === null || typeof file !== 'object' || !('name' in file)) throw new TypeError('That is no file.');
        return (file as { at?: string }).at ?? '';
      },
    },
    contextBridge: { exposeInMainWorld: (name: string, calls: Record<string, Call>) => { exposed[name] = calls; } },
    ipcRenderer: {
      invoke: (...given: unknown[]) => { invoked.push(given); return Promise.resolve('what the shell answers'); },
      send: (...given: unknown[]) => { sent.push(given); },
      on: (channel: string, listener: Listener) => { listening.set(channel, (listening.get(channel) ?? new Set()).add(listener)); },
      removeListener: (channel: string, listener: Listener) => { listening.get(channel)?.delete(listener); },
    },
  };
  const require = (name: string) => {
    if (name !== 'electron') throw new Error(`The script of a bridge asks for ${name}.`);
    return electron;
  };
  runInNewContext(read(file), { require, process: { platform } });
  return { exposed, invoked, sent, listening };
}

/** The calls and the values that an interface declares, by their names. */
function declared(file: string, name: string): string[] {
  const body = new RegExp(`export interface ${name} \\{\\n([\\s\\S]*?)\\n\\}`).exec(read(file))?.[1] ?? '';
  return [...body.matchAll(/^  (\w+)(?:\(|: )/gm)].map(found => found[1]);
}

const shell = read('electron/main.cjs');
/** What the shell answers, what it takes without an answer, and what it tells a page without being asked. */
const answered = [...shell.matchAll(/ipcMain\.handle\('([^']+)'/g)].map(found => found[1]);
const taken = [...shell.matchAll(/ipcMain\.on\('([^']+)'/g)].map(found => found[1]);
const told = [...new Set([...shell.matchAll(/\btell\('([^']+)'/g)].map(found => found[1]))];

/** Every call of a bridge made once, and the channels that this reached. */
function reached(file: string, name: string) {
  const bridge = bridgeOf(file);
  for (const [call, make] of Object.entries(bridge.exposed[name])) {
    // A value, such as the system, is there to be read, and is not called.
    if (typeof make === 'function') make(call.startsWith('on') ? () => undefined : {});
  }
  return {
    asked: bridge.invoked.map(given => given[0] as string),
    said: bridge.sent.map(given => given[0] as string),
    heard: [...bridge.listening.keys()],
  };
}

describe('the bridge of the main window', () => {
  it('offers the calls that the page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbina']);
    expect(Object.keys(exposed.columbina).sort()).toEqual(declared('src/bridge.d.ts', 'ColumbinaBridge').sort());
    expect(Object.keys(exposed.columbina).length).toBeGreaterThan(15);
  });

  it('asks the shell for everything that the shell answers to this window, and for nothing else', () => {
    const { asked, said, heard } = reached('electron/preload.cjs', 'columbina');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina:')).sort());
    // A line for the service is said and not asked: the shell gives no answer to it.
    expect(said).toEqual(taken);
    expect(taken).toEqual(['columbina:send']);
    // And it listens to everything that the shell tells of.
    expect([...heard].sort()).toEqual([...told].sort());
    expect(told.length).toBe(6);
  });

  it('hands on what a call names, and nothing else that came with it', () => {
    const { exposed, invoked } = bridgeOf('electron/preload.cjs');
    const more = { language: 'ja', cwd: 'C:/work', title: 'a title', body: 'a body', service: 'deepseek', api_key: 'never', then: 'more' };
    const handed = (call: string, ...given: unknown[]) => {
      exposed.columbina[call](...given);
      return invoked.at(-1);
    };
    expect(handed('selectFolder', more)).toEqual(['columbina:select-folder', { language: 'ja' }]);
    expect(handed('start', more)).toEqual(['columbina:start', { cwd: 'C:/work' }]);
    expect(handed('notify', more)).toEqual(['columbina:notify', { title: 'a title', body: 'a body' }]);
    expect(handed('setUpService', more)).toEqual(['columbina:set-up-service', { service: 'deepseek', language: 'ja' }]);
    expect(handed('setUpFeishu', more)).toEqual(['columbina:set-up-feishu', { language: 'ja' }]);
    expect(handed('feishuStart', more)).toEqual(['columbina:feishu-start', { language: 'ja' }]);
    expect(handed('chooseModel', { ...more, model: 'deepseek-pro' }))
      .toEqual(['columbina:choose-model', { service: 'deepseek', model: 'deepseek-pro' }]);
    expect(handed('chooseEffort', { ...more, effort: 'max' }))
      .toEqual(['columbina:choose-effort', { service: 'deepseek', effort: 'max' }]);
    expect(handed('listModels', 'deepseek', more)).toEqual(['columbina:models', 'deepseek']);
    expect(handed('chooseAttachments', more)).toEqual(['columbina:choose-attachments', { language: 'ja' }]);
    expect(handed('forgetAttachments', ['attachment-1', 7, 'attachment-2'], more))
      .toEqual(['columbina:forget-attachments', ['attachment-1', 'attachment-2']]);
    expect(handed('keepAttachments', ['attachment-1', { then: 'more' }])).toEqual(['columbina:keep-attachments', ['attachment-1']]);
    expect(handed('keepAttachments', 'attachment-1')).toEqual(['columbina:keep-attachments', []]);
    // Asked now only where the page says so in so many words.
    expect(handed('updates', { ...more, now: true })).toEqual(['columbina:updates', { now: true }]);
    for (const asked of [{ now: 'yes' }, { now: 1 }, {}, undefined]) expect(handed('updates', asked)).toEqual(['columbina:updates', { now: false }]);
    // A call that is made with nothing asks all the same, and the shell says what is missing.
    for (const call of ['selectFolder', 'start', 'notify', 'setUpService', 'setUpFeishu', 'feishuStart', 'chooseModel', 'chooseEffort']) {
      const [, given] = handed(call) as [string, Record<string, unknown>];
      expect(Object.values(given).every(value => value === undefined), call).toBe(true);
    }
    // What needs no word of the page is asked for without one.
    for (const call of [
      'getLocalMusic', 'stop', 'serviceTail', 'showLog', 'recentProjects', 'services', 'about',
      'feishuStatus', 'feishuStop', 'feishuForget', 'feishuLog',
    ]) {
      expect(handed(call, more), call).toHaveLength(1);
    }
  });

  it('names a file that was dropped by where Electron says it lies, and nothing that the page wrote', () => {
    const { exposed, invoked } = bridgeOf('electron/preload.cjs');
    exposed.columbina.attachDropped([
      { name: 'notes.md', at: 'C:\\work\\notes.md' },
      // A file that the page made itself lies nowhere.
      { name: 'made-up.txt' },
      // A path that the page wrote is no file.
      'C:\\Users\\me\\.ssh\\id_rsa',
      { name: 'photo.png', at: '/Users/me/photo.png' },
    ]);
    expect(invoked.at(-1)).toEqual(['columbina:attach-paths', ['C:\\work\\notes.md', '/Users/me/photo.png']]);
    exposed.columbina.attachDropped(undefined);
    expect(invoked.at(-1)).toEqual(['columbina:attach-paths', []]);
  });

  it('tells the page which system it runs on, and asks the shell nothing for that', () => {
    for (const platform of ['win32', 'darwin', 'linux']) {
      const { exposed, invoked, sent } = bridgeOf('electron/preload.cjs', platform);
      expect(exposed.columbina.platform as unknown).toBe(platform);
      expect([...invoked, ...sent]).toEqual([]);
    }
  });

  it('gives the page the answer of the shell', async () => {
    const { exposed } = bridgeOf('electron/preload.cjs');
    await expect(exposed.columbina.selectFolder({ language: 'en' })).resolves.toBe('what the shell answers');
  });

  it('lets the page listen to the shell, and stop listening', () => {
    const { exposed, listening } = bridgeOf('electron/preload.cjs');
    const heard: unknown[][] = [];
    const stop = exposed.columbina.onLine((...given: unknown[]) => heard.push(given));
    const others = exposed.columbina.onClose(() => heard.push(['closed']));
    for (const listener of listening.get('columbina:line')!) listener({ sender: 'the shell' }, 'a line');
    // The page is given what was said, and not the event that Electron wraps it in.
    expect(heard).toEqual([['a line']]);
    stop();
    expect(listening.get('columbina:line')!.size).toBe(0);
    expect(listening.get('columbina:closed')!.size).toBe(1);
    others();
    expect(listening.get('columbina:closed')!.size).toBe(0);
  });
});

describe('the bridge of the window for the key', () => {
  it('offers the calls that its page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/key-preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbinaKey']);
    expect(Object.keys(exposed.columbinaKey).sort()).toEqual(declared('src/key/bridge.d.ts', 'KeyBridge').sort());
  });

  it('asks the shell for everything that the shell answers to this window, and listens to nothing', () => {
    const { asked, said, heard } = reached('electron/key-preload.cjs', 'columbinaKey');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina-key:')).sort());
    expect(said).toEqual([]);
    expect(heard).toEqual([]);
  });

  it('hands the shell what is to be stored as it is given', () => {
    // The shell cuts it down to what the program reads, and is tested for that.
    const { exposed, invoked } = bridgeOf('electron/key-preload.cjs');
    const document = { service: 'deepseek', model: 'deepseek-flash', api_key: 'made up' };
    exposed.columbinaKey.store(document);
    expect(invoked.at(-1)).toEqual(['columbina-key:store', document]);
  });
});

describe('the bridge of the window for the app of Feishu', () => {
  it('offers the calls that its page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/feishu-preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbinaFeishu']);
    expect(Object.keys(exposed.columbinaFeishu).sort()).toEqual(declared('src/feishu/bridge.d.ts', 'FeishuBridge').sort());
  });

  it('asks the shell for everything that the shell answers to this window, and listens to nothing', () => {
    const { asked, said, heard } = reached('electron/feishu-preload.cjs', 'columbinaFeishu');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina-feishu:')).sort());
    expect(said).toEqual([]);
    expect(heard).toEqual([]);
  });

  it('hands the shell what is to be stored as it is given', () => {
    // The shell cuts it down to what the program reads, and is tested for that.
    const { exposed, invoked } = bridgeOf('electron/feishu-preload.cjs');
    const document = { domain: 'feishu', app_id: 'cli_a1b2c3d4', app_secret: 'made up' };
    exposed.columbinaFeishu.store(document);
    expect(invoked.at(-1)).toEqual(['columbina-feishu:store', document]);
  });
});

describe('the three bridges', () => {
  it('reach every call that the shell answers between them', () => {
    const main = reached('electron/preload.cjs', 'columbina');
    const key = reached('electron/key-preload.cjs', 'columbinaKey');
    const feishu = reached('electron/feishu-preload.cjs', 'columbinaFeishu');
    expect([...main.asked, ...key.asked, ...feishu.asked].sort()).toEqual([...answered].sort());
    expect([...main.said, ...key.said, ...feishu.said]).toEqual(taken);
    expect(main.asked.filter(channel => key.asked.includes(channel) || feishu.asked.includes(channel))).toEqual([]);
    expect(key.asked.filter(channel => feishu.asked.includes(channel))).toEqual([]);
  });
});
