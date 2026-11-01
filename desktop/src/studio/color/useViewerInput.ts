import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError, viewPointerOf, type ViewerTool, type WindowSelection, type WindowState, type WindowUnbound } from '../engine';
import type { Comparison } from './useComparison';

interface Capture {
  event: number; instance: number; id: number | null; terminal: 'up' | 'cancel' | null;
  finishing: Promise<void> | null; done: boolean; request: Promise<void> | null;
}
interface Owner {
  key: string; session: number; binding: number | null; sequence: number; lastCapture: number;
  capture: Capture | null; retired: boolean; pointer: boolean; bridge: Pick<EngineBridge, 'call'>;
  bindingPending: boolean; earlyRetirements: Map<number, WindowUnbound>; retryAt: number | null; retryScheduled: boolean;
}
export type WindowInputStatus = 'idle' | 'arming' | 'armed' | 'refused';

/** The Color viewer's sole native pointer subscription and ownership switch.
 * Window coordinates and hits belong to the engine; Image Wipe coordinates belong to comparison.
 */
export function useViewerInput({ bridge, session, native, loaded, timeline, tool, selection, intention, creating = false, comparison, viewer,
  onMenu, onWindowState }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; native: boolean; loaded: boolean; timeline: string | null;
  tool: ViewerTool; selection: WindowSelection | null; intention: number; creating?: boolean; comparison: Comparison;
  viewer: RefObject<HTMLElement | null>; onMenu: (point: { x: number; y: number }) => void;
  onWindowState: (state: WindowState) => void;
}) {
  const [status, setStatus] = useState<WindowInputStatus>('idle');
  const [capturing, setCapturing] = useState(false);
  const [closing, setClosing] = useState(false);
  const [retry, setRetry] = useState(0);
  const retiring = useRef(new Set<Capture>());
  const windowOn = native && loaded && !comparison.splitting && tool === 'window' && selection !== null;
  const cursor = native && loaded && (comparison.splitting || tool === 'image_wipe') ? comparison.pointerCursor : null;
  // Revision, server selection, edited object and palette interaction are deliberately absent.
  const key = JSON.stringify([session, timeline, windowOn ? [selection!.target, selection!.node, selection!.clip, intention] : null, cursor]);
  const now = useRef({ key, session, timeline, selection, windowOn, creating, comparison, onMenu, onWindowState });
  now.current = { key, session, timeline, selection, windowOn, creating, comparison, onMenu, onWindowState };
  const owner = useRef<Owner | null>(null);
  const switching = useRef<Promise<void>>(Promise.resolve());
  const nativeSeen = useRef({ session, instance: -1, event: -1 });
  if (nativeSeen.current.session !== session) nativeSeen.current = { session, instance: -1, event: -1 };
  const current = (of: Owner) => !of.retired && owner.current === of && now.current.key === of.key;

  const recover = useCallback((of: Owner) => {
    const at = now.current;
    if (!current(of) || of.retryAt === null || of.retryScheduled || !at.windowOn || !at.selection || at.selection.revision < of.retryAt) return;
    of.retryScheduled = true;
    // Use the same cleanup/activation queue as a user ownership switch, with the latest real copy.
    setRetry(value => value + 1);
  }, []);
  const unbound = useCallback((of: Owner, notice: WindowUnbound) => {
    if (!current(of) || notice.binding !== of.binding || notice.sequence <= of.sequence) return;
    of.sequence = notice.sequence;
    of.retryAt = notice.revision;
    // The server has ended any capture before retiring the binding, including a down whose reply is still pending.
    if (of.capture) { of.capture.done = true; of.capture = null; }
    setCapturing(false); setStatus('refused');
    recover(of);
  }, [recover]);

  const accept = useCallback((of: Owner, capture: Capture, answer: WindowState) => {
    if (!current(of) || of.retryAt !== null || capture.done || of.capture !== capture || answer.binding !== of.binding || answer.sequence <= of.sequence
      || answer.capture <= of.lastCapture || (capture.id !== null && answer.capture !== capture.id) || answer.event < capture.event) return;
    capture.id = answer.capture;
    of.sequence = answer.sequence;
    now.current.onWindowState(answer);
    if (answer.phase === 'ended' || answer.phase === 'refused') {
      capture.done = true; of.lastCapture = answer.capture; of.capture = null;
      setCapturing(false); setStatus(answer.phase === 'refused' ? 'refused' : 'idle');
    } else {
      setStatus(answer.phase);
      if (capture.terminal) void terminal(of, capture);
    }
  }, []);

  const terminal = useCallback((of: Owner, capture: Capture): Promise<void> => {
    if (capture.finishing) return capture.finishing;
    if (capture.done || capture.id === null || of.binding === null || capture.terminal === null) return Promise.resolve();
    capture.finishing = callEngine(of.bridge, 'window.input', { binding: of.binding, capture: capture.id, kind: capture.terminal },
      { expectedSession: of.session }).then(answer => { accept(of, capture, answer); }).catch(() => {
      if (current(of) && of.capture === capture) { of.capture = null; setCapturing(false); setStatus('refused'); }
    });
    return capture.finishing;
  }, [accept]);

  useEffect(() => {
    const of: Owner = { key, session, bridge, binding: null, sequence: -1, lastCapture: -1, capture: null, retired: false, pointer: false,
      bindingPending: false, earlyRetirements: new Map(), retryAt: null, retryScheduled: false };
    owner.current = of;
    setStatus('idle'); setCapturing(false); comparison.resetPointer();
    // Cleanup and activation share this queue. A late bind is unbound before its successor takes the pointer.
    switching.current = switching.current.then(async () => {
      if (!current(of)) return;
      let sentRevision: number | null = null;
      try {
        if (windowOn && selection) {
          const asked = now.current.selection!;
          sentRevision = asked.revision; of.bindingPending = true;
          const bound = await callEngine(bridge, 'window.bind', { ...asked }, { expectedSession: of.session });
          of.binding = bound.binding; of.sequence = bound.sequence;
          const retired = of.earlyRetirements.get(bound.binding);
          if (retired) unbound(of, retired);
        } else if (cursor !== null) {
          await callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: true, cursor }, { expectedSession: of.session });
          of.pointer = true;
        }
      } catch (failure) {
        if (current(of) && windowOn) {
          setStatus('refused');
          if (sentRevision !== null && failure instanceof EngineError && (failure.data as { reason?: string } | null)?.reason === 'stale_revision') {
            // The copy may have advanced before this refusal arrived. Never retry the same sent revision.
            of.retryAt = sentRevision + 1; recover(of);
          }
        }
      } finally { of.bindingPending = false; of.earlyRetirements.clear(); }
    });
    return () => {
      of.retired = true;
      const capture = of.capture;
      if (capture && !capture.done) {
        retiring.current.add(capture); setClosing(true);
        capture.terminal ??= 'up'; void terminal(of, capture);
      }
      comparison.resetPointer();
      switching.current = switching.current.then(async () => {
        // A down without a capture number must settle before unbind; its terminal intention survives.
        if (capture?.request) await capture.request;
        if (capture) await terminal(of, capture);
        try {
          if (of.binding !== null) await callEngine(bridge, 'window.unbind', { binding: of.binding }, { expectedSession: of.session });
          else if (of.pointer) await callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: false }, { expectedSession: of.session });
        } catch { /* The old session or binding may already have ended. */ }
        if (capture) { retiring.current.delete(capture); setClosing(retiring.current.size > 0); }
      });
    };
  }, [bridge, key, retry]);

  useEffect(() => { if (owner.current) recover(owner.current); }, [selection?.revision, windowOn, key, recover]);

  useEffect(() => bridge.onEvent(event => {
    const of = owner.current;
    if (!of || !current(of) || event.session !== of.session) return;
    if (event.kind === 'notification' && event.method === 'window.unbound') {
      const notice = event.params as WindowUnbound;
      if (of.bindingPending && of.binding === null) {
        // A retirement can overtake its bind reply. Keep candidates by binding; an old binding cannot poison the new one.
        const previous = of.earlyRetirements.get(notice.binding);
        if (!previous || notice.sequence > previous.sequence) of.earlyRetirements.set(notice.binding, notice);
      } else unbound(of, notice);
      return;
    }
    if (event.kind === 'notification' && event.method === 'window.state') {
      if (of.capture) accept(of, of.capture, event.params as WindowState);
      return;
    }
    const pointer = viewPointerOf(event);
    if (!pointer || pointer.monitor !== 'timeline' || (pointer.snapshot && pointer.snapshot.timeline !== now.current.timeline)) return;
    if (of.retryAt !== null) return;
    if (of.binding === null && !of.pointer) return;
    if (pointer.snapshot?.window && pointer.snapshot.window.binding !== of.binding) return;
    const seen = nativeSeen.current;
    if (pointer.instance !== undefined) {
      if (pointer.instance < seen.instance) return;
      if (pointer.instance > seen.instance) { seen.instance = pointer.instance; seen.event = -1; }
    }
    if (pointer.event !== undefined) { if (pointer.event <= seen.event) return; seen.event = pointer.event; }
    if (pointer.kind === 'down' && pointer.button === 'right') { now.current.onMenu(pointer.window); return; }
    if (pointer.kind === 'down' && pointer.button === 'left') viewer.current?.focus({ preventScroll: true });
    if (of.binding === null) { if (of.pointer) now.current.comparison.handlePointer(pointer); return; }
    if (pointer.kind === 'down') {
      if (pointer.button !== 'left' || of.capture || now.current.creating || pointer.event === undefined || pointer.instance === undefined
        || !pointer.presentation || pointer.snapshot?.window?.binding !== of.binding) return;
      const capture: Capture = { event: pointer.event, instance: pointer.instance, id: null, terminal: null, finishing: null, done: false, request: null };
      of.capture = capture; setCapturing(true); setStatus('arming');
      capture.request = callEngine(bridge, 'window.input', { binding: of.binding, kind: 'down', event: pointer.event },
        { expectedSession: of.session }).then(async answer => {
        if (answer.binding !== of.binding || answer.event < capture.event || answer.capture <= of.lastCapture) return;
        // Even a retired owner must learn its capture so its cleanup can finish it.
        if (capture.id === null) capture.id = answer.capture;
        accept(of, capture, answer);
        if (capture.terminal) await terminal(of, capture);
      }).catch(failure => {
        capture.done = true;
        if (current(of) && of.capture === capture) {
          of.capture = null; setCapturing(false);
          setStatus(failure instanceof EngineError && (failure.data as { reason?: string } | null)?.reason === 'hit' ? 'idle' : 'refused');
        }
      });
      return;
    }
    const capture = of.capture;
    if (!capture || pointer.instance !== capture.instance) return;
    if (pointer.kind === 'up' || pointer.kind === 'cancel') {
      // Native lost capture finishes; only Escape requests restoration.
      capture.terminal ??= 'up'; void terminal(of, capture);
    } else if (pointer.kind === 'move' && !capture.terminal && capture.id !== null && pointer.event !== undefined) {
      void callEngine(bridge, 'window.input', { binding: of.binding, capture: capture.id, kind: 'move', event: pointer.event },
        { expectedSession: of.session }).then(answer => accept(of, capture, answer)).catch(() => {});
    }
  }), [bridge, accept, terminal, unbound, viewer]);

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      const of = owner.current; const capture = of?.capture;
      if (event.key !== 'Escape' || !of || !current(of) || !capture || capture.done) return;
      event.preventDefault(); event.stopPropagation();
      capture.terminal = 'cancel'; void terminal(of, capture);
    };
    window.addEventListener('keydown', escape, true);
    return () => window.removeEventListener('keydown', escape, true);
  }, [terminal]);
  const busy = useCallback(() => owner.current?.capture != null || retiring.current.size > 0, []);
  return { status, capturing: capturing || closing, busy };
}
