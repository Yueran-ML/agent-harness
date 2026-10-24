import { useCallback, useEffect, useRef, useState } from 'react';
import { assistantAfter, assistantOf, type Assistant, type Kind } from './app/assistant';
import type { ColumbinaBridge } from './bridge';

const said = (problem: unknown) => (problem instanceof Error ? problem.message : String(problem));

/** What the panel is doing with the assistant, while it does it. */
export type Busy = 'starting' | 'stopping' | 'forgetting' | 'settingUp' | 'changing';

/** What did not go through, and what the shell said of it. */
export interface Failure {
  what: 'start' | 'stop' | 'forget' | 'setUp' | 'writable' | 'folder';
  said: string;
}

/** What the shell is asked of an assistant, by what the assistant is. */
function callsOf(bridge: ColumbinaBridge, kind: Kind) {
  return kind === 'feishu' ? {
    status: () => bridge.feishuStatus(),
    start: (asked: { language: string }) => bridge.feishuStart(asked),
    stop: () => bridge.feishuStop(),
    forget: () => bridge.feishuForget(),
    log: () => bridge.feishuLog(),
    writable: (writable: boolean) => bridge.feishuWritable(writable),
    folder: () => bridge.openFeishuFolder(),
    setUp: (asked: { language: string }) => bridge.setUpFeishu(asked),
    on: (listener: (told: Record<string, unknown>) => void) => bridge.onFeishu(listener),
  } : kind === 'wecom' ? {
    status: () => bridge.wecomStatus(),
    start: (asked: { language: string }) => bridge.wecomStart(asked),
    stop: () => bridge.wecomStop(),
    forget: () => bridge.wecomForget(),
    log: () => bridge.wecomLog(),
    writable: (writable: boolean) => bridge.wecomWritable(writable),
    folder: () => bridge.openWecomFolder(),
    setUp: (asked: { language: string }) => bridge.setUpWecom(asked),
    on: (listener: (told: Record<string, unknown>) => void) => bridge.onWecom(listener),
  } : {
    status: () => bridge.qqStatus(),
    start: (asked: { language: string }) => bridge.qqStart(asked),
    stop: () => bridge.qqStop(),
    forget: () => bridge.qqForget(),
    log: () => bridge.qqLog(),
    // The agents of the assistant in QQ change nothing, and nothing asks it to.
    writable: async (writable: boolean) => ({ writable }),
    folder: () => bridge.openQqFolder(),
    setUp: (asked: { language: string }) => bridge.setUpQq(asked),
    on: (listener: (told: Record<string, unknown>) => void) => bridge.onQq(listener),
  };
}

/** The calls for an assistant, where there is a shell to call. */
const shellFor = (kind: Kind) => (window.columbina ? callsOf(window.columbina, kind) : null);

/**
 * An assistant in Feishu or in WeCom, as the shell tells of it, and what the settings do with it:
 * start and stop it, set up its app or bot in a window of its own, and delete that. No secret
 * comes this way.
 */
export function useAssistant(kind: Kind, language: string) {
  const [assistant, setAssistant] = useState<Assistant>(window.columbina ? { kind: 'unknown' } : { kind: 'unavailable' });
  const [busy, setBusy] = useState<Busy | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  // That the app or the bot was deleted here, which is said until something else is done.
  const [forgotten, setForgotten] = useState(false);
  // What the page knows last, also between two renders: what the assistant says is read against it.
  const latest = useRef(assistant);
  const put = useCallback((next: Assistant) => {
    latest.current = next;
    setAssistant(next);
  }, []);
  // The asks of how the assistant is doing, by number. Only the answer to the last one counts,
  // and none to one that was asked before the assistant said something of itself.
  const asked = useRef(0);

  const refresh = useCallback(async () => {
    const shell = shellFor(kind);
    if (!shell) return;
    const number = ++asked.current;
    let answer: Assistant;
    try {
      answer = assistantOf(kind, await shell.status());
    } catch (problem) {
      answer = { kind: 'failed', said: said(problem) };
    }
    if (number === asked.current) put(answer);
  }, [kind, put]);

  useEffect(() => {
    const shell = shellFor(kind);
    if (!shell) return undefined;
    const off = shell.on((told) => {
      const after = assistantAfter(kind, latest.current, told);
      // Something that belongs to nothing that the page knows of: the page asks what there is.
      if (after === null) {
        void refresh();
        return;
      }
      // What the assistant says goes before any answer that was asked for earlier.
      asked.current += 1;
      put(after);
    });
    void refresh();
    return off;
  }, [kind, refresh, put]);

  /** Do something with the assistant, and say what did not go through. */
  const doing = useCallback(async (now: Busy, what: Failure['what'], act: () => Promise<unknown>) => {
    setBusy(now);
    setFailure(null);
    setForgotten(false);
    try {
      await act();
      return true;
    } catch (problem) {
      setFailure({ what, said: said(problem) });
      return false;
    } finally {
      setBusy(null);
    }
  }, []);

  const start = useCallback(async () => {
    const shell = shellFor(kind);
    if (!shell) return;
    // What the assistant says by itself, such as its help, it says in the language of the page.
    await doing('starting', 'start', () => shell.start({ language }));
    await refresh();
  }, [kind, doing, refresh, language]);

  const stop = useCallback(async () => {
    const shell = shellFor(kind);
    if (!shell) return;
    await doing('stopping', 'stop', () => shell.stop());
    await refresh();
  }, [kind, doing, refresh]);

  /** Stop the assistant and delete its app or bot. True where it went through. */
  const forget = useCallback(async (): Promise<boolean> => {
    const shell = shellFor(kind);
    if (!shell) return false;
    const done = await doing('forgetting', 'forget', async () => {
      const answer = await shell.forget();
      const error = (answer as { error?: unknown } | null)?.error;
      if (typeof error === 'string') throw new Error(error);
    });
    if (done) setForgotten(true);
    await refresh();
    return done;
  }, [kind, doing, refresh]);

  /** Open the window for the app or the bot, and ask again how the assistant is doing once it has closed. */
  const setUp = useCallback(async () => {
    const shell = shellFor(kind);
    if (!shell) return;
    await doing('settingUp', 'setUp', () => shell.setUp({ language }));
    await refresh();
  }, [kind, doing, refresh, language]);

  const showLog = useCallback(() => {
    void shellFor(kind)?.log().catch(() => {});
  }, [kind]);

  /** Say whether the agent of the assistant may change the files of its folder. One that runs starts again for it. */
  const setWritable = useCallback(async (writable: boolean) => {
    const shell = shellFor(kind);
    if (!shell) return;
    await doing('changing', 'writable', async () => {
      const answer = await shell.writable(writable);
      const error = (answer as { error?: unknown } | null)?.error;
      if (typeof error === 'string') throw new Error(error);
    });
    await refresh();
  }, [kind, doing, refresh]);

  /** Open the folder of the assistant, as the program names it. */
  const openFolder = useCallback(async () => {
    const shell = shellFor(kind);
    if (!shell) return;
    setFailure(null);
    try {
      await shell.folder();
    } catch (problem) {
      setFailure({ what: 'folder', said: said(problem) });
    }
  }, [kind]);

  return { assistant, busy, failure, forgotten, refresh, start, stop, forget, setUp, showLog, setWritable, openFolder };
}

export type AssistantHandle = ReturnType<typeof useAssistant>;
