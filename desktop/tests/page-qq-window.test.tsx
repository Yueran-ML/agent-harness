// @vitest-environment jsdom
/**
 * The window for the bot of QQ, mounted over a shell that the test plays. Every AppSecret that is
 * typed here is made up.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QqApp } from '../src/qq/QqApp';
import { likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  delete document.documentElement.dataset.theme;
  document.documentElement.lang = '';
});

const SECRET = 'made-up-secret-0123456789';
const set = {
  set_up: true, app_id: '102030405', paired: true, folder: 'C:/me/Columbina/qq',
  running: false, state: null, code: null, error: null,
};

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" in the window.`);
  return found;
};
const field = (name: string) => one(name) as HTMLInputElement;
const write = (name: string, text: string) => { fireEvent.change(one(name), { target: { value: text } }); };
/** Let everything that is on its way arrive. */
const settled = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  const promise = new Promise<T>((resolve) => { release = resolve; });
  return { promise, release };
}

/** Open the window over a shell that answers as the test says. */
async function opened(given: { language?: string; theme?: string; status?: unknown } = {}) {
  const shell = {
    context: vi.fn(async () => ({ language: 'en', theme: 'moonveil', status: { set_up: false, running: false } as unknown, ...given })),
    store: vi.fn(async (_document: { app_id: string; secret: string }): Promise<unknown> => ({
      stored: { app_id: '102030405' }, secret_in: 'keyring',
    })),
    openPage: vi.fn(async (_address: string) => {}),
    close: vi.fn(async () => {}),
  };
  vi.stubGlobal('columbinaQq', shell);
  render(<QqApp />);
  await settled();
  return shell;
}
const store = async () => { await act(async () => { fireEvent.submit(one('qq-store').closest('form')!); }); };
const storable = () => !(one('qq-store') as HTMLButtonElement).disabled;

