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
/** The assistants that the shell runs, by the names of their commands, which name their channels too. */
const kinds = [...shell.matchAll(/^ {2}(\w+): assistantOf\('(\w+)'/gm)].map(([, key, kind]) => (key === kind ? kind : `${key}?`));
/** A channel that the shell names, or the channels of each assistant where it names them by the assistant. */
const channels = (named: string) => (/\$\{(?:one\.)?kind\}/.test(named)
  ? kinds.map(kind => named.replace(/\$\{(?:one\.)?kind\}/g, kind)) : [named]);
/** The assistants whose agents change no file: the shell answers them no question of it. */
const unchanging = [...shell.matchAll(/^ {2}(\w+): assistantOf\([^)]*, false\),$/gm)].map(found => found[1]);
/** What the shell answers, what it takes without an answer, and what it tells a page without being asked. */
const answered = [...shell.matchAll(/ipcMain\.handle\((['`])([^'`]+)\1/g)]
  .flatMap(found => channels(found[2]))
  .filter(channel => !unchanging.some(kind => channel === `columbina:${kind}-writable`));
const taken = [...shell.matchAll(/ipcMain\.on\('([^']+)'/g)].map(found => found[1]);
const told = [...new Set([...shell.matchAll(/\btell\((['`])([^'`]+)\1/g)].flatMap(found => channels(found[2])))];

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

describe('the channels of the assistants', () => {
  it('are those of the ones in Feishu, in WeCom and in QQ', () => {
    expect(kinds).toEqual(['feishu', 'wecom', 'qq']);
    expect(answered.filter(channel => channel.startsWith('columbina-wecom:')).sort())
      .toEqual(['columbina-wecom:close', 'columbina-wecom:context', 'columbina-wecom:open-page', 'columbina-wecom:store']);
    expect(answered).toContain('columbina:set-up-wecom');
    expect(told).toContain('columbina:wecom');
    expect(answered.filter(channel => channel.startsWith('columbina-qq:')).sort())
      .toEqual(['columbina-qq:close', 'columbina-qq:context', 'columbina-qq:open-page', 'columbina-qq:store']);
    expect(answered).toContain('columbina:set-up-qq');
    expect(told).toContain('columbina:qq');
    // Its agents change no file, and nothing asks whether they may.
    expect(unchanging).toEqual(['qq']);
    expect(answered).not.toContain('columbina:qq-writable');
  });
});

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
    expect(told.length).toBe(9);
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
    expect(handed('startChat', more)).toEqual(['columbina:start-chat']);
    expect(handed('notify', more)).toEqual(['columbina:notify', { title: 'a title', body: 'a body' }]);
    expect(handed('setUpService', more)).toEqual(['columbina:set-up-service', { service: 'deepseek', language: 'ja' }]);
    expect(handed('setUpFeishu', more)).toEqual(['columbina:set-up-feishu', { language: 'ja' }]);
    expect(handed('feishuStart', more)).toEqual(['columbina:feishu-start', { language: 'ja' }]);
    expect(handed('setUpWecom', more)).toEqual(['columbina:set-up-wecom', { language: 'ja' }]);
    expect(handed('wecomStart', more)).toEqual(['columbina:wecom-start', { language: 'ja' }]);
    expect(handed('setUpQq', more)).toEqual(['columbina:set-up-qq', { language: 'ja' }]);
    expect(handed('qqStart', more)).toEqual(['columbina:qq-start', { language: 'ja' }]);
    expect(handed('chooseModel', { ...more, model: 'deepseek-pro' }))
      .toEqual(['columbina:choose-model', { service: 'deepseek', model: 'deepseek-pro' }]);
    expect(handed('chooseEffort', { ...more, effort: 'max' }))
      .toEqual(['columbina:choose-effort', { service: 'deepseek', effort: 'max' }]);
    expect(handed('listModels', 'deepseek', more)).toEqual(['columbina:models', 'deepseek']);
    expect(handed('balance', 'deepseek', more)).toEqual(['columbina:balance', 'deepseek']);
    expect(handed('setReasoningLanguage', 'zh', more)).toEqual(['columbina:reasoning-language', 'zh']);
    expect(handed('chooseAttachments', more)).toEqual(['columbina:choose-attachments', { language: 'ja' }]);
    expect(handed('forgetAttachments', ['attachment-1', 7, 'attachment-2'], more))
      .toEqual(['columbina:forget-attachments', ['attachment-1', 'attachment-2']]);
    expect(handed('keepAttachments', ['attachment-1', { then: 'more' }])).toEqual(['columbina:keep-attachments', ['attachment-1']]);
    expect(handed('keepAttachments', 'attachment-1')).toEqual(['columbina:keep-attachments', []]);
    const bytes = new ArrayBuffer(4);
    expect(handed('attachPasted', [{ type: 'image/png', bytes, name: 'more' }], more))
      .toEqual(['columbina:attach-pasted', [{ type: 'image/png', bytes }]]);
    expect(handed('attachPasted', 'not pictures')).toEqual(['columbina:attach-pasted', []]);
    expect(handed('schedules', { what: 'add', id: 'x', task: { name: 'n', every: 'day' }, ...more }))
      .toEqual(['columbina:schedules', { what: 'add', id: 'x', task: { name: 'n', every: 'day' } }]);
    expect(handed('worktrees', { what: 'add', name: 'task', force: 'yes', ...more }))
      .toEqual(['columbina:worktrees', { what: 'add', cwd: 'C:/work', name: 'task', force: false }]);
    expect(handed('qqGroups', { what: 'name', id: 'G1', name: 'Home', ...more }))
      .toEqual(['columbina:qq-groups', { what: 'name', id: 'G1', name: 'Home' }]);
    // Asked now only where the page says so in so many words.
    expect(handed('updates', { ...more, now: true })).toEqual(['columbina:updates', { now: true }]);
    for (const asked of [{ now: 'yes' }, { now: 1 }, {}, undefined]) expect(handed('updates', asked)).toEqual(['columbina:updates', { now: false }]);
    // On only where the page says so in so many words.
    expect(handed('feishuWritable', true)).toEqual(['columbina:feishu-writable', true]);
    expect(handed('wecomWritable', true)).toEqual(['columbina:wecom-writable', true]);
    expect(handed('qqThinking', { model: 'm', effort: 'off', more: 1 })).toEqual(['columbina:qq-thinking', { model: 'm', effort: 'off' }]);
    expect(handed('feishuThinking')).toEqual(['columbina:feishu-thinking', { model: null, effort: null }]);
    expect(handed('wecomThinking', { effort: 'low' })).toEqual(['columbina:wecom-thinking', { model: null, effort: 'low' }]);
    expect(handed('setLoginItem', true)).toEqual(['columbina:set-login-item', true]);
    expect(handed('setPet', { on: true, chosen: 'builtin/cat', share: false, more: 1 }))
      .toEqual(['columbina:set-pet', { on: true, chosen: 'builtin/cat' }]);
    expect(handed('setPet')).toEqual(['columbina:set-pet', { on: undefined, chosen: undefined }]);
    for (const asked of ['true', 1, {}, undefined]) {
      expect(handed('feishuWritable', asked)).toEqual(['columbina:feishu-writable', false]);
      expect(handed('wecomWritable', asked)).toEqual(['columbina:wecom-writable', false]);
      expect(handed('setLoginItem', asked)).toEqual(['columbina:set-login-item', false]);
    }
    // A call that is made with nothing asks all the same, and the shell says what is missing.
    for (const call of [
      'selectFolder', 'start', 'notify', 'setUpService', 'setUpFeishu', 'feishuStart', 'setUpWecom', 'wecomStart', 'chooseModel',
      'chooseEffort', 'setUpQq', 'qqStart',
    ]) {
      const [, given] = handed(call) as [string, Record<string, unknown>];
      expect(Object.values(given).every(value => value === undefined), call).toBe(true);
    }
    // What needs no word of the page is asked for without one.
    for (const call of [
      'getLocalMusic', 'stop', 'serviceTail', 'showLog', 'recentProjects', 'services', 'about',
      'feishuStatus', 'feishuStop', 'feishuForget', 'feishuLog', 'openFeishuFolder',
      'wecomStatus', 'wecomStop', 'wecomForget', 'wecomLog', 'openWecomFolder',
      'qqStatus', 'qqStop', 'qqForget', 'qqLog', 'openQqFolder', 'pets', 'openPetsFolder',
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

describe('the bridge of the window for the bot of WeCom', () => {
  it('offers the calls that its page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/wecom-preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbinaWecom']);
    expect(Object.keys(exposed.columbinaWecom).sort()).toEqual(declared('src/wecom/bridge.d.ts', 'WecomBridge').sort());
  });

  it('asks the shell for everything that the shell answers to this window, and listens to nothing', () => {
    const { asked, said, heard } = reached('electron/wecom-preload.cjs', 'columbinaWecom');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina-wecom:')).sort());
    expect(said).toEqual([]);
    expect(heard).toEqual([]);
  });

  it('hands the shell what is to be stored as it is given', () => {
    const { exposed, invoked } = bridgeOf('electron/wecom-preload.cjs');
    const document = { bot_id: 'aibTestBot0123', secret: 'made up' };
    exposed.columbinaWecom.store(document);
    expect(invoked.at(-1)).toEqual(['columbina-wecom:store', document]);
  });
});

describe('the bridge of the window for the bot of QQ', () => {
  it('offers the calls that its page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/qq-preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbinaQq']);
    expect(Object.keys(exposed.columbinaQq).sort()).toEqual(declared('src/qq/bridge.d.ts', 'QqBridge').sort());
  });

  it('asks the shell for everything that the shell answers to this window, and listens to nothing', () => {
    const { asked, said, heard } = reached('electron/qq-preload.cjs', 'columbinaQq');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina-qq:')).sort());
    expect(said).toEqual([]);
    expect(heard).toEqual([]);
  });

  it('hands the shell what is to be stored as it is given', () => {
    const { exposed, invoked } = bridgeOf('electron/qq-preload.cjs');
    const document = { app_id: 'aibTestBot0123', secret: 'made up' };
    exposed.columbinaQq.store(document);
    expect(invoked.at(-1)).toEqual(['columbina-qq:store', document]);
  });
});

describe('the bridge of the window of the pet', () => {
  it('offers the calls that the page is told of, and no other', () => {
    const { exposed } = bridgeOf('electron/pet-preload.cjs');
    expect(Object.keys(exposed)).toEqual(['columbinaPet']);
    expect(Object.keys(exposed.columbinaPet).sort()).toEqual(declared('src/pet/bridge.d.ts', 'PetBridge').sort());
  });

  it('asks the shell for what it answers to the pet, and hears what it tells the pet', () => {
    const { asked, said, heard } = reached('electron/pet-preload.cjs', 'columbinaPet');
    expect([...asked].sort()).toEqual(answered.filter(channel => channel.startsWith('columbina-pet:')).sort());
    expect(said).toEqual([]);
    expect([...heard].sort()).toEqual(['columbina-pet:sheet', 'columbina-pet:shown']);
    const told = [...shell.matchAll(/\btellPet\((['`])([^'`]+)\1/g)].map(found => found[2]);
    expect([...new Set(told)].sort()).toEqual(['columbina-pet:sheet', 'columbina-pet:shown']);
  });

  it('hands on the phase of a drag', () => {
    const pet = bridgeOf('electron/pet-preload.cjs');
    pet.exposed.columbinaPet.drag('move');
    expect(pet.invoked.at(-1)).toEqual(['columbina-pet:drag', 'move']);
  });
});

describe('the six bridges', () => {
  it('reach every call that the shell answers between them', () => {
    const main = reached('electron/preload.cjs', 'columbina');
    const key = reached('electron/key-preload.cjs', 'columbinaKey');
    const feishu = reached('electron/feishu-preload.cjs', 'columbinaFeishu');
    const wecom = reached('electron/wecom-preload.cjs', 'columbinaWecom');
    const qq = reached('electron/qq-preload.cjs', 'columbinaQq');
    const pet = reached('electron/pet-preload.cjs', 'columbinaPet');
    const all = [main, key, feishu, wecom, qq, pet];
    expect(all.flatMap(bridge => bridge.asked).sort()).toEqual([...answered].sort());
    expect(all.flatMap(bridge => bridge.said)).toEqual(taken);
    // No channel is asked by two of them.
    const asked = all.flatMap(bridge => [...new Set(bridge.asked)]);
    expect(asked.filter((channel, index) => asked.indexOf(channel) !== index)).toEqual([]);
  });
});
