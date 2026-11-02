import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError, viewPointerOf, type ViewerTool, type WindowSelection, type WindowState, type WindowUnbound,
  type Viewport, type PreparedViewport, type ViewNavigation, type WindowDeleted, type WindowTopology } from '../engine';
import type { Comparison } from './useComparison';
import type { ViewerPicker } from './pickers';
import type { WindowNotice } from './WindowPalette';

interface Capture {
  event: number; instance: number; id: number | null; terminal: 'up' | 'cancel' | null;
  finishing: Promise<void> | null; done: boolean; request: Promise<void> | null;
}
interface Owner {
  key: string; session: number; binding: number | null; sequence: number; lastCapture: number;
  capture: Capture | null; retired: boolean; pointer: boolean; bridge: Pick<EngineBridge, 'call'>;
  bindingPending: boolean; earlyRetirements: Map<number, WindowUnbound>; retryAt: number | null; retryScheduled: boolean;
  navigation: Promise<void>; navigationRevision: number; leftHeld: boolean; inheritedButtons: number; pan: Pan | null;
}
type Point = { x: number; y: number };
/** `deletes` is the native middle down event of an unmoved tap that may delete a Polygon point. */
interface Pan { instance: number; from: Point; prepared: PreparedViewport | null; valid: boolean; sent: Point; moved: boolean; deletes: number | null }
// Project defaults, to be calibrated with a real mouse in T29.
export const VIEWER_ZOOM_STEP = 1.1;
export const VIEWER_WHEEL_UNIT = 120;
export const VIEWER_ZOOM_CHOICES = [0.25, 0.5, 0.75, 1, 1.5, 2, 4];
const settingOf = (answer: Viewport): Viewport => answer.mode === 'fit' ? { mode: 'fit' } : { mode: 'scale', scale: answer.scale, center: answer.center };
const buttonBits = (buttons: readonly string[]) => (buttons.includes('left') ? 1 : 0) | (buttons.includes('middle') ? 4 : 0);
export type WindowInputStatus = 'idle' | 'arming' | 'armed' | 'refused';

/** The Color viewer's sole native pointer subscription and ownership switch.
 * Window coordinates and hits belong to the engine; Image Wipe coordinates belong to comparison.
 * A palette's picker of colour (`pickers.ts`), where one is in force, takes the left button before
 * a window or a wipe line does; a split screen keeps the pointer for its tiles.
 */
