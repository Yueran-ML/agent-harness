/**
 * How the Color page changes grades: each command one transaction of the engine, which one undo
 * takes back, in the one history that the Edit page and the agent share; and why, in the words of
 * the Edit page (`hint.studio.edit`), the engine refused one. As on the Edit page, one transaction
 * is sent at a time: an undo or a redo asked for meanwhile is done after it, another change is not
 * sent, and the page says to try again.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Applied } from '../engine';
import { refusalText } from '../inspector/refusal';
import { isRefused, type Worked } from './nodeEdits';

export type HistoryMethod = 'history.undo' | 'history.redo';

export interface GradeEditing {
  /**
   * Send a command worked out by `nodeEdits`: its changes as one transaction, then `then` with
   * what the engine did; or, where the editor refused it itself, say why. Nothing to do sends nothing.
   */
  run: (worked: Worked, then?: (applied: Applied) => void) => void;
  /** Undo or redo in the one history of the engine (Ctrl+Z, Ctrl+Shift+Z). */
  step: (method: HistoryMethod) => void;
  /** Why the last change was not made, or null. */
  problem: string | null;
  setProblem: (problem: string | null) => void;
}

export function useGradeEditing({ bridge, revision, session }: {
  bridge: Pick<EngineBridge, 'call'>;
  revision: number;
  session: number;
}): GradeEditing {
  const words = useWords().hint.studio;
  const [problem, setProblem] = useState<string | null>(null);
  const now = useRef({ bridge, revision, session, words });
  now.current = { bridge, revision, session, words };
  const busy = useRef(false);
  const queued = useRef<HistoryMethod[]>([]);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => { setProblem(null); queued.current = []; }, [session]);

  const send = useCallback(async (method: 'transaction.apply' | HistoryMethod, params?: Record<string, unknown>, then?: (applied: Applied) => void) => {
    if (busy.current) {
      if (method !== 'transaction.apply') queued.current.push(method);
      else if (live.current) setProblem(now.current.words.edit.pending);
      return;
    }
    const asked = now.current;
    busy.current = true;
    setProblem(null);
    try {
      const answer = await callEngine(asked.bridge, method, params);
      if (then && live.current && asked.session === now.current.session) then(answer as Applied);
    } catch (error) {
      queued.current = [];
      if (live.current && asked.session === now.current.session) setProblem(refusalText(error, now.current.words));
    } finally {
      busy.current = false;
    }
    const next = queued.current.shift();
    if (next && live.current && asked.session === now.current.session) void send(next);
  }, []);

  const run = useCallback((worked: Worked, then?: (applied: Applied) => void) => {
    if (worked === null) return;
    if (isRefused(worked)) { setProblem(now.current.words.edit[worked.refused]); return; }
    if (worked.length === 0) return;
    void send('transaction.apply', { revision: now.current.revision, ops: worked }, then);
  }, [send]);
  const step = useCallback((method: HistoryMethod) => { void send(method); }, [send]);
  return { run, step, problem, setProblem };
}
