// @vitest-environment jsdom
/**
 * The assistant in Feishu in the settings: what the page holds of it, against a shell that the
 * test plays, and the part of the settings that shows it. No App Secret comes this way.
 */
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AssistantPanel } from '../src/app/AssistantPanel';
import { Words } from '../src/app/locale';
import { messagesFor } from '../src/i18n';
import { useAssistant } from '../src/useAssistant';
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
    feishuWritable: vi.fn(async (_writable: boolean): Promise<unknown> => ({ writable: _writable })),
    openFeishuFolder: vi.fn(async () => {}),
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
    <AssistantPanel kind="feishu" language={language} onOpen={opened} />
  </Words.Provider>);
  await act(async () => {});
  return { ...result, opened };
}

describe('whether the assistant may change the files of its folder', () => {
  const folder = 'C:\\Users\\me\\Columbina\\feishu';
  const box = () => one('feishu-writable') as HTMLInputElement;

  it('is said, and changed at a click, after which the page asks again how the assistant is doing', async () => {
    const { shell } = shellOfFeishu({ ...app, folder, writable: false });
    await drawn();
    expect(box().checked).toBe(false);
    expect(box().disabled).toBe(false);
    expect(document.body.textContent).toContain('Let the assistant change the files in its folder');
    expect(document.body.textContent).toContain('It can then make and change files in its own folder, and still runs no commands.');
    shell.feishuStatus.mockResolvedValue({ ...app, folder, writable: true });
    await press('feishu-writable');
    expect(shell.feishuWritable.mock.calls).toEqual([[true]]);
    expect(shell.feishuStatus).toHaveBeenCalledTimes(2);
    expect(box().checked).toBe(true);
  });

  it('is not changed here where the assistant runs elsewhere', async () => {
    shellOfFeishu({ ...app, folder, running: true, state: 'connected', paired: true });
    await drawn();
    expect(box().disabled).toBe(true);
  });

  it('says where it could not be changed, and why', async () => {
    const { shell } = shellOfFeishu({ ...app, folder });
    shell.feishuWritable.mockResolvedValueOnce({ error: 'an assistant in Feishu runs on this computer', code: 'RUNNING' });
    await drawn();
    await press('feishu-writable');
    const failed = one('feishu-failed');
    expect(failed.textContent).toContain('Whether the assistant may change files could not be changed.');
    expect(failed.textContent).toContain('An assistant in Feishu runs on this computer.');
    shell.feishuWritable.mockRejectedValueOnce(new Error('The program did not answer.'));
    await press('feishu-writable');
    expect(one('feishu-failed').textContent).toContain('The program did not answer.');
    // What is done next says nothing of it any more.
    await press('feishu-folder');
    expect(marked('feishu-failed')).toEqual([]);
  });

  it('opens the folder of the assistant, which is named where it is pointed at', async () => {
    const { shell } = shellOfFeishu({ ...app, folder });
    await drawn();
    expect(one('feishu-folder').title).toBe(folder);
    expect(one('feishu-folder').textContent).toBe('Open its folder');
    await press('feishu-folder');
    expect(shell.openFeishuFolder).toHaveBeenCalledOnce();
    expect(marked('feishu-failed')).toEqual([]);
    shell.openFeishuFolder.mockRejectedValueOnce(new Error('No application opens it.'));
    await press('feishu-folder');
    expect(one('feishu-failed').textContent).toContain('The folder of the assistant could not be opened.');
    // A program that names no folder has none to open.
    cleanup();
    shellOfFeishu(app);
    await drawn();
    expect(marked('feishu-folder')).toEqual([]);
  });
});

