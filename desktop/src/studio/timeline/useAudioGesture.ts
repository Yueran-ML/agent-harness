import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine } from '../engine';
import type { Change } from './commands';

interface Context { bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; identity: string; locked: boolean; onFailure: (error: unknown) => void }
interface Gesture {
  context: Context; id: string; confirmed: number; restore: Change[]; last: Change[];
  next: Change[] | null; sending: boolean; sent: boolean; ending: boolean; failed: boolean;
  idle: ReturnType<typeof setTimeout> | null;
}
const renderer = crypto.randomUUID();
let sequence = 0;
// The server closes a wire gesture after 10 s without transactions (session.rs GESTURE_IDLE).
const KEEP_ALIVE_MS = 5000;
function clearIdle(g: Gesture) { if (g.idle !== null) clearTimeout(g.idle); g.idle = null; }

/** A small audio transaction lane. The start revision is never rebased, including on cancellation.
 * Only one request is in flight; subsequent pointer moves replace its unsent successor. */
export function useAudioGesture(context: Context) {
  const now = useRef(context); now.current = context;
  const owner = useRef<Gesture | null>(null);
  const confirmed = useRef(context.revision);
  const identity = useRef({ session: context.session, value: context.identity });
  const live = useRef(true);
  const [, redraw] = useState(0);
  const refresh = () => { if (live.current) redraw(n => n + 1); };
  const external = (g: Gesture) => now.current.session === g.context.session && now.current.identity === g.context.identity
    && now.current.revision > g.confirmed;
  const pump = (g: Gesture): void => {
    if (g.sending || g.failed || !g.next) return;
    // A notification can arrive before our reply. Check only after that reply identifies our revision.
    if (external(g)) { g.failed = true; g.next = null; clearIdle(g); refresh(); return; }
    const ops = g.next, end = g.ending;
    clearIdle(g);
    g.next = null; g.sending = true; g.sent = true; g.last = ops;
    void callEngine(g.context.bridge, 'transaction.apply', {
      revision: g.context.revision, ops, gesture: { id: g.id, ...(end ? { end: true } : {}) },
    }, { expectedSession: g.context.session }).then(result => {
      g.confirmed = result.revision;
      if (owner.current === g) confirmed.current = result.revision;
    }).catch(error => {
      g.failed = true; g.next = null;
      if (live.current && now.current.session === g.context.session) now.current.onFailure(error);
    }).finally(() => {
      g.sending = false;
      if (end || g.failed || external(g)) {
        g.failed = true; g.next = null; clearIdle(g);
        if (owner.current === g) owner.current = null;
      } else if (g.next) pump(g);
      else if (live.current && owner.current === g && !g.ending) {
        // Replaying an absolute preview is a real write in the same undo step. It keeps a held
        // pointer cancellable without rebasing; queued motion, release and cancel always take priority.
        g.idle = setTimeout(() => {
          g.idle = null;
          if (!live.current || owner.current !== g || g.ending || g.failed || g.sending || g.next) return;
          g.next = g.last; pump(g);
        }, KEEP_ALIVE_MS);
      }
      refresh();
    });
  };
  const cancel = () => {
    const g = owner.current;
    if (!g || g.failed) return;
    clearIdle(g);
    if (!g.sent) { owner.current = null; refresh(); return; }
    g.next = g.restore; g.ending = true; pump(g);
  };
  useEffect(() => {
    live.current = true;
    const blur = () => cancel();
    window.addEventListener('blur', blur);
    return () => { live.current = false; window.removeEventListener('blur', blur); cancel(); };
  }, []);
  useEffect(() => {
    const changed = identity.current.session !== context.session || identity.current.value !== context.identity;
    identity.current = { session: context.session, value: context.identity };
    if (changed) confirmed.current = context.revision;
    const old = owner.current;
    if (old && (old.context.identity !== context.identity || old.context.session !== context.session || context.locked)) {
      cancel(); owner.current = null; confirmed.current = context.revision;
    } else if (old && !old.sending && external(old)) { old.failed = true; old.next = null; clearIdle(old); owner.current = null; refresh(); }
    if (!owner.current) confirmed.current = Math.max(confirmed.current, context.revision);
  }, [context.identity, context.session, context.revision, context.locked]);
  const start = (restore: Change[]) => {
    if (context.locked || owner.current || context.revision < confirmed.current) return false;
    owner.current = { context, id: `audio-${renderer}-${++sequence}`, confirmed: context.revision, restore, last: restore,
      next: null, sending: false, sent: false, ending: false, failed: false, idle: null };
    return true;
  };
  return {
    start, cancel,
    /** Includes an ending transaction waiting for its reply, for history-command ordering. */
    busy: () => Boolean(owner.current && !owner.current.failed),
    active: () => Boolean(owner.current && !owner.current.failed && !owner.current.ending),
    preview: (ops: Change[]) => { const g = owner.current; if (g && !g.failed && !g.ending) { g.next = ops; pump(g); } },
    end: () => { const g = owner.current; if (!g || g.failed) return; clearIdle(g); if (!g.sent) { owner.current = null; return; } g.next ??= g.last; g.ending = true; pump(g); },
    commit: (ops: Change[]) => { if (!ops.length || !start([])) return; const g = owner.current!; g.next = ops; g.ending = true; pump(g); },
  };
}
export type AudioGesture = ReturnType<typeof useAudioGesture>;
