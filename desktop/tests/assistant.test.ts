/**
 * What the page makes of the assistants in Feishu and in WeCom, and what the windows for the app
 * and for the bot make of what is typed: tried with plain values.
 */
import { describe, expect, it } from 'vitest';
import { assistantAfter, assistantOf, doingOf, endedOf, type App, type Assistant } from '../src/app/assistant';
import { appIdFits, blankInside, complete, CONSOLES, documentOf, formOf, hostOf, OTHER, SCOPES, storedOf } from '../src/feishu/form';
import * as wecom from '../src/wecom/form';
import * as qq from '../src/qq/form';

const status = {
  set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
  running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
};
const app = (given: Partial<App> = {}): Extract<Assistant, { kind: 'app' }> => ({
  kind: 'app', platform: 'feishu', id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
  folder: null, writable: false, running: false, own: false, state: null, code: null, error: null, ended: null, ...given,
});

describe('the folder of the assistant, as the shell says it', () => {
  it('is named, with whether its agent may change the files there', () => {
    expect(assistantOf('feishu', { ...status, folder: 'C:/me/Columbina/feishu', writable: true }))
      .toMatchObject({ folder: 'C:/me/Columbina/feishu', writable: true });
    // Where nothing that could be one is said, there is none, and nothing may be changed.
    expect(assistantOf('feishu', { ...status, folder: 7, writable: 'yes' })).toMatchObject({ folder: null, writable: false });
    expect(assistantOf('feishu', status)).toMatchObject({ folder: null, writable: false });
  });
});

describe('how the assistant is doing, as the shell says it', () => {
  it('is not there to ask where there is no service program', () => {
    expect(assistantOf('feishu', null)).toEqual({ kind: 'unavailable' });
    expect(assistantOf('feishu', 'no answer')).toEqual({ kind: 'unavailable' });
  });

  it('could not be read where the program says why', () => {
    expect(assistantOf('feishu', { error: 'the file of the app cannot be read', code: 'STORE' }))
      .toEqual({ kind: 'failed', said: 'the file of the app cannot be read' });
  });

  it('has no app where none is set up, and says how the last one ended badly', () => {
    expect(assistantOf('feishu', { set_up: false, running: false, own: false, ended: null })).toEqual({ kind: 'none', ended: null });
    expect(assistantOf('feishu', { set_up: false, ended: { code: 2, said: 'no app of Feishu is set up' } }))
      .toEqual({ kind: 'none', ended: { code: 2, said: 'no app of Feishu is set up' } });
  });

  it('is an app that does not run', () => {
    expect(assistantOf('feishu', status)).toEqual(app());
  });

  it('is an app in Lark, with the console of Lark, whatever the program named', () => {
    expect(assistantOf('feishu', { ...status, domain: 'lark', console: 'https://elsewhere.example/app' }))
      .toEqual(app({ platform: 'lark', console: 'https://open.larksuite.com/app' }));
  });

  it('runs elsewhere where the program says so and the application runs none', () => {
    expect(assistantOf('feishu', { ...status, running: true, state: 'connected', paired: true }))
      .toEqual(app({ running: true, state: 'connected', paired: true }));
  });

  it('runs here from the moment that the application started it, before it said anything', () => {
    expect(assistantOf('feishu', { ...status, own: true, ended: { code: 1, said: 'an old end' } }))
      .toEqual(app({ running: true, own: true }));
  });

  it('shows a code of pairing only while it runs, and only one of six digits', () => {
    expect(assistantOf('feishu', { ...status, own: true, code: '123456' }).kind === 'app'
      && (assistantOf('feishu', { ...status, own: true, code: '123456' }) as App).code).toBe('123456');
    for (const code of ['12345', '1234567', 'abcdef', 123456, null]) {
      expect((assistantOf('feishu', { ...status, own: true, code }) as App).code).toBeNull();
    }
    expect((assistantOf('feishu', { ...status, code: '123456' }) as App).code).toBeNull();
  });

  it('failed where the last one failed, also after it ended', () => {
    expect(assistantOf('feishu', { ...status, state: 'failed', error: 'Feishu refuses the app' }))
      .toEqual(app({ state: 'failed', error: 'Feishu refuses the app' }));
  });

  it('is stopping where the program said that it stopped and it has not ended yet', () => {
    expect((assistantOf('feishu', { ...status, own: true, state: 'stopped' }) as App).state).toBe('stopping');
    expect((assistantOf('feishu', { ...status, state: 'stopped' }) as App).state).toBeNull();
    expect((assistantOf('feishu', { ...status, own: true, state: 'made up' }) as App).state).toBeNull();
  });
});