export function useViewerInput({ bridge, session, native, loaded, timeline, tool, selection, intention, creating = false, comparison, viewer,
  onMenu, onWindowState, onDeleted, onIdleEscape, picker = null }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; native: boolean; loaded: boolean; timeline: string | null;
  tool: ViewerTool; selection: WindowSelection | null; intention: number; creating?: boolean; comparison: Comparison;
  viewer: RefObject<HTMLElement | null>; onMenu: (point: { x: number; y: number }) => void;
  onWindowState: (state: WindowState) => void; onDeleted?: (answer: WindowDeleted) => void;
  /** Esc in the viewer with no capture; true when it was used (an unfinished Curve is left, keeping its draft). */
  onIdleEscape?: () => boolean;
  /** The picker of colour in force, which takes the pointer while it is. */
  picker?: ViewerPicker | null;
}) {
  const [status, setStatus] = useState<WindowInputStatus>('idle');
  // Why the engine refused a Polygon point insertion (at down) or a middle-click deletion.
  const [notice, setNotice] = useState<WindowNotice | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [closing, setClosing] = useState(false);
  const [retry, setRetry] = useState(0);
  const [viewport, setViewport] = useState<Viewport>({ mode: 'fit' });
  const retiring = useRef(new Set<Capture>());
  const picking = native && loaded && !comparison.splitting && picker !== null;
  const windowOn = native && loaded && !comparison.splitting && !picking && tool === 'window' && selection !== null && selection.visible && selection.outline;
  const cursor = native && loaded ? (picking ? picker.cursor
    : comparison.splitting || tool === 'image_wipe' ? comparison.pointerCursor ?? 'arrow' : 'arrow') : null;
  // Revision, server selection, edited object and palette interaction are deliberately absent.
  const key = JSON.stringify([session, timeline, tool, windowOn ? [selection!.target, selection!.node, selection!.clip, intention] : null, cursor]);
  const now = useRef({ key, session, timeline, selection, windowOn, creating, comparison, tool, onMenu, onWindowState, onDeleted, onIdleEscape, picker: picking ? picker : null });
  now.current = { key, session, timeline, selection, windowOn, creating, comparison, tool, onMenu, onWindowState, onDeleted, onIdleEscape, picker: picking ? picker : null };
  const owner = useRef<Owner | null>(null);
  const switching = useRef<Promise<void>>(Promise.resolve());
  const nativeSeen = useRef({ session, instance: -1, event: -1, buttons: 0 });
  if (nativeSeen.current.session !== session) nativeSeen.current = { session, instance: -1, event: -1, buttons: 0 };
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

  const enqueue = useCallback((of: Owner, instance: number | undefined, work: () => Promise<void>, confirmInherited = false) => {
    const capture = of.capture;
    const leftBlocked = () => of.leftHeld && !(confirmInherited && (of.inheritedButtons & 1) !== 0);
    of.navigationRevision += 1;
    of.navigation = of.navigation.then(async () => {
      if (!current(of) || of.retryAt !== null || leftBlocked() || (instance !== undefined && nativeSeen.current.instance !== instance)) return;
      // A physical Up may precede both the down reply and C2's terminal reply. Finish that
      // same capture normally before changing the map; navigation never requests Cancel.
      if (capture && !capture.done) { if (capture.request) await capture.request; await terminal(of, capture); }
      if (!current(of) || leftBlocked() || of.retryAt !== null || of.capture || (instance !== undefined && nativeSeen.current.instance !== instance)) return;
      await work();
    }).catch(() => { /* A replaced surface/context or an unavailable picture ends this intention. */ });
  }, [terminal]);

  const prepared = useCallback(async (of: Owner, instance?: number): Promise<PreparedViewport | null> => {
    const answer = await callEngine(of.bridge, 'view.viewport', { monitor: 'timeline', operation: 'prepare', ...(instance === undefined ? {} : { instance }) },
      { expectedSession: of.session });
    if (!current(of) || of.retryAt !== null || !answer.geometry || typeof answer.instance !== 'number'
      || answer.instance < nativeSeen.current.instance || (instance !== undefined && answer.instance !== instance)) return null;
    if (answer.instance > nativeSeen.current.instance) nativeSeen.current = { session: of.session, instance: answer.instance, event: -1, buttons: 0 };
    setViewport(settingOf(answer));
    return { ...answer, instance: answer.instance, geometry: answer.geometry };
  }, []);

  const publish = useCallback(async (of: Owner, request: ViewNavigation) => {
    if (!current(of) || nativeSeen.current.instance !== request.instance) return;
    const answer = await callEngine(of.bridge, 'view.viewport', request, { expectedSession: of.session });
    if (current(of) && of.retryAt === null && nativeSeen.current.instance === request.instance && (answer.mode === 'fit' || answer.mode === 'scale')) {
      setViewport(settingOf(answer));
    }
  }, []);

  const navigate = useCallback((intention: { operation: 'zoom'; factor: number; at?: Point } | { operation: 'scale'; scale: number } | { operation: 'fit' }, instance?: number) => {
    const of = owner.current;
    if (!of || !current(of) || of.pan || (of.binding === null && !of.pointer)) return;
    const held = nativeSeen.current.buttons & 5;
    const confirmInherited = held !== 0;
    if (confirmInherited && (of.capture || (held & ~of.inheritedButtons) !== 0)) return;
    // Pin a known instance when the intention is made, including keyboard/header work
    // waiting behind another RPC. Only the first no-mouse prepare may discover one.
    const expected = instance ?? (nativeSeen.current.instance < 0 ? undefined : nativeSeen.current.instance);
    enqueue(of, expected, async () => {
      // A later middle Down queues its baseline after already accepted navigation.
      // Only inherited-button confirmation must reject a pan started in the meantime.
      if (confirmInherited && of.pan) return;
      const event = nativeSeen.current.event;
      const ready = instance !== undefined && !confirmInherited ? { instance } : await prepared(of, expected);
      if (!ready) return;
      if (confirmInherited) {
        if (nativeSeen.current.event !== event || of.capture || of.pan || !('buttons' in ready) || !Array.isArray(ready.buttons)) return;
        nativeSeen.current.buttons = buttonBits(ready.buttons);
        of.leftHeld = (nativeSeen.current.buttons & 1) !== 0;
        of.inheritedButtons &= nativeSeen.current.buttons;
      }
      if (!of.leftHeld && (!confirmInherited || (nativeSeen.current.buttons & 4) === 0)) await publish(of, { monitor: 'timeline', instance: ready.instance, ...intention });
    }, confirmInherited);
  }, [enqueue, prepared, publish]);

  const movePan = useCallback((of: Owner, pan: Pan, at: Point, moved: boolean) => {
    pan.moved ||= moved || at.x !== pan.from.x || at.y !== pan.from.y;
    if (!pan.moved) return;
    enqueue(of, pan.instance, async () => {
      if (!pan.valid || !pan.prepared || (at.x === pan.sent.x && at.y === pan.sent.y)) return;
      try {
        await publish(of, { monitor: 'timeline', instance: pan.instance, operation: 'pan', base: settingOf(pan.prepared),
          geometry: pan.prepared.geometry, offset: { x: at.x - pan.from.x, y: at.y - pan.from.y } });
        pan.sent = at;
      } catch { pan.valid = false; }
    });
  }, [enqueue, publish]);

  /** The engine hit-tests the middle down's own frame; this only names that native event. */
  const remove = useCallback((of: Owner, event: number) => {
    if (!current(of) || of.binding === null || of.retryAt !== null || of.capture) return;
    callEngine(of.bridge, 'window.input', { binding: of.binding, kind: 'delete', event }, { expectedSession: of.session }).then(answer => {
      if (!current(of)) return;
      setNotice(null); now.current.onDeleted?.(answer as unknown as WindowDeleted);
    }).catch(failure => {
      const topology = failure instanceof EngineError ? (failure.data as { topology?: WindowTopology } | null)?.topology : undefined;
      if (current(of) && topology) setNotice({ action: 'delete', topology });
    });
  }, []);

  useEffect(() => {
    const of: Owner = { key, session, bridge, binding: null, sequence: -1, lastCapture: -1, capture: null, retired: false, pointer: false,
      bindingPending: false, earlyRetirements: new Map(), retryAt: null, retryScheduled: false,
      navigation: Promise.resolve(), navigationRevision: 0, leftHeld: (nativeSeen.current.buttons & 1) !== 0,
      inheritedButtons: nativeSeen.current.buttons, pan: null };
    owner.current = of;
    setStatus('idle'); setCapturing(false); setNotice(null); comparison.resetPointer();
    // Cleanup and activation share this queue. A late bind is unbound before its successor takes the pointer.
    switching.current = switching.current.then(async () => {
      if (!current(of)) return;
      const queriedAt = of.navigationRevision;
      void callEngine(bridge, 'view.viewport', { monitor: 'timeline' }, { expectedSession: of.session }).then(answer => {
        if (current(of) && of.navigationRevision === queriedAt && (answer.mode === 'fit' || answer.mode === 'scale')) setViewport(settingOf(answer));
      }).catch(() => {});
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
      if (of.pan) of.pan.valid = false;
      const capture = of.capture;
      if (capture && !capture.done) {
        retiring.current.add(capture); setClosing(true);
        capture.terminal ??= 'up'; void terminal(of, capture);
      }
      comparison.resetPointer();
      switching.current = switching.current.then(async () => {
        await of.navigation;
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
    const seen = nativeSeen.current;
    if (pointer.instance !== undefined) {
      if (pointer.instance < seen.instance) return;
      if (pointer.instance > seen.instance) { seen.instance = pointer.instance; seen.event = -1; seen.buttons = 0; of.leftHeld = false; of.inheritedButtons = 0; if (of.pan) of.pan.valid = false; of.pan = null; }
    }
    if (pointer.event !== undefined) { if (pointer.event <= seen.event) return; seen.event = pointer.event; }
    of.leftHeld = pointer.buttons.includes('left');
    seen.buttons = buttonBits(pointer.buttons);
    of.inheritedButtons &= seen.buttons;
    if (pointer.kind === 'down') of.inheritedButtons &= ~buttonBits(pointer.button ? [pointer.button] : []);
    // An on-boundary release can precede the activation reply. Keep physical state
    // current, but never dispatch that old gesture through a pending/new binding.
    if (of.retryAt !== null || (of.binding === null && !of.pointer)) return;
    if (pointer.snapshot?.window && pointer.snapshot.window.binding !== of.binding) return;
    if (pointer.kind === 'down' && pointer.button === 'right') { now.current.onMenu(pointer.window); return; }
    if (pointer.kind === 'down' && pointer.button === 'left') viewer.current?.focus({ preventScroll: true });
    if (pointer.kind === 'wheel') {
      if (!of.leftHeld && !pointer.buttons.includes('middle') && !of.pan && pointer.instance !== undefined && pointer.delta) {
        viewer.current?.focus({ preventScroll: true });
        navigate({ operation: 'zoom', factor: VIEWER_ZOOM_STEP ** (pointer.delta / VIEWER_WHEEL_UNIT),
          at: { x: pointer.window.x + 0.5, y: pointer.window.y + 0.5 } }, pointer.instance);
      }
      return;
    }
    if (pointer.kind === 'down' && pointer.button === 'middle') {
      const geometry = pointer.presentation?.map;
      if (of.leftHeld || of.pan || pointer.instance === undefined || !geometry) return;
      viewer.current?.focus({ preventScroll: true });
      // Only an idle Window binding's tap may delete; a tap during a closing left capture just navigates.
      const deletes = of.binding !== null && !of.capture && pointer.event !== undefined
        && pointer.snapshot?.window?.binding === of.binding ? pointer.event : null;
      const pan: Pan = { instance: pointer.instance, from: pointer.window, sent: pointer.window, prepared: null, valid: true, moved: false, deletes };
      of.pan = pan;
      enqueue(of, pan.instance, async () => {
        pan.prepared = await prepared(of, pan.instance);
        if (!pan.prepared || !(['surface', 'picture'] as const).every(key => geometry[key].every((side, i) => side === pan.prepared!.geometry[key][i]))) pan.valid = false;
      });
      return;
    }
    if (of.pan) {
      const pan = of.pan;
      if (pointer.kind === 'move' || pointer.kind === 'cancel' || (pointer.kind === 'up' && pointer.button === 'middle')) {
        movePan(of, pan, pointer.window, pointer.middle_moved ?? false);
        if (pointer.kind !== 'move') of.pan = null;
        // An unmoved release (native latch and position) is a tap: delete the Polygon point under its down.
        if (pointer.kind === 'up' && !pan.moved && pan.deletes !== null) remove(of, pan.deletes);
      }
      return;
    }
    // A tool/owner switch during an already-held middle button cannot turn that physical
    // drag into a Window or wipe drag when another button is pressed.
    if (pointer.kind === 'down' && pointer.button === 'left' && pointer.buttons.includes('middle')) return;
    if (of.binding === null) {
      if (of.pointer && now.current.picker) now.current.picker.onPointer(pointer);
      else if (of.pointer && (now.current.comparison.splitting || now.current.tool === 'image_wipe')) now.current.comparison.handlePointer(pointer);
      return;
    }
    if (pointer.kind === 'down') {
      if (pointer.button !== 'left' || of.capture || now.current.creating || pointer.event === undefined || pointer.instance === undefined
        || !pointer.presentation || pointer.snapshot?.window?.binding !== of.binding) return;
      const capture: Capture = { event: pointer.event, instance: pointer.instance, id: null, terminal: null, finishing: null, done: false, request: null };
      of.capture = capture; setCapturing(true); setStatus('arming'); setNotice(null);
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
          const data = failure instanceof EngineError ? failure.data as { reason?: string; topology?: WindowTopology } | null : null;
          // A refused Polygon edge insertion is explained, not reported as unavailable controls.
          if (data?.reason === 'insert' && data.topology) setNotice({ action: 'insert', topology: data.topology });
          setStatus(data?.reason === 'hit' || data?.reason === 'insert' ? 'idle' : 'refused');
        }
      });
      return;
    }
    const capture = of.capture;
    if (!capture || pointer.instance !== capture.instance) return;
    if ((pointer.kind === 'up' && pointer.button === 'left') || pointer.kind === 'cancel') {
      // Native lost capture finishes; only Escape requests restoration.
      capture.terminal ??= 'up'; void terminal(of, capture);
    } else if (pointer.kind === 'move' && !capture.terminal && capture.id !== null && pointer.event !== undefined) {
      void callEngine(bridge, 'window.input', { binding: of.binding, capture: capture.id, kind: 'move', event: pointer.event },
        { expectedSession: of.session }).then(answer => accept(of, capture, answer)).catch(() => {});
    }
  }), [bridge, accept, terminal, unbound, viewer, navigate, enqueue, prepared, movePan, remove]);

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      const target = event.target;
      if (event.defaultPrevented || event.metaKey || !(target instanceof Element) || !viewer.current?.contains(target)
        || target.closest('input,textarea,select,[contenteditable="true"]')) return;
      let intention: Parameters<typeof navigate>[0] | null = null;
      if (event.ctrlKey && !event.altKey && ['=', '+', '-'].includes(event.key)) intention = { operation: 'zoom', factor: event.key === '-' ? 1 / VIEWER_ZOOM_STEP : VIEWER_ZOOM_STEP };
      else if (!event.ctrlKey && event.shiftKey && (event.code === 'KeyZ' || event.key.toLowerCase() === 'z')) intention = event.altKey ? { operation: 'scale', scale: 1 } : { operation: 'fit' };
      if (intention) { event.preventDefault(); navigate(intention); }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [navigate, viewer]);

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      const of = owner.current; const capture = of?.capture;
      if (event.key !== 'Escape' || !of || !current(of)) return;
      if (!capture || capture.done) {
        // Idle: only within the viewer, and never while a closing capture is still retiring.
        const target = event.target;
        if (capture || retiring.current.size > 0 || of.binding === null || event.defaultPrevented
          || !(target instanceof Node) || !viewer.current?.contains(target)) return;
        if (now.current.onIdleEscape?.()) { event.preventDefault(); event.stopPropagation(); }
        return;
      }
      // During a drag Esc only rolls back that drag (an appended point with it); the draft stays.
      event.preventDefault(); event.stopPropagation();
      capture.terminal = 'cancel'; void terminal(of, capture);
    };
    window.addEventListener('keydown', escape, true);
    return () => window.removeEventListener('keydown', escape, true);
  }, [terminal, viewer]);
  const busy = useCallback(() => owner.current?.capture != null || retiring.current.size > 0, []);
  const setZoom = useCallback((value: number | 'fit') => navigate(value === 'fit' ? { operation: 'fit' } : { operation: 'scale', scale: value }), [navigate]);
  return { status, capturing: capturing || closing, busy, viewport, setZoom, notice };
}
