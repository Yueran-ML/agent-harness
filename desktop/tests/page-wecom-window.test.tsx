// @vitest-environment jsdom
/**
 * The window for the bot of WeCom, mounted over a shell that the test plays. Every Secret that is
 * typed here is made up.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WecomApp } from '../src/wecom/WecomApp';
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
  set_up: true, bot_id: 'aibTestBot0123', paired: true, folder: 'C:/me/Columbina/wecom', writable: false,
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
    store: vi.fn(async (_document: { bot_id: string; secret: string }): Promise<unknown> => ({
      stored: { bot_id: 'aibTestBot0123' }, secret_in: 'keyring',
    })),
    openPage: vi.fn(async (_address: string) => {}),
    close: vi.fn(async () => {}),
  };
  vi.stubGlobal('columbinaWecom', shell);
  render(<WecomApp />);
  await settled();
  return shell;
}
const store = async () => { await act(async () => { fireEvent.submit(one('wecom-store').closest('form')!); }); };
const storable = () => !(one('wecom-store') as HTMLButtonElement).disabled;

describe('the window for the bot of WeCom', () => {
  it('opens empty where no bot is set up, with the steps in the admin console, and begins at the Bot ID', async () => {
    const shell = await opened();
    expect(shell.context).toHaveBeenCalledOnce();
    expect(one('wecom-page').querySelector('h1')?.textContent).toBe('Assistant in WeCom');
    expect([field('wecom-bot-id').value, field('wecom-secret').value]).toEqual(['', '']);
    expect(document.activeElement).toBe(one('wecom-bot-id'));
    expect(storable()).toBe(false);
    expect(marked('wecom-secret-again')).toEqual([]);
    expect((one('wecom-steps') as HTMLDetailsElement).open).toBe(true);
    expect([...one('wecom-steps').querySelectorAll('ol > li')].map(step => step.textContent)).toEqual([
      'In the admin console of WeCom, make a smart bot.',
      'On the page of the bot, turn on the API mode and choose the long connection. The page then shows its Bot ID and Secret.',
      'Once the bot is stored here, start the assistant in the settings.',
      'Find the bot in WeCom, and send it the code in a chat of your own.',
    ]);
    expect(one('wecom-page').textContent).toContain('The Secret goes from this window to Columbina');
    // The window of Feishu asks for permissions, and this one for none.
    expect(document.querySelector('.setup-scopes')).toBeNull();
  });

  it('opens the admin console of WeCom, and names its machine', async () => {
    const shell = await opened();
    expect(one('wecom-steps').querySelector('.caption.mono')?.textContent).toBe('work.weixin.qq.com');
    fireEvent.click(one('wecom-console'));
    expect(shell.openPage.mock.calls).toEqual([['https://work.weixin.qq.com/wework_admin/frame']]);
  });

  it('says what a Bot ID is, and when what is written is none', async () => {
    await opened();
    const note = () => document.getElementById('wecom-bot-id')!;
    expect(note().textContent).toBe('As the page of the bot shows it.');
    write('wecom-bot-id', 'aib Test');
    expect(note().textContent).toBe('A Bot ID is made of letters, digits and signs, with no blank in it.');
    expect(note().dataset.tone).toBe('bad');
    expect(field('wecom-bot-id').getAttribute('aria-invalid')).toBe('true');
    write('wecom-bot-id', ' aibTestBot0123 ');
    expect(note().textContent).toBe('As the page of the bot shows it.');
    expect(field('wecom-bot-id').hasAttribute('aria-invalid')).toBe(false);
  });

  it('stores the bot with the program, and says where the Secret went and what to do next', async () => {
    const shell = await opened();
    write('wecom-bot-id', '  aibTestBot0123 ');
    expect(storable()).toBe(false);
    write('wecom-secret', ` ${SECRET}\n`);
    expect(storable()).toBe(true);
    document.body.scrollTop = 400;
    document.documentElement.scrollTop = 400;
    await store();
    expect(shell.store.mock.calls).toEqual([[{ bot_id: 'aibTestBot0123', secret: SECRET }]]);
    expect([document.body.scrollTop, document.documentElement.scrollTop]).toEqual([0, 0]);
    expect(one('wecom-done').textContent).toContain('WeCom takes the bot aibTestBot0123, and it is stored.');
    expect(one('wecom-done').textContent).toContain('The Secret is in the keyring of the system.');
    // What is left to do comes next: the steps from the third on, beside the way to the console.
    expect(one('wecom-next').textContent).toBe('Next steps');
    expect((one('wecom-steps').querySelector('ol') as HTMLOListElement).start).toBe(3);
    expect([...one('wecom-steps').querySelectorAll('ol > li')].map(step => step.textContent?.slice(0, 24))).toEqual([
      'Once the bot is stored h', 'Find the bot in WeCom, a',
    ]);
    expect(marked('wecom-console')).toHaveLength(1);
    // The Secret has no business in the window any more.
    expect(marked('wecom-secret')).toEqual([]);
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(document.activeElement).toBe(one('wecom-close'));
    fireEvent.click(one('wecom-close'));
    expect(shell.close).toHaveBeenCalledOnce();
  });

  it('warns where no keyring took the Secret', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'file' });
    write('wecom-bot-id', 'aibTestBot0123');
    write('wecom-secret', SECRET);
    await store();
    expect(one('wecom-done').querySelector('[data-tone="warn"]')?.textContent)
      .toBe('No keyring took the Secret. It lies in a file in the folder of Columbina, as it is and not encrypted.');
  });

  it('says what the program says is wrong, and keeps the form', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({
      error: 'WeCom does not take the Bot ID or the Secret: invalid secret (errcode 853000)', code: 'REFUSED',
    });
    write('wecom-bot-id', 'aibTestBot0123');
    write('wecom-secret', SECRET);
    await store();
    expect(one('wecom-refused').textContent).toBe('WeCom does not take the Bot ID or the Secret: invalid secret (errcode 853000).');
    expect(field('wecom-bot-id').value).toBe('aibTestBot0123');
    write('wecom-secret', `${SECRET}x`);
    expect(marked('wecom-refused')).toEqual([]);
    shell.store.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await store();
    expect(one('wecom-refused').textContent).toBe('The setup program did not end in time.');
    shell.store.mockResolvedValueOnce(null);
    await store();
    expect(one('wecom-refused').textContent).toBe('That did not go through.');
  });

  it('stores nothing while it stores, nor what is not complete', async () => {
    const shell = await opened();
    const answer = held<unknown>();
    shell.store.mockImplementationOnce(() => answer.promise);
    write('wecom-bot-id', 'aibTestBot0123');
    write('wecom-secret', 'two halves');
    expect(one('wecom-blank').textContent).toBe('A Secret has no blank in it. See whether what was pasted brought more along.');
    expect(storable()).toBe(false);
    await store();
    expect(shell.store).not.toHaveBeenCalled();
    write('wecom-secret', SECRET);
    await store();
    expect(one('wecom-store').textContent).toBe('Checking');
    expect(field('wecom-secret').disabled).toBe(true);
    expect(field('wecom-bot-id').disabled).toBe(true);
    await store();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(shell.close).not.toHaveBeenCalled();
    await act(async () => answer.release({ stored: { bot_id: 'aibTestBot0123' }, secret_in: 'keyring' }));
    expect(shell.store).toHaveBeenCalledOnce();
  });

  it('opens at the bot that is set up, whose Secret is typed again, with the steps folded', async () => {
    await opened({ status: set, theme: 'moonwhite' });
    expect(field('wecom-bot-id').value).toBe('aibTestBot0123');
    expect(document.activeElement).toBe(one('wecom-secret'));
    expect(one('wecom-secret-again').textContent).toBe('The Secret that is stored is not shown. Enter it again to store the bot anew.');
    expect((one('wecom-steps') as HTMLDetailsElement).open).toBe(false);
    expect(document.documentElement.dataset.theme).toBe('moonwhite');
  });

  it('opens at the bot whose assistant failed, so that its Secret can be given again', async () => {
    await opened({ status: { ...set, state: 'failed', error: 'WeCom refused the bot' } });
    expect(marked('wecom-close')).toEqual([]);
    expect(field('wecom-bot-id').value).toBe('aibTestBot0123');
  });

  it('says what is wrong where what is set up cannot be read, or the shell does not answer, and only closes', async () => {
    const shell = await opened({ status: { error: 'the file of the bot cannot be read', code: 'STORE' } });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The file of the bot cannot be read.');
    expect(marked('wecom-store')).toEqual([]);
    expect(one('wecom-close').textContent).toBe('Close');
    fireEvent.click(one('wecom-close'));
    expect(shell.close).toHaveBeenCalledOnce();
    cleanup();
    await opened({ status: null });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('That did not go through.');
    cleanup();
    vi.stubGlobal('columbinaWecom', {
      context: vi.fn(async () => { throw new Error('The setup program did not end in time.'); }),
      store: vi.fn(), openPage: vi.fn(), close: vi.fn(),
    });
    render(<WecomApp />);
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
    expect(one('wecom-page').querySelector('h1')?.textContent).toBe('企业微信助手');
    expect(document.documentElement.lang).toBe('zh-CN');
    expect(one('wecom-store').textContent).toBe('验证并保存');
    expect(one('wecom-steps').querySelector('ol > li')?.textContent).toBe('在企业微信管理后台创建一个智能机器人。');
  });
});