describe('what the assistant that the application runs says of itself', () => {
  it('belongs to no app where the page knows of none', () => {
    for (const current of [{ kind: 'unknown' }, { kind: 'unavailable' }, { kind: 'none', ended: null }] as Assistant[]) {
      expect(assistantAfter('feishu', current, { state: 'connected' })).toBeNull();
    }
  });

  it('says how it is doing', () => {
    const before = app({ ended: { code: 1, said: 'an old end' } });
    expect(assistantAfter('feishu', before, { state: 'connecting', paired: false, code: '123456', error: null }))
      .toEqual(app({ running: true, own: true, state: 'connecting', code: '123456' }));
    expect(assistantAfter('feishu', before, { state: 'connecting', paired: true, code: null, error: 'the connection was closed' }))
      .toEqual(app({ running: true, own: true, state: 'connecting', paired: true, error: 'the connection was closed' }));
    expect(assistantAfter('feishu', before, { state: 'stopped', paired: true }))
      .toEqual(app({ running: true, own: true, state: 'stopping', paired: true }));
    expect(assistantAfter('feishu', before, { state: 'failed', paired: false, error: 'Feishu refuses the app' }))
      .toEqual(app({ running: true, own: true, state: 'failed', error: 'Feishu refuses the app' }));
  });

  it('belongs to another app where it names another', () => {
    const running = app({ running: true, own: true, state: 'connected' });
    expect(assistantAfter('feishu', running, { state: 'connected', app_id: 'cli_other123', domain: 'feishu' })).toBeNull();
    expect(assistantAfter('feishu', running, { state: 'connected', app_id: 'cli_a1b2c3d4', domain: 'lark' })).toBeNull();
    expect(assistantAfter('feishu', running, { state: 'connected', app_id: 'cli_a1b2c3d4', domain: 'feishu' }))
      .toEqual(app({ running: true, own: true, state: 'connected' }));
    // A line that names no app is of the one that the page knows of.
    expect(assistantAfter('feishu', running, { state: 'connected', domain: 'elsewhere' })).toEqual(app({ running: true, own: true, state: 'connected' }));
  });

  it('ended, and leaves nothing to say where it was stopped', () => {
    const running = app({ running: true, own: true, state: 'connected', code: '123456', error: 'old' });
    expect(assistantAfter('feishu', running, { ended: true, code: 0, said: 'stopped' })).toEqual(app());
  });

  it('ended badly, and says the last line that it wrote', () => {
    const running = app({ running: true, own: true, state: 'connecting' });
    expect(assistantAfter('feishu', running, { ended: true, code: 2, said: 'an assistant in Feishu runs on this machine already' }))
      .toEqual(app({ ended: { code: 2, said: 'an assistant in Feishu runs on this machine already' } }));
  });

  it('ended after it failed, and keeps why it failed', () => {
    const failed = app({ running: true, own: true, state: 'failed', error: 'Feishu refuses the app', paired: true });
    expect(assistantAfter('feishu', failed, { ended: true, code: 1, said: 'Feishu refuses the app' })).toEqual(app({
      state: 'failed', error: 'Feishu refuses the app', paired: true, ended: { code: 1, said: 'Feishu refuses the app' },
    }));
  });
});

