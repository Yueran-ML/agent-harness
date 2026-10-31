/**
 * A scope of the engine for a pane of the page (PROTOCOL.md, "Scopes"): subscribed while the pane
 * is shown, set anew when what it asks for changes, let go when the pane is hidden or closed, and
 * its pictures drawn into the pane's canvas as they come, at most once a frame of the screen.
 *
 * The scopes of a page share the engine's connection of the shell, which holds 16 at once. The
 * panes take places among those 16 here before they subscribe; one that finds none waits, says
 * so, and takes the first that is given back.
 */
import { useEffect, useRef, useState, type RefObject } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError, type EngineEvent, type Monitor, type ScopeAnswer, type ScopeFrame, type ScopeGraticule } from '../engine';
import { drawPicture, MOST_SCOPES } from './scopeSettings';

export type ScopeBridge = Pick<EngineBridge, 'call' | 'onEvent'>;

/**
 * How a pane's scope stands: not asked for (`idle`); waiting for a place among the 16
 * (`tooMany`); subscribed, its first picture not come (`preparing`: the engine may be compiling
 * its shaders, which takes long the first time); drawn (`shown`); refused by an engine without
 * monitors (`unavailable`); or failed.
 */
export type ScopeStatus = 'idle' | 'preparing' | 'shown' | 'tooMany' | 'unavailable' | 'failed';

/** The scopes of the page on one bridge: who holds a place among the 16, and which pane each picture goes to. */
export class ScopeHub {
  private readonly held = new Set<object>();
  private readonly waiting = new Set<() => void>();
  private readonly takers = new Map<string, (frame: ScopeFrame) => void>();
  private stop: (() => void) | null = null;

  constructor(private readonly bridge: Pick<EngineBridge, 'onEvent'>, private readonly most = MOST_SCOPES) {}

  /** How many places are held. */
  get count(): number { return this.held.size; }

  /** Take a place for `holder`, or false where all are held. */
  take(holder: object): boolean {
    if (this.held.has(holder)) return true;
    if (this.held.size >= this.most) return false;
    this.held.add(holder);
    return true;
  }

  /** Give a place back, and wake those that wait for one. */
  give(holder: object): void {
    if (!this.held.delete(holder)) return;
    for (const wake of [...this.waiting]) wake();
  }

  /** Be woken when a place is given back; returns what stops it. */
  wait(wake: () => void): () => void {
    this.waiting.add(wake);
    return () => { this.waiting.delete(wake); };
  }

  /** Have the pictures of scope `id` of engine `session` go to `taker`; returns what stops it. */
  listen(session: number, id: string, taker: (frame: ScopeFrame) => void): () => void {
    const key = `${session}:${id}`;
    this.takers.set(key, taker);
    this.stop ??= this.bridge.onEvent(event => this.heard(event));
    return () => {
      if (this.takers.get(key) === taker) this.takers.delete(key);
      if (this.takers.size === 0 && this.stop) { this.stop(); this.stop = null; }
    };
  }

  private heard(event: EngineEvent): void {
    if (event.kind !== 'notification' || event.method !== 'scopes.frame') return;
    const frame = event.params as Partial<ScopeFrame> | null;
    if (typeof frame?.scope_id !== 'string') return;
    this.takers.get(`${event.session}:${frame.scope_id}`)?.(frame as ScopeFrame);
  }
}

const hubs = new WeakMap<object, ScopeHub>();
/** The hub of the scopes of `bridge`. */
export function hubOf(bridge: Pick<EngineBridge, 'onEvent'>): ScopeHub {
  let hub = hubs.get(bridge);
  if (!hub) { hub = new ScopeHub(bridge); hubs.set(bridge, hub); }
  return hub;
}

const isAnswer = (value: unknown): value is ScopeAnswer => {
  const answer = value as Partial<ScopeAnswer> | null;
  return typeof answer?.scope_id === 'string' && typeof answer.size?.width === 'number' && typeof answer.size.height === 'number'
    && Array.isArray(answer.graticule?.lines) && Array.isArray(answer.graticule.labels);
};
const reasonOf = (failure: unknown) => (failure instanceof EngineError && typeof failure.data === 'object' && failure.data !== null
  ? (failure.data as { reason?: unknown }).reason : undefined);

/** What a pane draws of its scope: how it stands, its graticule, and the canvas its pictures are drawn into. */
export interface ScopeShown {
  status: ScopeStatus;
  graticule: ScopeGraticule | null;
  canvas: RefObject<HTMLCanvasElement | null>;
}

/**
 * Keep a scope of `view` with `settings` for a pane while `active` holds and an engine is open
 * (`session`). `settings` null is a pane not measured yet, which asks for nothing.
 */
