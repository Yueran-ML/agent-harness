// @vitest-environment jsdom
/**
 * The window for the app of Feishu, mounted over a shell that the test plays. Every App Secret
 * that is typed here is made up.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FeishuApp } from '../src/feishu/FeishuApp';
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
  set_up: true, domain: 'lark', app_id: 'cli_a1b2c3d4', console: 'https://open.larksuite.com/app', paired: true,
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
    store: vi.fn(async (_document: { domain: string; app_id: string; app_secret: string }): Promise<unknown> => ({
      stored: { domain: 'feishu', app_id: 'cli_a1b2c3d4' }, secret_in: 'keyring',
    })),
    openPage: vi.fn(async (_address: string) => {}),
    close: vi.fn(async () => {}),
  };
  vi.stubGlobal('columbinaFeishu', shell);
  render(<FeishuApp />);
  await settled();
  return shell;
}
const store = async () => { await act(async () => { fireEvent.submit(one('feishu-store').closest('form')!); }); };
const storable = () => !(one('feishu-store') as HTMLButtonElement).disabled;

describe('the window for the app of Feishu', () => {
  it('opens empty on Feishu where no app is set up, with the steps in the console, and begins at the App ID', async () => {
    const shell = await opened();
    expect(shell.context).toHaveBeenCalledOnce();
    expect(one('feishu-page').querySelector('h1')?.textContent).toBe('Assistant in Feishu');
    expect((one('feishu-platform') as HTMLSelectElement).value).toBe('feishu');
    expect([...(one('feishu-platform') as HTMLSelectElement).options].map(option => option.textContent)).toEqual(['Feishu', 'Lark']);
    expect([field('feishu-app-id').value, field('feishu-secret').value]).toEqual(['', '']);
    expect(document.activeElement).toBe(one('feishu-app-id'));
    expect(storable()).toBe(false);
    expect(marked('feishu-secret-again')).toEqual([]);
    expect((one('feishu-steps') as HTMLDetailsElement).open).toBe(true);
    expect(one('feishu-steps').querySelectorAll('ol > li')).toHaveLength(7);
    expect([...one('feishu-scopes').querySelectorAll('code')].map(code => code.textContent)).toEqual([
      'im:message.p2p_msg:readonly', 'im:message:send_as_bot', 'im:message.reactions:write_only', 'im:message:readonly',
      'docx:document:readonly', 'wiki:wiki:readonly', 'sheets:spreadsheet:readonly',
    ]);
    expect(one('feishu-page').textContent).toContain('The App Secret goes from this window to Columbina');
  });

  it('opens the console of the platform that is chosen', async () => {
    const shell = await opened();
    expect(one('feishu-steps').querySelector('.caption.mono')?.textContent).toBe('open.feishu.cn');
    fireEvent.click(one('feishu-console'));
    fireEvent.change(one('feishu-platform'), { target: { value: 'lark' } });
    expect(one('feishu-steps').querySelector('.caption.mono')?.textContent).toBe('open.larksuite.com');
    fireEvent.click(one('feishu-console'));
    expect(shell.openPage.mock.calls).toEqual([['https://open.feishu.cn/app'], ['https://open.larksuite.com/app']]);
  });

  it('says what an App ID is, and when what is written is none', async () => {
    await opened();
    expect(one('feishu-app-id').nextElementSibling).toBeNull();
    const note = () => document.getElementById('feishu-app-id')!;
    expect(note().textContent).toBe('It begins with cli_.');
    write('feishu-app-id', 'a1b2c3d4');
    expect(note().textContent).toBe('An App ID begins with cli_, followed by letters and digits.');
    expect(note().dataset.tone).toBe('bad');
    expect(field('feishu-app-id').getAttribute('aria-invalid')).toBe('true');
    write('feishu-app-id', 'cli_a1b2c3d4');
    expect(note().textContent).toBe('It begins with cli_.');
    expect(field('feishu-app-id').hasAttribute('aria-invalid')).toBe(false);
  });

  it('stores the app with the program, and says where the App Secret went and what to do next', async () => {
    const shell = await opened();
    fireEvent.change(one('feishu-platform'), { target: { value: 'lark' } });
    write('feishu-app-id', '  cli_a1b2c3d4 ');
    expect(storable()).toBe(false);
    write('feishu-secret', ` ${SECRET}\n`);
    expect(storable()).toBe(true);
    // Read down to its foot, as the window was when the button was pressed.
    document.body.scrollTop = 400;
    document.documentElement.scrollTop = 400;
    await store();
    expect(shell.store.mock.calls).toEqual([[{ domain: 'lark', app_id: 'cli_a1b2c3d4', app_secret: SECRET }]]);
    // What was stored is said at the top, where it is read first.
    expect([document.body.scrollTop, document.documentElement.scrollTop]).toEqual([0, 0]);
    // The platform that was chosen took it, and its name is said.
    expect(one('feishu-done').textContent).toContain('Lark takes the app cli_a1b2c3d4, and it is stored.');
    expect(one('feishu-steps').textContent).toContain('Find the bot by its name in Lark');
    expect(one('feishu-done').textContent).toContain('The App Secret is in the keyring of the system.');
    // What is left to do comes next: the steps from the fourth on, beside the way to the console.
    expect(one('feishu-next').textContent).toBe('Next steps');
    expect((one('feishu-steps').querySelector('ol') as HTMLOListElement).start).toBe(4);
    expect([...one('feishu-steps').querySelectorAll('ol > li')].map(step => step.textContent?.slice(0, 22))).toEqual([
      'Once the app is stored', 'While the assistant ru', 'Under Version Manageme', 'Find the bot by its na',
    ]);
    expect(marked('feishu-scopes')).toEqual([]);
    expect(marked('feishu-console')).toHaveLength(1);
    // The App Secret has no business in the window any more.
    expect(marked('feishu-secret')).toEqual([]);
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(document.activeElement).toBe(one('feishu-close'));
    fireEvent.click(one('feishu-close'));
    expect(shell.close).toHaveBeenCalledOnce();
  });

  it('warns where no keyring took the App Secret', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ stored: { domain: 'feishu', app_id: 'cli_a1b2c3d4' }, secret_in: 'file' });
    write('feishu-app-id', 'cli_a1b2c3d4');
    write('feishu-secret', SECRET);
    await store();
    expect(one('feishu-done').querySelector('[data-tone="warn"]')?.textContent)
      .toBe('No keyring took the App Secret. It lies in a file in the folder of Columbina, as it is and not encrypted.');
  });

  it('says what the program says is wrong, and keeps the form', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ error: 'Feishu does not take the app: app secret invalid (10014)', code: 'REFUSED' });
    write('feishu-app-id', 'cli_a1b2c3d4');
    write('feishu-secret', SECRET);
    await store();
    expect(one('feishu-refused').textContent).toBe('Feishu does not take the app: app secret invalid (10014).');
    expect(field('feishu-app-id').value).toBe('cli_a1b2c3d4');
    // What is written next lets it go.
    write('feishu-secret', `${SECRET}x`);
    expect(marked('feishu-refused')).toEqual([]);
    shell.store.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await store();
    expect(one('feishu-refused').textContent).toBe('The setup program did not end in time.');
    shell.store.mockResolvedValueOnce(null);
    await store();
    expect(one('feishu-refused').textContent).toBe('That did not go through.');
  });

  it('stores nothing while it stores, nor what is not complete', async () => {
    const shell = await opened();
    const answer = held<unknown>();
    shell.store.mockImplementationOnce(() => answer.promise);
    write('feishu-app-id', 'cli_a1b2c3d4');
    write('feishu-secret', 'two halves');
    expect(one('feishu-blank').textContent).toBe('An App Secret has no blank in it. See whether what was pasted brought more along.');
    expect(storable()).toBe(false);
    await store();
    expect(shell.store).not.toHaveBeenCalled();
    write('feishu-secret', SECRET);
    await store();
    expect(one('feishu-store').textContent).toBe('Checking');
    expect(field('feishu-secret').disabled).toBe(true);
    await store();
    // Escape does not close the window while it stores.
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(shell.close).not.toHaveBeenCalled();
    await act(async () => answer.release({ stored: { app_id: 'cli_a1b2c3d4' }, secret_in: 'keyring' }));
    expect(shell.store).toHaveBeenCalledOnce();
  });

  it('opens at the app that is set up, whose App Secret is typed again, with the steps folded', async () => {
    await opened({ status: set, theme: 'moonwhite' });
    expect((one('feishu-platform') as HTMLSelectElement).value).toBe('lark');
    expect(field('feishu-app-id').value).toBe('cli_a1b2c3d4');
    expect(document.activeElement).toBe(one('feishu-secret'));
    expect(one('feishu-secret-again').textContent).toBe('The App Secret that is stored is not shown. Enter it again to store the app anew.');
    expect((one('feishu-steps') as HTMLDetailsElement).open).toBe(false);
    expect(document.documentElement.dataset.theme).toBe('moonwhite');
  });

  it('opens at the app whose assistant failed, so that its App Secret can be given again', async () => {
    await opened({ status: { ...set, state: 'failed', error: 'Feishu refuses the connection: forbidden' } });
    expect(marked('feishu-close')).toEqual([]);
    expect(field('feishu-app-id').value).toBe('cli_a1b2c3d4');
    expect(document.activeElement).toBe(one('feishu-secret'));
  });

  it('says that nothing is known where the shell answers with no status', async () => {
    await opened({ status: null });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('That did not go through.');
    expect(marked('feishu-store')).toEqual([]);
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

  it('says what is wrong where what is set up cannot be read, and only closes', async () => {
    const shell = await opened({ status: { error: 'the file of the app cannot be read', code: 'STORE' } });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The file of the app cannot be read.');
    expect(marked('feishu-store')).toEqual([]);
    fireEvent.click(one('feishu-close'));
    expect(shell.close).toHaveBeenCalledOnce();
    expect(one('feishu-close').textContent).toBe('Close');
  });

  it('says what is wrong where the shell does not answer', async () => {
    vi.stubGlobal('columbinaFeishu', {
      context: vi.fn(async () => { throw new Error('The setup program did not end in time.'); }),
      store: vi.fn(), openPage: vi.fn(), close: vi.fn(),
    });
    render(<FeishuApp />);
    await settled();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The setup program did not end in time.');
  });

  it('speaks the language that the main window speaks', async () => {
    await opened({ language: 'zh-CN' });
    expect(one('feishu-page').querySelector('h1')?.textContent).toBe('飞书助手');
    expect(document.documentElement.lang).toBe('zh-CN');
    expect(one('feishu-store').textContent).toBe('验证并保存');
    expect([...(one('feishu-platform') as HTMLSelectElement).options].map(option => option.textContent)).toEqual(['飞书', 'Lark']);
  });
});
