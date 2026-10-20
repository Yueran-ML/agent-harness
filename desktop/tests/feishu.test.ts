/**
 * What the page makes of the assistant in Feishu, and what the window for its app makes of what
 * is typed: tried with plain values.
 */
import { describe, expect, it } from 'vitest';
import { assistantAfter, assistantOf, doingOf, endedOf, type App, type Assistant } from '../src/app/feishu';
import { appIdFits, blankInside, complete, CONSOLES, documentOf, formOf, hostOf, SCOPES, storedOf } from '../src/feishu/form';

const status = {
  set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
  running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
};
const app = (given: Partial<App> = {}): Extract<Assistant, { kind: 'app' }> => ({
  kind: 'app', platform: 'feishu', appId: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
  running: false, own: false, state: null, code: null, error: null, ended: null, ...given,
});

describe('how the assistant is doing, as the shell says it', () => {
  it('is not there to ask where there is no service program', () => {
    expect(assistantOf(null)).toEqual({ kind: 'unavailable' });
    expect(assistantOf('no answer')).toEqual({ kind: 'unavailable' });
  });

  it('could not be read where the program says why', () => {
    expect(assistantOf({ error: 'the file of the app cannot be read', code: 'STORE' }))
      .toEqual({ kind: 'failed', said: 'the file of the app cannot be read' });
  });

  it('has no app where none is set up, and says how the last one ended badly', () => {
    expect(assistantOf({ set_up: false, running: false, own: false, ended: null })).toEqual({ kind: 'none', ended: null });
    expect(assistantOf({ set_up: false, ended: { code: 2, said: 'no app of Feishu is set up' } }))
      .toEqual({ kind: 'none', ended: { code: 2, said: 'no app of Feishu is set up' } });
  });

  it('is an app that does not run', () => {
    expect(assistantOf(status)).toEqual(app());
  });

  it('is an app in Lark, with the console of Lark, whatever the program named', () => {
    expect(assistantOf({ ...status, domain: 'lark', console: 'https://elsewhere.example/app' }))
      .toEqual(app({ platform: 'lark', console: 'https://open.larksuite.com/app' }));
  });

  it('runs elsewhere where the program says so and the application runs none', () => {
    expect(assistantOf({ ...status, running: true, state: 'connected', paired: true }))
      .toEqual(app({ running: true, state: 'connected', paired: true }));
  });

  it('runs here from the moment that the application started it, before it said anything', () => {
    expect(assistantOf({ ...status, own: true, ended: { code: 1, said: 'an old end' } }))
      .toEqual(app({ running: true, own: true }));
  });

  it('shows a code of pairing only while it runs, and only one of six digits', () => {
    expect(assistantOf({ ...status, own: true, code: '123456' }).kind === 'app'
      && (assistantOf({ ...status, own: true, code: '123456' }) as App).code).toBe('123456');
    for (const code of ['12345', '1234567', 'abcdef', 123456, null]) {
      expect((assistantOf({ ...status, own: true, code }) as App).code).toBeNull();
    }
    expect((assistantOf({ ...status, code: '123456' }) as App).code).toBeNull();
  });

  it('failed where the last one failed, also after it ended', () => {
    expect(assistantOf({ ...status, state: 'failed', error: 'Feishu refuses the app' }))
      .toEqual(app({ state: 'failed', error: 'Feishu refuses the app' }));
  });

  it('is stopping where the program said that it stopped and it has not ended yet', () => {
    expect((assistantOf({ ...status, own: true, state: 'stopped' }) as App).state).toBe('stopping');
    expect((assistantOf({ ...status, state: 'stopped' }) as App).state).toBeNull();
    expect((assistantOf({ ...status, own: true, state: 'made up' }) as App).state).toBeNull();
  });
});

describe('what the assistant that the application runs says of itself', () => {
  it('belongs to no app where the page knows of none', () => {
    for (const current of [{ kind: 'unknown' }, { kind: 'unavailable' }, { kind: 'none', ended: null }] as Assistant[]) {
      expect(assistantAfter(current, { state: 'connected' })).toBeNull();
    }
  });

  it('says how it is doing', () => {
    const before = app({ ended: { code: 1, said: 'an old end' } });
    expect(assistantAfter(before, { state: 'connecting', paired: false, code: '123456', error: null }))
      .toEqual(app({ running: true, own: true, state: 'connecting', code: '123456' }));
    expect(assistantAfter(before, { state: 'connecting', paired: true, code: null, error: 'the connection was closed' }))
      .toEqual(app({ running: true, own: true, state: 'connecting', paired: true, error: 'the connection was closed' }));
    expect(assistantAfter(before, { state: 'stopped', paired: true }))
      .toEqual(app({ running: true, own: true, state: 'stopping', paired: true }));
    expect(assistantAfter(before, { state: 'failed', paired: false, error: 'Feishu refuses the app' }))
      .toEqual(app({ running: true, own: true, state: 'failed', error: 'Feishu refuses the app' }));
  });

  it('belongs to another app where it names another', () => {
    const running = app({ running: true, own: true, state: 'connected' });
    expect(assistantAfter(running, { state: 'connected', app_id: 'cli_other123', domain: 'feishu' })).toBeNull();
    expect(assistantAfter(running, { state: 'connected', app_id: 'cli_a1b2c3d4', domain: 'lark' })).toBeNull();
    expect(assistantAfter(running, { state: 'connected', app_id: 'cli_a1b2c3d4', domain: 'feishu' }))
      .toEqual(app({ running: true, own: true, state: 'connected' }));
    // A line that names no app is of the one that the page knows of.
    expect(assistantAfter(running, { state: 'connected', domain: 'elsewhere' })).toEqual(app({ running: true, own: true, state: 'connected' }));
  });

  it('ended, and leaves nothing to say where it was stopped', () => {
    const running = app({ running: true, own: true, state: 'connected', code: '123456', error: 'old' });
    expect(assistantAfter(running, { ended: true, code: 0, said: 'stopped' })).toEqual(app());
  });

  it('ended badly, and says the last line that it wrote', () => {
    const running = app({ running: true, own: true, state: 'connecting' });
    expect(assistantAfter(running, { ended: true, code: 2, said: 'an assistant in Feishu runs on this machine already' }))
      .toEqual(app({ ended: { code: 2, said: 'an assistant in Feishu runs on this machine already' } }));
  });

  it('ended after it failed, and keeps why it failed', () => {
    const failed = app({ running: true, own: true, state: 'failed', error: 'Feishu refuses the app', paired: true });
    expect(assistantAfter(failed, { ended: true, code: 1, said: 'Feishu refuses the app' })).toEqual(app({
      state: 'failed', error: 'Feishu refuses the app', paired: true, ended: { code: 1, said: 'Feishu refuses the app' },
    }));
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
      .toEqual({ kind: 'refused', said: 'Feishu does not take the app' });
    expect(storedOf({ stored: { app_id: 7 } })).toEqual({ kind: 'refused', said: '' });
    expect(storedOf(null)).toEqual({ kind: 'refused', said: '' });
  });

  it('opens the consoles that the shell opens, and names their machines', () => {
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