describe('the assistant in WeCom', () => {
  const bot = {
    set_up: true, bot_id: 'aibTestBot0123', paired: false, folder: 'C:/me/Columbina/wecom', writable: false,
    running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
  };
  const ofBot = (given: Partial<App> = {}) => app({
    platform: 'wecom', id: 'aibTestBot0123', console: 'https://work.weixin.qq.com/wework_admin/frame',
    folder: 'C:/me/Columbina/wecom', ...given,
  });

  it('is a bot, named by its Bot ID, with the admin console of WeCom', () => {
    expect(assistantOf('wecom', bot)).toEqual(ofBot());
    expect(assistantOf('wecom', { set_up: false, running: false, folder: 'C:/me/Columbina/wecom', ended: null }))
      .toEqual({ kind: 'none', ended: null });
  });

  it('goes by its own names only: neither a platform nor an App ID makes it another', () => {
    expect(assistantOf('wecom', { ...bot, domain: 'lark', app_id: 'cli_a1b2c3d4' })).toEqual(ofBot());
    // And the assistant in Feishu is not named by a Bot ID.
    expect(assistantOf('feishu', { ...status, bot_id: 'aibTestBot0123' })).toEqual(app());
  });

  it('runs, and says how it is doing, line by line', () => {
    const running = assistantOf('wecom', { ...bot, own: true, code: '123456' });
    expect(running).toEqual(ofBot({ running: true, own: true, code: '123456' }));
    expect(assistantAfter('wecom', running, { state: 'connected', paired: true, code: null, error: null, bot_id: 'aibTestBot0123' }))
      .toEqual(ofBot({ running: true, own: true, state: 'connected', paired: true }));
    expect(assistantAfter('wecom', running, { ended: true, code: 1, said: 'WeCom refused the bot' }))
      .toEqual(ofBot({ ended: { code: 1, said: 'WeCom refused the bot' } }));
  });

  it('belongs to another bot where a line names another, and not where it names an app', () => {
    const running = ofBot({ running: true, own: true, state: 'connected' });
    expect(assistantAfter('wecom', running, { state: 'connected', bot_id: 'aibOtherBot99' })).toBeNull();
    expect(assistantAfter('wecom', running, { state: 'connected', app_id: 'cli_a1b2c3d4' }))
      .toEqual(ofBot({ running: true, own: true, state: 'connected' }));
    // A line of the assistant in Feishu that names a bot is still of the app that the page knows of.
    expect(assistantAfter('feishu', app({ running: true, own: true }), { state: 'connected', bot_id: 'aibOtherBot99' }))
      .toEqual(app({ running: true, own: true, state: 'connected' }));
  });
});

describe('how an assistant ended', () => {
  it('is nothing where nothing is said of an end', () => {
    expect(endedOf(null)).toBeNull();
    expect(endedOf({})).toBeNull();
    expect(endedOf('ended')).toBeNull();
  });

  it('is its code, or the signal that ended it, and the last line that it wrote', () => {
    expect(endedOf({ code: 2, said: 'no app' })).toEqual({ code: 2, said: 'no app' });
    expect(endedOf({ code: 'SIGKILL' })).toEqual({ code: 'SIGKILL', said: '' });
    expect(endedOf({ code: null, said: 'spawn ENOENT' })).toEqual({ code: null, said: 'spawn ENOENT' });
    expect(endedOf({ said: 7 })).toEqual({ code: null, said: '' });
  });
});

describe('what the panel says of an assistant that runs', () => {
  it('says that it runs elsewhere, where the application does not run it', () => {
    expect(doingOf(app({ running: true, state: 'connected' }))).toBe('elsewhere');
  });

  it('says how the assistant that the application runs is doing', () => {
    const own = (given: Partial<App>) => doingOf(app({ running: true, own: true, ...given }));
    expect(own({ state: null })).toBe('connecting');
    expect(own({ state: 'connecting' })).toBe('connecting');
    expect(own({ state: 'connecting', error: 'the connection was closed' })).toBe('reconnecting');
    expect(own({ state: 'stopping', error: 'the connection was closed' })).toBe('stopping');
    expect(own({ state: 'connected', code: '123456' })).toBe('pair');
    expect(own({ state: 'connected' })).toBe('noCode');
    expect(own({ state: 'connected', paired: true, code: '123456' })).toBe('connected');
  });
});

