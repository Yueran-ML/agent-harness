import { useCallback, useEffect, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { refusalOf, type Refusal } from './refusal';

/** What a commit sends, worked out from the timeline as the engine has it when the commit is sent. */
export type Build = (timeline: Timeline) => Change[] | null;
export interface CommitOptions {
  /** Sentences that say a reason better in this panel than the general ones. */
  overrides?: Partial<Record<string, string>>;
  /** What to do with the longest transition that fits, where the engine refuses with one. */
  onLongest?: (frames: number) => void;
}
export interface Shown extends Refusal { onLongest?: (frames: number) => void }
interface Queued extends CommitOptions { build: Build; done: (sent: boolean) => void }
/** How long a commit waits for the copy to take in the one before it. */
const WAIT_MS = 2000;

/**
 * Commits of the inspector, one transaction each, in the order they were made. Each is worked out
 * from the timeline as it is when it goes, after the copy has taken in the one before, and carries
 * that revision; so two fields changed one after the other never undo each other, and a change by
 * someone else in between is refused as stale rather than overwritten.
 */
export function useCommit({ bridge, revision, session, timeline }: {
  bridge: Pick<EngineBridge, 'call'>; revision: number; session: number; timeline: Timeline | null;
}) {
  const words = useWords().hint.studio;
  const [refusal, setRefusal] = useState<Shown | null>(null);
  const [pending, setPending] = useState(false);
  const queue = useRef<Queued[]>([]);
  const awaited = useRef(-Infinity);
  const busy = useRef(false);
  const live = useRef(true);
  const waiting = useRef<ReturnType<typeof setTimeout> | null>(null);
  const now = useRef({ bridge, revision, session, timeline, words });
  now.current = { bridge, revision, session, timeline, words };

  const pump = useCallback((): void => {
    if (busy.current) return;
    const context = now.current;
    // The copy has not yet taken in the commit before: wait for it, so this one is worked out from it.
    // Where it never comes (the copy is fetched anew), go after a while: the engine refuses a stale one.
    if (context.revision < awaited.current) {
      if (queue.current.length > 0 && waiting.current === null) {
        waiting.current = setTimeout(() => { waiting.current = null; awaited.current = -Infinity; pump(); }, WAIT_MS);
      }
      return;
    }
    if (waiting.current !== null) { clearTimeout(waiting.current); waiting.current = null; }
    const item = queue.current.shift();
    if (!item) return;
    const ops = context.timeline ? item.build(context.timeline) : null;
    if (!ops || ops.length === 0) { item.done(Boolean(context.timeline)); pump(); return; }
    busy.current = true; setPending(true);
    const { session: from } = context;
    callEngine(context.bridge, 'transaction.apply', { revision: context.revision, ops })
      .then(applied => {
        if (typeof applied?.revision === 'number') awaited.current = applied.revision;
        item.done(true);
      })
      .catch((error: unknown) => {
        if (live.current && from === now.current.session) setRefusal({ ...refusalOf(error, now.current.words, item.overrides), onLongest: item.onLongest });
        item.done(false);
      })
      .finally(() => {
        busy.current = false;
        if (live.current) setPending(false);
        pump();
      });
  }, []);

  useEffect(() => {
    live.current = true;
    return () => { live.current = false; if (waiting.current !== null) clearTimeout(waiting.current); waiting.current = null; };
  }, []);
  useEffect(() => { pump(); }, [revision, pump]);
  useEffect(() => {
    for (const item of queue.current) item.done(false);
    queue.current = []; awaited.current = -Infinity; setRefusal(null);
  }, [session, timeline?.id]);

  /** Send one transaction; true once the engine took it, false where it was refused or dropped. */
  const commit = useCallback((build: Build, options: CommitOptions = {}) => new Promise<boolean>(resolve => {
    setRefusal(null);
    queue.current.push({ build, ...options, done: resolve });
    pump();
  }), [pump]);
  return { commit, refusal, setRefusal, pending };
}
export type Commit = ReturnType<typeof useCommit>['commit'];