describe('what the page holds of the assistant', () => {
  it('has nothing to ask where there is no shell, as in a replay', async () => {
    const { result } = renderHook(() => useAssistant('feishu', 'en'));
    expect(result.current.assistant).toEqual({ kind: 'unavailable' });
    await act(async () => {
      await result.current.refresh();
      await result.current.start();
      await result.current.stop();
      expect(await result.current.forget()).toBe(false);
      await result.current.setUp();
      result.current.showLog();
      await result.current.setWritable(true);
      await result.current.openFolder();
    });
    expect(result.current.assistant).toEqual({ kind: 'unavailable' });
    expect(result.current.failure).toBeNull();
  });

  it('takes what the assistant says over an answer that was asked for before it', async () => {
    const { shell, tell } = shellOfFeishu();
    const late = held<unknown>();
    const { result } = renderHook(() => useAssistant('feishu', 'en'));
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
    renderHook(() => useAssistant('feishu', 'en'));
    await act(async () => {});
    expect(shell.feishuStatus).toHaveBeenCalledTimes(1);
    shell.feishuStatus.mockResolvedValueOnce({ ...app, own: true });
    await tell({ state: 'connecting', paired: false, code: '123456', error: null });
    await act(async () => {});
    expect(shell.feishuStatus).toHaveBeenCalledTimes(2);
  });

  it('stops listening when it is gone', async () => {
    const { listening } = shellOfFeishu();
    const { unmount } = renderHook(() => useAssistant('feishu', 'en'));
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
    render(<AssistantPanel kind="feishu" language="en" onOpen={vi.fn()} />);
    expect(document.body.textContent).toBe('Asking how the assistant in Feishu is doing.');
    await act(async () => answer.release(app));
    cleanup();
    vi.unstubAllGlobals();
    render(<AssistantPanel kind="feishu" language="en" onOpen={vi.fn()} />);
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
    expect(one('feishu-app').querySelector('.assistant-light')?.hasAttribute('data-on')).toBe(false);
    expect(one('feishu-not-running').textContent).toBe('The assistant does not run.');
    expect(buttons()).toEqual([
      'Start the assistant', 'Change the app', 'Delete the app', 'Keep', 'Open the developer console', 'Show the log',
    ]);
    expect(document.body.textContent).toContain('While the assistant runs, closing the window keeps Columbina in the tray');
    // Of the service it is told once it runs.
    expect(marked('feishu-service')).toEqual([]);

    shell.feishuStatus.mockResolvedValueOnce({ ...app, own: true, on: true });
    await press('feishu-start');
    // What the assistant says by itself, it is to say in the language of the page.
    expect(shell.feishuStart.mock.calls).toEqual([[{ language: 'en' }]]);
    expect(one('feishu-doing').textContent).toBe('Connecting to Feishu.');
    expect(one('feishu-app').querySelector('.assistant-light')?.hasAttribute('data-on')).toBe(true);
    expect(buttons()).toEqual(['Stop', 'Change the app', 'Delete the app', 'Keep', 'Open the developer console', 'Show the log']);

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
    expect(document.body.textContent).not.toContain('While the assistant runs, closing the window keeps Columbina in the tray');
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

describe('the assistant in WeCom in the settings', () => {
  const bot = {
    set_up: true, bot_id: 'aibTestBot0123', paired: false, folder: 'C:\Users\me\Columbina\wecom', writable: false,
    running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
  };

  /** The shell as the page meets it, as far as the assistant in WeCom goes, beside one in Feishu that is set up. */
  function shellOfWecom(answer: unknown = bot) {
    const listeners = new Set<(said: Record<string, unknown>) => void>();
    const { shell: feishu } = shellOfFeishu();
    const shell = {
      ...feishu,
      wecomStatus: vi.fn(async (): Promise<unknown> => answer),
      wecomStart: vi.fn(async (_asked: { language: string }) => {}),
      wecomStop: vi.fn(async () => {}),
      wecomForget: vi.fn(async (): Promise<unknown> => ({ forgotten: true })),
      wecomLog: vi.fn(async () => {}),
      wecomWritable: vi.fn(async (_writable: boolean): Promise<unknown> => ({ writable: _writable })),
      openWecomFolder: vi.fn(async () => {}),
      setUpWecom: vi.fn(async (_asked: { language: string }) => ({ changed: false })),
      onWecom: vi.fn((listener: (said: Record<string, unknown>) => void) => {
        listeners.add(listener);
        return () => { listeners.delete(listener); };
      }),
    };
    vi.stubGlobal('columbina', shell);
    return {
      shell,
      tell: (...said: Record<string, unknown>[]) => act(() => {
        for (const document of said) for (const listener of [...listeners]) listener(document);
      }),
    };
  }

  async function drawnWecom(language = 'en') {
    const opened = vi.fn();
    render(<Words.Provider value={messagesFor(language)}>
      <AssistantPanel kind="wecom" language={language} onOpen={opened} />
    </Words.Provider>);
    await act(async () => {});
    return { opened };
  }

  it('offers to set up a bot where none is, and asks the shell of WeCom alone', async () => {
    const { shell } = shellOfWecom({ set_up: false, running: false, folder: bot.folder, own: false, on: false, ended: null });
    await drawnWecom();
    expect(document.body.textContent).toContain('Talk to Columbina in WeCom, through a smart bot of your own.');
    expect(buttons()).toEqual(['Set up a bot']);
    shell.wecomStatus.mockResolvedValueOnce(bot);
    shell.setUpWecom.mockResolvedValueOnce({ changed: true });
    await press('wecom-set-up');
    expect(shell.setUpWecom.mock.calls).toEqual([[{ language: 'en' }]]);
    expect(one('wecom-app').textContent).toBe('WeComaibTestBot0123');
    expect(shell.feishuStatus).not.toHaveBeenCalled();
    expect(shell.setUpFeishu).not.toHaveBeenCalled();
  });

  it('shows the bot that does not run, starts its assistant, and shows the code to send it in WeCom', async () => {
    const { shell, tell } = shellOfWecom();
    const { opened } = await drawnWecom();
    expect(one('wecom-not-running').textContent).toBe('The assistant does not run.');
    expect(buttons()).toEqual([
      'Start the assistant', 'Change the bot', 'Delete the bot', 'Keep', 'Open the admin console', 'Open its folder', 'Show the log',
    ]);
    shell.wecomStatus.mockResolvedValueOnce({ ...bot, own: true, on: true });
    await press('wecom-start');
    expect(shell.wecomStart.mock.calls).toEqual([[{ language: 'en' }]]);
    expect(shell.feishuStart).not.toHaveBeenCalled();
    expect(one('wecom-doing').textContent).toBe('Connecting to WeCom.');

    await tell({ state: 'connected', paired: false, code: '123456', error: null, bot_id: 'aibTestBot0123' });
    expect(one('wecom-code').textContent).toBe('123456');
    expect(one('wecom-pair').textContent).toContain('Send the bot this code in a chat of your own in WeCom');
    expect(one('wecom-not-heard').textContent).toContain('the bot uses the API mode with a long connection');
    await tell({ state: 'connected', paired: true, code: null, error: null, bot_id: 'aibTestBot0123' });
    expect(one('wecom-doing').textContent).toBe('Connected. The assistant answers you in WeCom.');

    fireEvent.click(one('wecom-console'));
    expect(opened.mock.calls).toEqual([['https://work.weixin.qq.com/wework_admin/frame']]);
    fireEvent.click(one('wecom-log'));
    expect(shell.wecomLog).toHaveBeenCalledOnce();
    await press('wecom-folder');
    expect(shell.openWecomFolder).toHaveBeenCalledOnce();
    expect(shell.openFeishuFolder).not.toHaveBeenCalled();
  });

  it('asks again where the assistant tells of another bot than the page knows of', async () => {
    const { shell, tell } = shellOfWecom({ ...bot, own: true, state: 'connected', paired: true });
    await drawnWecom();
    expect(shell.wecomStatus).toHaveBeenCalledTimes(1);
    await tell({ state: 'connected', paired: true, code: null, error: null, bot_id: 'aibOtherBot99' });
    await act(async () => {});
    expect(shell.wecomStatus).toHaveBeenCalledTimes(2);
  });

  it('stops the assistant, changes whether it may change files, and deletes the bot with the shell of WeCom', async () => {
    const { shell } = shellOfWecom({ ...bot, own: true, state: 'connected', paired: true });
    await drawnWecom();
    await press('wecom-stop');
    expect(shell.wecomStop).toHaveBeenCalledOnce();
    await press('wecom-writable');
    expect(shell.wecomWritable.mock.calls).toEqual([[true]]);
    await press('wecom-forget');
    expect(one('wecom-forget-ask').textContent).toContain('Delete the bot aibTestBot0123 on this computer?');
    shell.wecomStatus.mockResolvedValueOnce({ set_up: false, running: false, own: false, on: false, ended: null });
    await press('wecom-forget-sure');
    expect(shell.wecomForget).toHaveBeenCalledOnce();
    expect(one('wecom-forgotten').textContent).toBe('The bot was deleted on this computer.');
    expect([shell.feishuStop, shell.feishuWritable, shell.feishuForget].map(call => call.mock.calls.length)).toEqual([0, 0, 0]);
  });

  it('says what did not go through, as it does of the assistant in Feishu', async () => {
    const { shell } = shellOfWecom({ error: 'what is kept of the bot cannot be read', code: 'STORE' });
    await drawnWecom();
    expect(one('wecom-status-failed').querySelector('p')?.textContent).toBe(
      'Columbina could not find out how the assistant in WeCom is doing.',
    );
    shell.wecomStatus.mockResolvedValue(bot);
    await press('wecom-check-again');
    shell.wecomStart.mockRejectedValueOnce(new Error('The assistant in WeCom could not be started: spawn ENOENT'));
    await press('wecom-start');
    expect(one('wecom-failed').querySelector('p')?.textContent).toBe('The assistant could not be started.');
  });

  it('says it in Chinese', async () => {
    shellOfWecom({ ...bot, own: true, state: 'connected', code: '123456' });
    await drawnWecom('zh-CN');
    expect(one('wecom-pair').textContent).toContain('在企业微信里和机器人单聊，发送这个配对码');
    expect(one('wecom-app').textContent).toBe('企业微信aibTestBot0123');
    expect(one('wecom-not-heard').textContent).toContain('用的是「长连接」');
  });
});

describe('the assistant in QQ in the settings', () => {
  const bot = {
    set_up: true, app_id: '102030405', paired: false, folder: 'C:/Users/me/Columbina/qq',
    running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
  };

  /** The shell as the page meets it, as far as the assistant in QQ goes, beside one in Feishu that is set up. */
  function shellOfQq(answer: unknown = bot) {
    const listeners = new Set<(said: Record<string, unknown>) => void>();
    const { shell: feishu } = shellOfFeishu();
    const shell = {
      ...feishu,
      qqStatus: vi.fn(async (): Promise<unknown> => answer),
      qqStart: vi.fn(async (_asked: { language: string }) => {}),
      qqStop: vi.fn(async () => {}),
      qqForget: vi.fn(async (): Promise<unknown> => ({ forgotten: true })),
      qqLog: vi.fn(async () => {}),
      openQqFolder: vi.fn(async () => {}),
      setUpQq: vi.fn(async (_asked: { language: string }) => ({ changed: false })),
      onQq: vi.fn((listener: (said: Record<string, unknown>) => void) => {
        listeners.add(listener);
        return () => { listeners.delete(listener); };
      }),
    };
    vi.stubGlobal('columbina', shell);
    return {
      shell,
      tell: (...said: Record<string, unknown>[]) => act(() => {
        for (const document of said) for (const listener of [...listeners]) listener(document);
      }),
    };
  }

  async function drawnQq(language = 'en') {
    const opened = vi.fn();
    render(<Words.Provider value={messagesFor(language)}>
      <AssistantPanel kind="qq" language={language} onOpen={opened} />
    </Words.Provider>);
    await act(async () => {});
    return { opened };
  }

  it('offers to set up a bot where none is, and asks the shell of QQ alone', async () => {
    const { shell } = shellOfQq({ set_up: false, running: false, folder: bot.folder, own: false, on: false, ended: null });
    await drawnQq();
    expect(document.body.textContent).toContain('Chat with Columbina in QQ, through a bot of your own.');
    shell.qqStatus.mockResolvedValueOnce(bot);
    shell.setUpQq.mockResolvedValueOnce({ changed: true });
    await press('qq-set-up');
    expect(shell.setUpQq.mock.calls).toEqual([[{ language: 'en' }]]);
    expect(one('qq-app').textContent).toBe('QQ102030405');
    expect(shell.setUpFeishu).not.toHaveBeenCalled();
  });

  it('starts the assistant and shows the code, and offers no way to let it change files', async () => {
    const { shell, tell } = shellOfQq();
    const { opened } = await drawnQq();
    expect(buttons()).toEqual([
      'Start the assistant', 'Change the bot', 'Delete the bot', 'Keep', 'Open the open platform', 'Open its folder', 'Show the log',
    ]);
    expect(marked('qq-writable')).toEqual([]);
    shell.qqStatus.mockResolvedValueOnce({ ...bot, own: true, on: true });
    await press('qq-start');
    expect(shell.qqStart.mock.calls).toEqual([[{ language: 'en' }]]);

    await tell({ state: 'connected', paired: false, code: '123456', error: null, app_id: '102030405' });
    expect(one('qq-code').textContent).toBe('123456');
    expect(one('qq-pair').textContent).toContain('Send the bot this code in a chat of your own in QQ');
    expect(one('qq-not-heard').textContent).toContain('its events come by WebSocket');
    await tell({ state: 'connected', paired: true, code: null, error: null, app_id: '102030405' });
    expect(one('qq-doing').textContent).toBe('Connected. The assistant answers you in QQ.');

    fireEvent.click(one('qq-console'));
    expect(opened.mock.calls).toEqual([['https://q.qq.com/']]);
    fireEvent.click(one('qq-log'));
    expect(shell.qqLog).toHaveBeenCalledOnce();
    await press('qq-folder');
    expect(shell.openQqFolder).toHaveBeenCalledOnce();
    await press('qq-stop');
    expect(shell.qqStop).toHaveBeenCalledOnce();
  });

  it('deletes the bot with the shell of QQ', async () => {
    const { shell } = shellOfQq();
    await drawnQq();
    await press('qq-forget');
    expect(one('qq-forget-ask').textContent).toContain('Delete the bot 102030405 on this computer?');
    shell.qqStatus.mockResolvedValueOnce({ set_up: false, running: false, own: false, on: false, ended: null });
    await press('qq-forget-sure');
    expect(shell.qqForget).toHaveBeenCalledOnce();
    expect(one('qq-forgotten').textContent).toBe('The bot was deleted on this computer.');
  });

  it('says it in Chinese', async () => {
    shellOfQq({ ...bot, own: true, state: 'connected', code: '123456' });
    await drawnQq('zh-CN');
    expect(one('qq-pair').textContent).toContain('在 QQ 里和机器人私聊，发送这个配对码');
    expect(one('qq-app').textContent).toBe('QQ102030405');
  });
});

describe('the model of an assistant and how hard it thinks, in the settings', () => {
  const bot = {
    set_up: true, app_id: '102030405', paired: true, folder: 'C:/Users/me/Columbina/qq', model: null, effort: 'off',
    running: false, state: null, code: null, error: null, own: false, on: false, ended: null,
  };

  function shellOf(answer: unknown = bot) {
    const shell = {
      qqStatus: vi.fn(async (): Promise<unknown> => answer),
      qqThinking: vi.fn(async (asked: { model: string | null; effort: string | null }): Promise<unknown> => asked),
      onQq: vi.fn(() => () => {}),
    };
    vi.stubGlobal('columbina', shell);
    return shell;
  }

  async function drawnQq(language = 'en') {
    render(<Words.Provider value={messagesFor(language)}>
      <AssistantPanel kind="qq" language={language} onOpen={vi.fn()} />
    </Words.Provider>);
    await act(async () => {});
  }

  const model = () => one('qq-model') as HTMLInputElement;
  const effort = () => one('qq-effort') as HTMLSelectElement;
  const keep = () => one('qq-thinking-keep') as HTMLButtonElement;

  it('shows what was kept, and keeps a change, where empty follows what holds for all', async () => {
    const shell = shellOf();
    await drawnQq();
    expect([model().value, model().placeholder, effort().value]).toEqual(['', 'The model that Columbina uses', 'off']);
    expect([...effort().options].map(option => option.textContent)).toEqual([
      'As for all', 'As the service has it', 'Off', 'Low', 'High', 'The most',
    ]);
    expect(keep().disabled).toBe(true);
    fireEvent.change(model(), { target: { value: ' deepseek-v4-pro ' } });
    fireEvent.change(effort(), { target: { value: '' } });
    expect(keep().disabled).toBe(false);
    shell.qqStatus.mockResolvedValueOnce({ ...bot, model: 'deepseek-v4-pro', effort: null });
    await press('qq-thinking-keep');
    expect(shell.qqThinking.mock.calls).toEqual([[{ model: 'deepseek-v4-pro', effort: null }]]);
    expect([model().value, effort().value]).toEqual(['deepseek-v4-pro', '']);
    expect(keep().disabled).toBe(true);
  });

  it('says what the program refused', async () => {
    const shell = shellOf();
    shell.qqThinking.mockResolvedValueOnce({ error: 'a model is named by its id, as the service lists it', code: 'INVALID' });
    await drawnQq();
    fireEvent.change(effort(), { target: { value: 'high' } });
    await press('qq-thinking-keep');
    expect(one('qq-failed').querySelector('p')?.textContent).toBe('The model and the thinking of the assistant could not be changed.');
  });

  it('is changed where the assistant runs, and not here where it runs elsewhere', async () => {
    shellOf({ ...bot, running: true, own: false, state: 'connected' });
    await drawnQq();
    expect([model().disabled, effort().disabled, keep().disabled]).toEqual([true, true, true]);
  });

  it('says it in Chinese', async () => {
    shellOf();
    await drawnQq('zh-CN');
    expect(model().placeholder).toBe('跟随 Columbina 当前用的模型');
    expect(effort().selectedOptions[0]?.textContent).toBe('不思考');
  });
});
