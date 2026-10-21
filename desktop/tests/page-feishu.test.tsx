// @vitest-environment jsdom
/**
 * The assistant in Feishu in the settings: what the page holds of it, against a shell that the
 * test plays, and the part of the settings that shows it. No App Secret comes this way.
 */
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FeishuAssistant } from '../src/app/FeishuAssistant';
import { Words } from '../src/app/locale';
import { messagesFor } from '../src/i18n';
import { useFeishu } from '../src/useFeishu';
import { likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, 'clipboard');
});

const app = {
  set_up: true, domain: 'feishu', app_id: 'cli_a1b2c3d4', console: 'https://open.feishu.cn/app', paired: false,
  running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
};

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const buttons = () => [...document.querySelectorAll('button')].map(each => each.textContent);
const press = async (name: string) => { await act(async () => { fireEvent.click(one(name)); }); };
/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  let refuse!: (problem: Error) => void;
  const promise = new Promise<T>((resolve, reject) => { release = resolve; refuse = reject; });
  return { promise, release, refuse };
}

/** The shell as the page meets it, as far as the assistant in Feishu goes, played by the test. */
function shellOfFeishu(answer: unknown = app) {
  const listeners = new Set<(said: Record<string, unknown>) => void>();
  const shell = {
    feishuStatus: vi.fn(async (): Promise<unknown> => answer),
    feishuStart: vi.fn(async (_asked: { language: string }) => {}),
    feishuStop: vi.fn(async () => {}),
    feishuForget: vi.fn(async (): Promise<unknown> => ({ forgotten: true })),
    feishuLog: vi.fn(async () => {}),
    setUpFeishu: vi.fn(async (_asked: { language: string }) => ({ changed: false })),
    onFeishu: vi.fn((listener: (said: Record<string, unknown>) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    }),
  };
  vi.stubGlobal('columbina', shell);
  return {
    shell,
    /** Say what the assistant says of itself, line by line. */
    tell: (...said: Record<string, unknown>[]) => act(() => {
      for (const document of said) for (const listener of [...listeners]) listener(document);
    }),
    listening: () => listeners.size,
  };
}

/** Draw the part of the settings, in a language, and let what it asked for when it was drawn arrive. */
async function drawn(language = 'en') {
  const opened = vi.fn();
  const result = render(<Words.Provider value={messagesFor(language)}>
    <FeishuAssistant language={language} onOpen={opened} />
  </Words.Provider>);
  await act(async () => {});
  return { ...result, opened };
}

describe('what the page holds of the assistant', () => {
  it('has nothing to ask where there is no shell, as in a replay', async () => {
    const { result } = renderHook(() => useFeishu('en'));
    expect(result.current.assistant).toEqual({ kind: 'unavailable' });
    await act(async () => {
      await result.current.refresh();
      await result.current.start();
      await result.current.stop();
      expect(await result.current.forget()).toBe(false);
      await result.current.setUp();
      result.current.showLog();
    });
    expect(result.current.assistant).toEqual({ kind: 'unavailable' });
    expect(result.current.failure).toBeNull();
  });

  it('takes what the assistant says over an answer that was asked for before it', async () => {
    const { shell, tell } = shellOfFeishu();
    const late = held<unknown>();
    const { result } = renderHook(() => useFeishu('en'));
    await act(async () => {});
    shell.feishuStatus.mockImplementationOnce(() => late.promise);
    let asking!: Promise<void>;
    act(() => { asking = result.current.refresh(); });
    await tell({ state: 'connected', paired: true, code: null, error: null });
    await act(async () => {
      late.release(app);
      await asking;
    });
    expect(result.current.assistant).toMatchObject({ kind: 'app', running: true, own: true, state: 'connected', paired: true });
  });

  it('asks again where the assistant says something of an app that the page does not know of', async () => {
    const { shell, tell } = shellOfFeishu({ set_up: false, running: false, own: false, on: false, ended: null });
    renderHook(() => useFeishu('en'));
    await act(async () => {});
    expect(shell.feishuStatus).toHaveBeenCalledTimes(1);
    shell.feishuStatus.mockResolvedValueOnce({ ...app, own: true });
    await tell({ state: 'connecting', paired: false, code: '123456', error: null });
    await act(async () => {});
    expect(shell.feishuStatus).toHaveBeenCalledTimes(2);
  });

  it('stops listening when it is gone', async () => {
    const { listening } = shellOfFeishu();
    const { unmount } = renderHook(() => useFeishu('en'));
    expect(listening()).toBe(1);
    unmount();
    expect(listening()).toBe(0);
  });
});