describe('the window for the app', () => {
  it('opens with the app that is set up, or empty on Feishu', () => {
    expect(formOf({ set_up: true, domain: 'lark', app_id: 'cli_a1b2c3d4' }))
      .toEqual({ form: { platform: 'lark', appId: 'cli_a1b2c3d4', secret: '' }, setUp: true });
    expect(formOf({ set_up: false, running: false })).toEqual({ form: { platform: 'feishu', appId: '', secret: '' }, setUp: false });
    expect(formOf(null)).toEqual({ form: { platform: 'feishu', appId: '', secret: '' }, setUp: false });
    expect(formOf({ set_up: true, app_id: 7 })).toEqual({ form: { platform: 'feishu', appId: '', secret: '' }, setUp: false });
  });

  it('takes an App ID as the program does', () => {
    for (const fits of ['cli_a1b2', 'cli_a1b2c3d4e5f6g7h8', ' cli_a1b2c3d4 ', `cli_${'a'.repeat(60)}`]) expect(appIdFits(fits), fits).toBe(true);
    for (const wrong of ['', 'cli_', 'cli_a1b', 'a1b2c3d4', 'CLI_a1b2c3d4', 'cli_a1b2-c3d4', 'cli_a1b2 c3d4', `cli_${'a'.repeat(61)}`]) {
      expect(appIdFits(wrong), wrong).toBe(false);
    }
  });

  it('sees a blank inside an App Secret, and none around it', () => {
    expect(blankInside('  s3cr3t\n')).toBe(false);
    expect(blankInside('s3cr3t more')).toBe(true);
    expect(blankInside('s3cr3t\nApp Secret')).toBe(true);
  });

  it('is complete with an App ID and an App Secret without a blank inside', () => {
    const form = { platform: 'feishu' as const, appId: 'cli_a1b2c3d4', secret: 's3cr3t' };
    expect(complete(form)).toBe(true);
    expect(complete({ ...form, secret: '   ' })).toBe(false);
    expect(complete({ ...form, secret: 'two halves' })).toBe(false);
    expect(complete({ ...form, appId: 'cli_' })).toBe(false);
  });

  it('hands the program the app without the blanks around what was typed', () => {
    expect(documentOf({ platform: 'lark', appId: ' cli_a1b2c3d4 ', secret: '\ts3cr3t\n' }))
      .toEqual({ domain: 'lark', app_id: 'cli_a1b2c3d4', app_secret: 's3cr3t' });
  });

  it('reads what the program answered', () => {
    expect(storedOf({ stored: { domain: 'feishu', app_id: 'cli_a1b2c3d4' }, secret_in: 'keyring' }))
      .toEqual({ kind: 'stored', appId: 'cli_a1b2c3d4', secretIn: 'keyring' });
    expect(storedOf({ stored: { app_id: 'cli_a1b2c3d4' }, secret_in: 'file' }))
      .toEqual({ kind: 'stored', appId: 'cli_a1b2c3d4', secretIn: 'file' });
    expect(storedOf({ stored: { app_id: 'cli_a1b2c3d4' }, secret_in: 'elsewhere' }))
      .toEqual({ kind: 'stored', appId: 'cli_a1b2c3d4', secretIn: null });
    expect(storedOf({ error: 'Feishu does not take the app', code: 'REFUSED' }))
      .toEqual({ kind: 'refused', said: 'Feishu does not take the app', elsewhere: false });
    // An app of the other platform is told apart by the code, whatever the words.
    expect(storedOf({ error: 'Feishu does not know this app', code: 'WRONG_PLATFORM' }))
      .toEqual({ kind: 'refused', said: 'Feishu does not know this app', elsewhere: true });
    expect(storedOf({ error: 'WRONG_PLATFORM', code: 'REFUSED' }))
      .toEqual({ kind: 'refused', said: 'WRONG_PLATFORM', elsewhere: false });
    expect(storedOf({ stored: { app_id: 7 } })).toEqual({ kind: 'refused', said: '', elsewhere: false });
    expect(storedOf(null)).toEqual({ kind: 'refused', said: '', elsewhere: false });
  });

  it('opens the consoles that the shell opens, and names their machines', () => {
    expect(OTHER).toEqual({ feishu: 'lark', lark: 'feishu' });
    expect(CONSOLES).toEqual({ feishu: 'https://open.feishu.cn/app', lark: 'https://open.larksuite.com/app' });
    expect(hostOf(CONSOLES.lark)).toBe('open.larksuite.com');
    expect(hostOf('no address')).toBe('no address');
  });

  it('asks for the permissions that the assistant uses, and those for documents apart', () => {
    expect(SCOPES.messages).toEqual([
      'im:message.p2p_msg:readonly', 'im:message:send_as_bot', 'im:message.reactions:write_only', 'im:message:readonly',
    ]);
    expect(SCOPES.documents).toEqual(['docx:document:readonly', 'wiki:wiki:readonly', 'sheets:spreadsheet:readonly']);
  });
});