export function useScope({ bridge, session, view, settings, active }: {
  bridge: ScopeBridge; session: number | null; view: Monitor; settings: Record<string, unknown> | null; active: boolean;
}): ScopeShown {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const [status, setStatus] = useState<ScopeStatus>('idle');
  const [graticule, setGraticule] = useState<ScopeGraticule | null>(null);
  const key = settings === null ? null : JSON.stringify(settings);
  const latest = useRef({ key, settings });
  latest.current = { key, settings };
  // The engine the page has now: a scope of another is not this connection's to let go.
  const sessionNow = useRef(session);
  sessionNow.current = session;
  const sync = useRef<(() => void) | null>(null);
  const ready = active && session !== null && key !== null;

  useEffect(() => {
    if (!ready || session === null) { setStatus('idle'); return undefined; }
    const hub = hubOf(bridge);
    const holder = {};
    let id: string | null = null;
    let sent: string | null = null;
    let kind: unknown = null;
    let size: { width: number; height: number } | null = null;
    let busy = false;
    let ended = false;
    let pending: ScopeFrame | null = null;
    let painting = 0;
    let stopListening = () => {};
    let stopWaiting = () => {};

    const took = (answer: ScopeAnswer, asked: string | null) => {
      sent = asked;
      size = answer.size;
      setGraticule(answer.graticule);
      // Another scope is a new picture, maybe of shaders not compiled yet: it is prepared until it comes.
      if (answer.settings?.kind !== kind) { kind = answer.settings?.kind; setStatus('preparing'); }
    };
    const paint = () => {
      painting = 0;
      const frame = pending;
      pending = null;
      if (ended || frame === null || canvas.current === null) return;
      // A picture of the settings before those answered last is passed over.
      if (size !== null && (frame.width !== size.width || frame.height !== size.height)) return;
      if (drawPicture(canvas.current, frame.rgba, frame.width, frame.height)) setStatus('shown');
    };
    const heard = (frame: ScopeFrame) => {
      if (ended) return;
      pending = frame;
      if (painting !== 0) return;
      // Marked first: a frame of the screen may come at once, as it does in the tests.
      painting = -1;
      const asked = requestAnimationFrame(paint);
      if (painting === -1) painting = asked;
    };
    const failed = (failure: unknown, again: () => void) => {
      hub.give(holder);
      if (reasonOf(failure) === 'too_many') { setStatus('tooMany'); stopWaiting = hub.wait(again); return; }
      setStatus(failure instanceof EngineError && failure.code === -32601 ? 'unavailable' : 'failed');
    };
    const set = () => {
      if (ended || busy || id === null) return;
      const { key: asked, settings: given } = latest.current;
      if (asked === null || asked === sent) return;
      busy = true;
      void callEngine(bridge, 'scopes.set', { scope_id: id, settings: given })
        .then((answer) => { if (!ended && isAnswer(answer)) took(answer, asked); },
          () => { if (!ended) { sent = asked; setStatus('failed'); } })
        .finally(() => { busy = false; set(); });
    };
    const start = () => {
      stopWaiting();
      if (ended) return;
      if (!hub.take(holder)) { setStatus('tooMany'); stopWaiting = hub.wait(start); return; }
      setStatus('preparing');
      const { key: asked, settings: given } = latest.current;
      busy = true;
      void callEngine(bridge, 'scopes.subscribe', { view, settings: given })
        .then((answer) => {
          if (!isAnswer(answer)) throw new Error('The engine answered no scope.');
          if (ended) {
            // Hidden before the engine answered: the scope is let go at once.
            if (sessionNow.current === session) void callEngine(bridge, 'scopes.unsubscribe', { scope_id: answer.scope_id }).catch(() => {});
            return;
          }
          id = answer.scope_id;
          stopListening = hub.listen(session, id, heard);
          took(answer, asked);
        })
        .catch((failure) => { if (!ended) failed(failure, start); })
        .finally(() => { busy = false; set(); });
    };
    sync.current = set;
    start();
    return () => {
      ended = true;
      sync.current = null;
      stopWaiting();
      stopListening();
      if (painting > 0) cancelAnimationFrame(painting);
      if (id !== null && sessionNow.current === session) void callEngine(bridge, 'scopes.unsubscribe', { scope_id: id }).catch(() => {});
      hub.give(holder);
    };
  }, [ready, bridge, session, view]);

  // Other settings for the scope that is kept: `scopes.set`, the latest of them once the one before is answered.
  useEffect(() => { sync.current?.(); }, [key]);

  return { status: ready ? status : 'idle', graticule: ready ? graticule : null, canvas };
}