describe('the assistant in the settings', () => {
  it('says that it is asked how the assistant is doing, and that there is nothing to ask in a replay', async () => {
    const { shell } = shellOfFeishu();
    const answer = held<unknown>();
    shell.feishuStatus.mockImplementationOnce(() => answer.promise);
    render(<FeishuAssistant language="en" onOpen={vi.fn()} />);
    expect(document.body.textContent).toBe('Asking how the assistant in Feishu is doing.');
    await act(async () => answer.release(app));
    cleanup();
    vi.unstubAllGlobals();
    render(<FeishuAssistant language="en" onOpen={vi.fn()} />);
    expect(document.body.textContent).toBe(
      'The assistant in Feishu is set up here where the application runs the service itself, and not in a replay.',
    );
  });

  it('offers to set up an app where none is, in the window of its own, and asks again once it closed', async () => {
    const { shell } = shellOfFeishu({ set_up: false, running: false, own: false, on: false, ended: null });
    await drawn('ja');
    expect(document.body.textContent).toContain('Feishu や Lark で Columbina と話せます。');
    shell.feishuStatus.mockResolvedValueOnce(app);
    shell.setUpFeishu.mockResolvedValueOnce({ changed: true });
    await press('feishu-set-up');
    expect(shell.setUpFeishu.mock.calls).toEqual([[{ language: 'ja' }]]);
    expect(shell.feishuStatus).toHaveBeenCalledTimes(2);
    expect(one('feishu-app').textContent).toBe('Feishucli_a1b2c3d4');
    // Its assistant is started in the language of the page as well.
    await press('feishu-start');
    expect(shell.feishuStart.mock.calls).toEqual([[{ language: 'ja' }]]);
  });

  it('shows the app that does not run, and starts its assistant', async () => {
    const { shell, tell } = shellOfFeishu();
    const { opened } = await drawn();
    expect(one('feishu-app').textContent).toBe('Feishucli_a1b2c3d4');
    expect(one('feishu-app').querySelector('.feishu-light')?.hasAttribute('data-on')).toBe(false);
    expect(one('feishu-not-running').textContent).toBe('The assistant does not run.');
    expect(buttons()).toEqual([
      'Start the assistant', 'Change the app', 'Delete the app', 'Open the developer console', 'Show the log',
    ]);
    expect(document.body.textContent).toContain('The assistant runs only while Columbina is open.');
    // Of the service it is told once it runs.
    expect(marked('feishu-service')).toEqual([]);

    shell.feishuStatus.mockResolvedValueOnce({ ...app, own: true, on: true });
    await press('feishu-start');
    // What the assistant says by itself, it is to say in the language of the page.
    expect(shell.feishuStart.mock.calls).toEqual([[{ language: 'en' }]]);
    expect(one('feishu-doing').textContent).toBe('Connecting to Feishu.');
    expect(one('feishu-app').querySelector('.feishu-light')?.hasAttribute('data-on')).toBe(true);
    expect(buttons()).toEqual(['Stop', 'Change the app', 'Delete the app', 'Open the developer console', 'Show the log']);

    await tell({ state: 'connected', paired: false, code: '123456', error: null });
    expect(one('feishu-code').textContent).toBe('123456');
    expect(one('feishu-pair').textContent).toContain('Send the bot this code in Feishu');
    expect(one('feishu-pair').textContent).toContain('The code holds for 10 minutes.');
    expect(one('feishu-not-heard').textContent).toContain('No answer from the bot?');

    await tell({ state: 'connected', paired: true, code: null, error: null });
    expect(marked('feishu-pair')).toEqual([]);
    expect(one('feishu-doing').textContent).toBe('Connected. The assistant answers you in Feishu.');

    fireEvent.click(one('feishu-console'));
    expect(opened.mock.calls).toEqual([['https://open.feishu.cn/app']]);
    fireEvent.click(one('feishu-log'));
    expect(shell.feishuLog).toHaveBeenCalledOnce();
  });

  it('copies the code of pairing', async () => {
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    shellOfFeishu({ ...app, own: true, state: 'connected', code: '654321' });
    await drawn();
    await press('feishu-copy');
    expect(writeText.mock.calls).toEqual([['654321']]);
    expect(one('feishu-copy').textContent).toBe('Copied');
  });

  it('keeps the code to be typed by hand where there is no clipboard', async () => {
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: vi.fn(async () => { throw new Error('denied'); }) }, configurable: true });
    shellOfFeishu({ ...app, own: true, state: 'connected', code: '654321' });
    await drawn();
    await press('feishu-copy');
    expect(one('feishu-copy').textContent).toBe('Copy the code');
  });

  it('says that the code was given up on, and that Feishu cannot be reached for now', async () => {
    const { tell } = shellOfFeishu({ ...app, own: true, state: 'connected' });
    await drawn();
    expect(one('feishu-doing').textContent).toBe(
      'The code was given up on after too many wrong tries. Stop the assistant and start it again for a new one.',
    );
    await tell({ state: 'connecting', paired: true, code: null, error: 'Feishu could not be reached: [Errno 11001] getaddrinfo failed' });
    expect(one('feishu-doing').textContent).toBe('Feishu cannot be reached just now. The assistant keeps trying.');
    expect(document.querySelector('details')?.textContent).toContain('Feishu could not be reached: [Errno 11001] getaddrinfo failed.');
  });

  it('stops the assistant that it runs, and says nothing more once it has ended well', async () => {
    const { shell, tell } = shellOfFeishu({ ...app, own: true, paired: true, state: 'connected' });
    await drawn();
    const stopping = held<void>();
    shell.feishuStop.mockImplementationOnce(() => stopping.promise);
    act(() => { fireEvent.click(one('feishu-stop')); });
    expect(one('feishu-stop').textContent).toBe('Stopping');
    expect((one('feishu-stop') as HTMLButtonElement).disabled).toBe(true);
    await tell({ state: 'stopped', paired: true, code: null, error: null });
    expect(one('feishu-doing').textContent).toBe('Stopping the assistant.');
    await tell({ ended: true, code: 0, said: 'stopped' });
    shell.feishuStatus.mockResolvedValueOnce({ ...app, paired: true });
    await act(async () => stopping.release());
    expect(one('feishu-not-running').textContent).toBe('The assistant does not run.');
    expect(marked('feishu-ended')).toEqual([]);
  });

  it('says how an assistant that it ran ended badly, with the last line that it wrote', async () => {
    const { tell } = shellOfFeishu({ ...app, own: true });
    await drawn();
    await tell({ ended: true, code: 2, said: 'an assistant in Feishu runs on this machine already, and only one may run' });
    expect(one('feishu-ended').querySelector('p')?.textContent).toBe('The assistant stopped.');
    expect(one('feishu-ended').querySelector('details')?.textContent)
      .toContain('An assistant in Feishu runs on this machine already, and only one may run.');
  });

  it('says that the last run failed, and why', async () => {
    shellOfFeishu({ ...app, state: 'failed', error: 'Feishu refuses the app: the app is not enabled' });
    await drawn();
    expect(one('feishu-last-error').querySelector('p')?.textContent).toBe('The last run of the assistant did not end well.');
    expect(one('feishu-last-error').querySelector('details')?.textContent).toContain('Feishu refuses the app: the app is not enabled.');
    expect(marked('feishu-ended')).toEqual([]);
  });

  it('opens the console of the platform of the app', async () => {
    shellOfFeishu({ ...app, domain: 'lark', console: 'https://open.larksuite.com/app' });
    const { opened } = await drawn();
    expect(one('feishu-app').textContent).toBe('Larkcli_a1b2c3d4');
    fireEvent.click(one('feishu-console'));
    expect(opened.mock.calls).toEqual([['https://open.larksuite.com/app']]);
  });

  it('leaves an assistant that runs elsewhere to be stopped there', async () => {
    shellOfFeishu({ ...app, running: true, state: 'connected', paired: true });
    await drawn();
    expect(one('feishu-doing').textContent).toBe('The assistant runs elsewhere on this computer, such as in a terminal, and is stopped there.');
    expect(marked('feishu-stop')).toEqual([]);
    expect(marked('feishu-start')).toEqual([]);
    // It goes on with the app that it read, so the app is changed where it is stopped.
    expect(marked('feishu-change')).toEqual([]);
    expect(marked('feishu-forget')).toEqual([]);
    expect(document.body.textContent).not.toContain('The assistant runs only while Columbina is open.');
    // It talks to the service that was in use when it started.
    expect(one('feishu-service').textContent).toBe('The assistant talks to the service that Columbina uses when the assistant '
      + 'starts. After another service is chosen, stop the assistant and start it again.');
  });

  it('says that something did not go through while the assistant is connected', async () => {
    const error = 'an answer could not be sent to Feishu: Feishu did not do it: the bot is not in the chat (code 230002)';
    const { tell } = shellOfFeishu({ ...app, own: true, state: 'connected', paired: true, error });
    await drawn();
    expect(one('feishu-doing').textContent).toBe('Connected. The assistant answers you in Feishu.');
    expect(one('feishu-trouble').querySelector('p')?.textContent).toBe('Something did not go through lately.');
    expect(one('feishu-trouble').querySelector('details')?.textContent).toContain('the bot is not in the chat (code 230002).');
    // An answer that goes out again takes it away.
    await tell({ state: 'connected', paired: true, code: null, error: null, domain: 'feishu', app_id: 'cli_a1b2c3d4' });
    expect(marked('feishu-trouble')).toEqual([]);
  });

  it('says what went wrong while the code of pairing waits, too', async () => {
    shellOfFeishu({ ...app, own: true, state: 'connected', code: '123456', error: 'which messages were taken cannot be written down' });
    await drawn();
    expect(marked('feishu-pair')).toHaveLength(1);
    expect(marked('feishu-trouble')).toHaveLength(1);
  });

  it('names the platform of the app in what it says', async () => {
    const { tell } = shellOfFeishu({ ...app, domain: 'lark', own: true });
    await drawn();
    expect(one('feishu-doing').textContent).toBe('Connecting to Lark.');
    await tell({ state: 'connected', paired: false, code: '123456', error: null, domain: 'lark', app_id: 'cli_a1b2c3d4' });
    expect(one('feishu-pair').textContent).toContain('Send the bot this code in Lark');
    await tell({ state: 'connecting', paired: true, code: null, error: 'gone', domain: 'lark', app_id: 'cli_a1b2c3d4' });
    expect(one('feishu-doing').textContent).toBe('Lark cannot be reached just now. The assistant keeps trying.');
    await tell({ state: 'connected', paired: true, code: null, error: null, domain: 'lark', app_id: 'cli_a1b2c3d4' });
    expect(one('feishu-doing').textContent).toBe('Connected. The assistant answers you in Lark.');
  });

  it('asks again where the assistant tells of another app than the page knows of', async () => {
    const { shell, tell } = shellOfFeishu({ ...app, own: true });
    await drawn();
    expect(shell.feishuStatus).toHaveBeenCalledTimes(1);
    shell.feishuStatus.mockResolvedValueOnce({ ...app, app_id: 'cli_new12345', own: true });
    await tell({ state: 'connected', paired: false, code: '654321', error: null, domain: 'feishu', app_id: 'cli_new12345' });
    expect(shell.feishuStatus).toHaveBeenCalledTimes(2);
    expect(one('feishu-app').textContent).toBe('Feishucli_new12345');
  });

  it('says what did not go through when the assistant is started', async () => {
    const { shell } = shellOfFeishu();
    await drawn();
    shell.feishuStart.mockRejectedValueOnce(new Error('The assistant in Feishu could not be started: spawn ENOENT'));
    await press('feishu-start');
    expect(one('feishu-failed').querySelector('p')?.textContent).toBe('The assistant could not be started.');
    expect(one('feishu-failed').querySelector('details')?.textContent).toContain('spawn ENOENT');
    // Doing something else lets it go.
    await press('feishu-start');
    expect(marked('feishu-failed')).toEqual([]);
  });

  it('deletes the app once the user said so, and keeps it where the user did not', async () => {
    const { shell } = shellOfFeishu({ ...app, own: true, state: 'connected', paired: true });
    await drawn();
    await press('feishu-forget');
    expect(one('feishu-forget-ask').textContent).toContain('Delete the app cli_a1b2c3d4 on this computer?');
    expect(document.activeElement).toBe(one('feishu-keep'));
    expect(marked('feishu-forget')).toEqual([]);
    await press('feishu-keep');
    expect(marked('feishu-forget-ask')).toEqual([]);
    expect(shell.feishuForget).not.toHaveBeenCalled();

    await press('feishu-forget');
    shell.feishuStatus.mockResolvedValueOnce({ set_up: false, running: false, own: false, on: false, ended: null });
    await press('feishu-forget-sure');
    expect(shell.feishuForget).toHaveBeenCalledOnce();
    expect(one('feishu-forgotten').textContent).toBe('The app was deleted on this computer.');
    expect(marked('feishu-set-up')).toHaveLength(1);
  });

  it('says why the app could not be deleted, and still asks', async () => {
    const { shell } = shellOfFeishu();
    await drawn();
    shell.feishuForget.mockResolvedValueOnce({ error: 'the App Secret cannot be taken out of the keyring', code: 'STORE' });
    await press('feishu-forget');
    await press('feishu-forget-sure');
    expect(one('feishu-failed').querySelector('p')?.textContent).toBe('The app could not be deleted.');
    expect(one('feishu-failed').querySelector('details')?.textContent).toContain('The App Secret cannot be taken out of the keyring.');
    expect(marked('feishu-forget-ask')).toHaveLength(1);
  });

  it('says that how the assistant is doing could not be found out, and asks again', async () => {
    const { shell } = shellOfFeishu({ error: 'what is kept of the app cannot be read', code: 'STORE' });
    await drawn();
    expect(one('feishu-status-failed').querySelector('p')?.textContent).toBe(
      'Columbina could not find out how the assistant in Feishu is doing.',
    );
    shell.feishuStatus.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await press('feishu-check-again');
    expect(one('feishu-status-failed').querySelector('details')?.textContent).toContain('The setup program did not end in time.');
    shell.feishuStatus.mockResolvedValueOnce(app);
    await press('feishu-check-again');
    expect(one('feishu-app').textContent).toBe('Feishucli_a1b2c3d4');
  });

  it('says it in Chinese', async () => {
    shellOfFeishu({ ...app, own: true, state: 'connected', code: '123456' });
    await drawn('zh-CN');
    expect(one('feishu-pair').textContent).toContain('在飞书里给机器人发送这个配对码');
    expect(one('feishu-app').textContent).toBe('飞书cli_a1b2c3d4');
    expect(buttons()).toContain('停止');
  });
});
