import { useCallback, useEffect, useRef, useState } from 'react';
import { assistantAfter, assistantOf, type Assistant } from './app/feishu';

const said = (problem: unknown) => (problem instanceof Error ? problem.message : String(problem));

/** What the panel is doing with the assistant, while it does it. */
export type Busy = 'starting' | 'stopping' | 'forgetting' | 'settingUp';

/** What did not go through, and what the shell said of it. */
export interface Failure {
  what: 'start' | 'stop' | 'forget' | 'setUp';
  said: string;
}

/**
 * The assistant in Feishu, as the shell tells of it, and what the settings do with it: start and
 * stop it, set up its app in a window of its own, and delete the app. No App Secret comes this way.
 */
export function useFeishu(language: string) {
  const [assistant, setAssistant] = useState<Assistant>(window.columbina ? { kind: 'unknown' } : { kind: 'unavailable' });
  const [busy, setBusy] = useState<Busy | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  // That the app was deleted here, which is said until something else is done.
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
    const bridge = window.columbina;
    if (!bridge) return;
    const number = ++asked.current;
    let answer: Assistant;
    try {
      answer = assistantOf(await bridge.feishuStatus());
    } catch (problem) {
      answer = { kind: 'failed', said: said(problem) };
    }
    if (number === asked.current) put(answer);
  }, [put]);

  useEffect(() => {
    const bridge = window.columbina;
    if (!bridge) return undefined;
    const off = bridge.onFeishu((told) => {
      const after = assistantAfter(latest.current, told);
      // Something that belongs to no app that the page knows of: the page asks what there is.
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
  }, [refresh, put]);

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
    const bridge = window.columbina;
    if (!bridge) return;
    // What the assistant says by itself, such as its help, it says in the language of the page.
    await doing('starting', 'start', () => bridge.feishuStart({ language }));
    await refresh();
  }, [doing, refresh, language]);

  const stop = useCallback(async () => {
    const bridge = window.columbina;
    if (!bridge) return;
    await doing('stopping', 'stop', () => bridge.feishuStop());
    await refresh();
  }, [doing, refresh]);

  /** Stop the assistant and delete its app. True where it went through. */
  const forget = useCallback(async (): Promise<boolean> => {
    const bridge = window.columbina;
    if (!bridge) return false;
    const done = await doing('forgetting', 'forget', async () => {
      const answer = await bridge.feishuForget();
      const error = (answer as { error?: unknown } | null)?.error;
      if (typeof error === 'string') throw new Error(error);
    });
    if (done) setForgotten(true);
    await refresh();
    return done;
  }, [doing, refresh]);

  /** Open the window for the app, and ask again how the assistant is doing once it has closed. */
  const setUp = useCallback(async () => {
    const bridge = window.columbina;
    if (!bridge) return;
    await doing('settingUp', 'setUp', () => bridge.setUpFeishu({ language }));
    await refresh();
  }, [doing, refresh, language]);

  const showLog = useCallback(() => {
    void window.columbina?.feishuLog().catch(() => {});
  }, []);

  return { assistant, busy, failure, forgotten, refresh, start, stop, forget, setUp, showLog };
}

export type Feishu = ReturnType<typeof useFeishu>;