describe('the window for the bot of QQ', () => {
  it('opens empty where no bot is set up, with the steps on the open platform, and begins at the AppID', async () => {
    const shell = await opened();
    expect(shell.context).toHaveBeenCalledOnce();
    expect(one('qq-page').querySelector('h1')?.textContent).toBe('Assistant in QQ');
    expect([field('qq-app-id').value, field('qq-secret').value]).toEqual(['', '']);
    expect(document.activeElement).toBe(one('qq-app-id'));
    expect(storable()).toBe(false);
    expect(marked('qq-secret-again')).toEqual([]);
    expect((one('qq-steps') as HTMLDetailsElement).open).toBe(true);
    expect([...one('qq-steps').querySelectorAll('ol > li')].map(step => step.textContent)).toEqual([
      'On the open platform of QQ, make a bot.',
      'In its development settings, have its events come by WebSocket. They show its AppID and AppSecret.',
      'Once the bot is stored here, start the assistant in the settings.',
      'Find the bot in QQ and send it the code in a chat of your own. Put it in a group, and anyone can mention it there.',
    ]);
    expect(one('qq-page').textContent).toContain('The AppSecret goes from this window to Columbina');
    // The window of Feishu asks for permissions, and this one for none.
    expect(document.querySelector('.setup-scopes')).toBeNull();
  });

  it('opens the open platform of QQ, and names its machine', async () => {
    const shell = await opened();
    expect(one('qq-steps').querySelector('.caption.mono')?.textContent).toBe('q.qq.com');
    fireEvent.click(one('qq-console'));
    expect(shell.openPage.mock.calls).toEqual([['https://q.qq.com/']]);
  });

  it('says what an AppID is, and when what is written is none', async () => {
    await opened();
    const note = () => document.getElementById('qq-app-id')!;
    expect(note().textContent).toBe('As the development settings of the bot show it.');
    write('qq-app-id', '1020 30405');
    expect(note().textContent).toBe('An AppID is made of letters, digits and signs, with no blank in it.');
    expect(note().dataset.tone).toBe('bad');
    expect(field('qq-app-id').getAttribute('aria-invalid')).toBe('true');
    write('qq-app-id', ' 102030405 ');
    expect(note().textContent).toBe('As the development settings of the bot show it.');
    expect(field('qq-app-id').hasAttribute('aria-invalid')).toBe(false);
  });

  it('stores the bot with the program, and says where the AppSecret went and what to do next', async () => {
    const shell = await opened();
    write('qq-app-id', '  102030405 ');
    expect(storable()).toBe(false);
    write('qq-secret', ` ${SECRET}\n`);
    expect(storable()).toBe(true);
    document.body.scrollTop = 400;
    document.documentElement.scrollTop = 400;
    await store();
    expect(shell.store.mock.calls).toEqual([[{ app_id: '102030405', secret: SECRET }]]);
    expect([document.body.scrollTop, document.documentElement.scrollTop]).toEqual([0, 0]);
    expect(one('qq-done').textContent).toContain('QQ takes the bot 102030405, and it is stored.');
    expect(one('qq-done').textContent).toContain('The AppSecret is in the keyring of the system.');
    // What is left to do comes next: the steps from the third on, beside the way to the console.
    expect(one('qq-next').textContent).toBe('Next steps');
    expect((one('qq-steps').querySelector('ol') as HTMLOListElement).start).toBe(3);
    expect([...one('qq-steps').querySelectorAll('ol > li')].map(step => step.textContent?.slice(0, 24))).toEqual([
      'Once the bot is stored h', 'Find the bot in QQ and s',
    ]);
    expect(marked('qq-console')).toHaveLength(1);
    // The AppSecret has no business in the window any more.
    expect(marked('qq-secret')).toEqual([]);
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(document.activeElement).toBe(one('qq-close'));
    fireEvent.click(one('qq-close'));
    expect(shell.close).toHaveBeenCalledOnce();
  });

  it('warns where no keyring took the AppSecret', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ stored: { app_id: '102030405' }, secret_in: 'file' });
    write('qq-app-id', '102030405');
    write('qq-secret', SECRET);
    await store();
    expect(one('qq-done').querySelector('[data-tone="warn"]')?.textContent)
      .toBe('No keyring took the AppSecret. It lies in a file in the folder of Columbina, as it is and not encrypted.');
  });

  it('says what the program says is wrong, and keeps the form', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({
      error: 'QQ does not take this AppID with this AppSecret: it gave no access token', code: 'REFUSED',
    });
    write('qq-app-id', '102030405');
    write('qq-secret', SECRET);
    await store();
    expect(one('qq-refused').textContent).toBe('QQ does not take this AppID with this AppSecret: it gave no access token.');
    expect(field('qq-app-id').value).toBe('102030405');
    write('qq-secret', `${SECRET}x`);
    expect(marked('qq-refused')).toEqual([]);
    shell.store.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await store();
    expect(one('qq-refused').textContent).toBe('The setup program did not end in time.');
    shell.store.mockResolvedValueOnce(null);
    await store();
    expect(one('qq-refused').textContent).toBe('That did not go through.');
  });

  it('stores nothing while it stores, nor what is not complete', async () => {
    const shell = await opened();
    const answer = held<unknown>();
    shell.store.mockImplementationOnce(() => answer.promise);
    write('qq-app-id', '102030405');
    write('qq-secret', 'two halves');
    expect(one('qq-blank').textContent).toBe('An AppSecret has no blank in it. See whether what was pasted brought more along.');
    expect(storable()).toBe(false);
    await store();
    expect(shell.store).not.toHaveBeenCalled();
    write('qq-secret', SECRET);
    await store();
    expect(one('qq-store').textContent).toBe('Checking');
    expect(field('qq-secret').disabled).toBe(true);
    expect(field('qq-app-id').disabled).toBe(true);
    await store();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(shell.close).not.toHaveBeenCalled();
    await act(async () => answer.release({ stored: { app_id: '102030405' }, secret_in: 'keyring' }));
    expect(shell.store).toHaveBeenCalledOnce();
  });

  it('opens at the bot that is set up, whose AppSecret is typed again, with the steps folded', async () => {
    await opened({ status: set, theme: 'moonwhite' });
    expect(field('qq-app-id').value).toBe('102030405');
    expect(document.activeElement).toBe(one('qq-secret'));
    expect(one('qq-secret-again').textContent).toBe('The AppSecret that is stored is not shown. Enter it again to store the bot anew.');
    expect((one('qq-steps') as HTMLDetailsElement).open).toBe(false);
    expect(document.documentElement.dataset.theme).toBe('moonwhite');
  });

  it('opens at the bot whose assistant failed, so that its AppSecret can be given again', async () => {
    await opened({ status: { ...set, state: 'failed', error: 'QQ refused the bot' } });
    expect(marked('qq-close')).toEqual([]);
    expect(field('qq-app-id').value).toBe('102030405');
  });

  it('says what is wrong where what is set up cannot be read, or the shell does not answer, and only closes', async () => {
    const shell = await opened({ status: { error: 'the file of the bot cannot be read', code: 'STORE' } });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The file of the bot cannot be read.');
    expect(marked('qq-store')).toEqual([]);
    expect(one('qq-close').textContent).toBe('Close');
    fireEvent.click(one('qq-close'));
    expect(shell.close).toHaveBeenCalledOnce();
    cleanup();
    await opened({ status: null });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('That did not go through.');
    cleanup();
    vi.stubGlobal('columbinaQq', {
      context: vi.fn(async () => { throw new Error('The setup program did not end in time.'); }),
      store: vi.fn(), openPage: vi.fn(), close: vi.fn(),
    });
    render(<QqApp />);
    await settled();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The setup program did not end in time.');
  });

  it('closes with Escape and with Cancel', async () => {
    const shell = await opened();
    fireEvent.keyDown(window, { key: 'Escape', isComposing: true });
    expect(shell.close).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(shell.close).toHaveBeenCalledOnce();
    fireEvent.click([...document.querySelectorAll('button')].find(button => button.textContent === 'Cancel')!);
    expect(shell.close).toHaveBeenCalledTimes(2);
  });

  it('speaks the language that the main window speaks', async () => {
    await opened({ language: 'zh-CN' });
    expect(one('qq-page').querySelector('h1')?.textContent).toBe('QQ 助手');
    expect(document.documentElement.lang).toBe('zh-CN');
    expect(one('qq-store').textContent).toBe('验证并保存');
    expect(one('qq-steps').querySelector('ol > li')?.textContent).toBe('在 QQ 开放平台创建一个机器人。');
  });
});