describe('the window for the bot', () => {
  it('opens with the bot that is set up, or empty', () => {
    expect(wecom.formOf({ set_up: true, bot_id: 'aibTestBot0123' }))
      .toEqual({ form: { botId: 'aibTestBot0123', secret: '' }, setUp: true });
    expect(wecom.formOf({ set_up: false, running: false })).toEqual({ form: { botId: '', secret: '' }, setUp: false });
    expect(wecom.formOf(null)).toEqual({ form: { botId: '', secret: '' }, setUp: false });
    expect(wecom.formOf({ set_up: true, bot_id: 7 })).toEqual({ form: { botId: '', secret: '' }, setUp: false });
  });

  it('takes a Bot ID as the program does', () => {
    for (const fits of ['a', 'aibTestBot0123', ' aibTestBot0123 ', 'x'.repeat(128), 'bot-1_2.3~!']) {
      expect(wecom.botIdFits(fits), fits).toBe(true);
    }
    for (const wrong of ['', '   ', 'x'.repeat(129), 'aib Test', 'aib\tTest', 'aib机器人']) {
      expect(wecom.botIdFits(wrong), wrong).toBe(false);
    }
  });

  it('is complete with a Bot ID and a Secret without a blank inside', () => {
    const form = { botId: 'aibTestBot0123', secret: 's3cr3t' };
    expect(wecom.complete(form)).toBe(true);
    expect(wecom.complete({ ...form, secret: '  s3cr3t\n' })).toBe(true);
    expect(wecom.complete({ ...form, secret: '   ' })).toBe(false);
    expect(wecom.complete({ ...form, secret: 'two halves' })).toBe(false);
    expect(wecom.complete({ ...form, botId: '' })).toBe(false);
  });

  it('hands the program the bot without the blanks around what was typed', () => {
    expect(wecom.documentOf({ botId: ' aibTestBot0123 ', secret: '\ts3cr3t\n' }))
      .toEqual({ bot_id: 'aibTestBot0123', secret: 's3cr3t' });
  });

  it('reads what the program answered', () => {
    expect(wecom.storedOf({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'keyring' }))
      .toEqual({ kind: 'stored', botId: 'aibTestBot0123', secretIn: 'keyring' });
    expect(wecom.storedOf({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'file' }))
      .toEqual({ kind: 'stored', botId: 'aibTestBot0123', secretIn: 'file' });
    expect(wecom.storedOf({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'elsewhere' }))
      .toEqual({ kind: 'stored', botId: 'aibTestBot0123', secretIn: null });
    expect(wecom.storedOf({ error: 'WeCom refused the bot', code: 'REFUSED' }))
      .toEqual({ kind: 'refused', said: 'WeCom refused the bot' });
    expect(wecom.storedOf({ stored: { bot_id: 7 } })).toEqual({ kind: 'refused', said: '' });
    expect(wecom.storedOf(null)).toEqual({ kind: 'refused', said: '' });
  });

  it('opens the console that the shell opens', () => {
    expect(wecom.CONSOLE).toBe('https://work.weixin.qq.com/wework_admin/frame');
  });
});

describe('the assistant in QQ', () => {
  const bot = {
    set_up: true, app_id: '102030405', paired: false, folder: 'C:/me/Columbina/qq',
    running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
  };

  it('is a bot, named by its AppID, with the open platform of QQ, whose agents change nothing', () => {
    expect(assistantOf('qq', bot)).toEqual(app({
      platform: 'qq', id: '102030405', console: 'https://q.qq.com/', folder: 'C:/me/Columbina/qq', writable: false,
    }));
    // Neither a platform nor a Bot ID makes it another.
    expect(assistantOf('qq', { ...bot, domain: 'lark', bot_id: 'aibTestBot0123' })).toMatchObject({ platform: 'qq', id: '102030405' });
  });
});

describe('the window for the bot of QQ', () => {
  it('opens with the bot that is set up, or empty', () => {
    expect(qq.formOf({ set_up: true, app_id: '102030405' })).toEqual({ form: { appId: '102030405', secret: '' }, setUp: true });
    expect(qq.formOf({ set_up: true, app_id: 7 })).toEqual({ form: { appId: '', secret: '' }, setUp: false });
  });

  it('takes an AppID as the program does', () => {
    for (const fits of ['1', '102030405', ' 102030405 ', 'x'.repeat(64)]) expect(qq.appIdFits(fits), fits).toBe(true);
    for (const wrong of ['', 'x'.repeat(65), '1020 30405', 'QQ机器人']) expect(qq.appIdFits(wrong), wrong).toBe(false);
  });

  it('hands the program the bot, and reads what it answered', () => {
    expect(qq.complete({ appId: '102030405', secret: 's3cr3t' })).toBe(true);
    expect(qq.complete({ appId: '', secret: 's3cr3t' })).toBe(false);
    expect(qq.documentOf({ appId: ' 102030405 ', secret: '\ts3cr3t\n' })).toEqual({ app_id: '102030405', secret: 's3cr3t' });
    expect(qq.storedOf({ stored: { app_id: '102030405' }, secret_in: 'keyring' }))
      .toEqual({ kind: 'stored', appId: '102030405', secretIn: 'keyring' });
    expect(qq.storedOf({ error: 'QQ refused the bot', code: 'REFUSED' })).toEqual({ kind: 'refused', said: 'QQ refused the bot' });
  });
});
