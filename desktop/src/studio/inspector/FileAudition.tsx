import { useEffect, useMemo, useState, type KeyboardEvent, type PointerEvent } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine } from '../engine';
import { useFairlightText } from '../fairlight/words';

export type AuditionTarget = { kind: 'media'; media: string } | { kind: 'clip'; timeline: string; clip: string };
type Bridge = Pick<EngineBridge, 'call'>;
type Request = { kind: 'play'; position: number; channel: number | null; skim: boolean } | { kind: 'stop' };
type View = { position: number; playing: boolean; failed: boolean };
type Run = {
  alive: boolean; id: number | null; busy: boolean; continuous: boolean; serial: number;
  request: Request | null; position: number;
  poll: ReturnType<typeof setTimeout> | null;
  skimTimer: ReturnType<typeof setTimeout> | null;
  skim: { position: number; channel: number | null } | null;
};

// A previous inspector's delayed Play must settle before the next inspector's Play.
// Exact audition IDs make its later cleanup harmless to that newer playback.
const queues = new WeakMap<Bridge, Promise<unknown>>();
function ordered<T>(bridge: Bridge, action: () => Promise<T>): Promise<T> {
  const next = (queues.get(bridge) ?? Promise.resolve()).catch(() => {}).then(action);
  queues.set(bridge, next.catch(() => {}));
  return next;
}

/** A separate source-time transport; it never seeks either picture monitor or edits a clip. */
export function useFileAudition(bridge: Bridge, session: number, target: AuditionTarget, start: number, end: number, suspended = false) {
  const key = JSON.stringify([session, target, start, end, suspended]);
  const run = useMemo<Run>(() => ({ alive: true, id: null, busy: false, continuous: false, serial: 0,
    request: null, position: start, poll: null, skimTimer: null, skim: null }), [bridge, key]);
  const [display, setDisplay] = useState<{ run: Run; view: View }>(() => ({ run, view: { position: start, playing: false, failed: false } }));
  const view = display.run === run ? display.view : { position: start, playing: false, failed: false };
  const context = { expectedSession: session };
  const publish = (playing: boolean, failed = false) => {
    if (run.alive) setDisplay({ run, view: { position: run.position, playing, failed } });
  };
  const clearPoll = () => { if (run.poll !== null) clearTimeout(run.poll); run.poll = null; };
  const clearSkim = () => {
    if (run.skimTimer !== null) clearTimeout(run.skimTimer);
    run.skimTimer = null; run.skim = null;
  };
  const stopId = (id: number) => ordered(bridge, () => callEngine(bridge, 'audio.audition',
    { action: 'stop', audition: id }, context));
  const release = (id: number) => { void stopId(id).catch(() => {}); };

  const poll = () => {
    clearPoll();
    if (!run.alive || run.id === null) return;
    run.poll = setTimeout(async () => {
      run.poll = null;
      const serial = run.serial, id = run.id;
      try {
        const status = await ordered(bridge, () => callEngine(bridge, 'audio.audition', { action: 'state' }, context));
        if (!run.alive || serial !== run.serial) return;
        if (status.audition !== id) {
          run.id = null; run.continuous = false; publish(false); return;
        }
        if (status.position) run.position = status.position.ticks / status.position.scale;
        if (!status.playing) { run.id = null; run.continuous = false; }
        publish(status.playing && run.continuous, status.error !== null);
        if (status.playing) poll();
      } catch {
        if (run.alive && serial === run.serial) {
          if (id !== null) release(id);
          run.continuous = false; publish(false, true);
        }
      }
    }, 100);
  };

  const drain = async () => {
    if (run.busy) return;
    run.busy = true;
    try {
      while (run.alive && run.request) {
        const request = run.request;
        run.request = null;
        try {
          if (request.kind === 'stop') {
            if (run.id !== null) await stopId(run.id);
            run.id = null;
            if (!run.request) publish(false);
          } else {
            const status = await ordered(bridge, async () => run.alive ? callEngine(bridge, 'audio.audition', {
              action: 'play', target, channel: request.channel, skim: request.skim,
              at: { ticks: Math.round(request.position * 1_000_000), scale: 1_000_000 },
            }, context) : null);
            if (!status) continue;
            run.id = status.audition;
            if (!run.alive) { if (run.id !== null) release(run.id); break; }
            if (!run.request) {
              if (status.position) run.position = status.position.ticks / status.position.scale;
              run.continuous = status.playing && !request.skim;
              publish(run.continuous, status.error !== null);
              if (status.playing) poll();
            }
          }
        } catch {
          if (!run.request) { run.continuous = false; publish(false, true); }
        }
      }
    } finally { run.busy = false; }
  };
  const request = (next: Request) => {
    run.serial++; clearPoll(); run.request = next; void drain();
  };
  const stop = () => {
    clearSkim(); run.continuous = false; publish(false); request({ kind: 'stop' });
  };
  const play = () => {
    if (suspended || !(end > start)) return;
    clearSkim();
    if (run.position >= end || run.position < start) run.position = start;
    run.continuous = true; publish(true);
    // Stop then Play restores all channels, including after a solo audition.
    request({ kind: 'play', position: run.position, channel: null, skim: false });
  };
  const toggle = () => run.continuous ? stop() : play();
  const locate = (position: number, channel: number | null) => {
    clearSkim(); run.position = Math.max(start, Math.min(end, position));
    if (run.continuous) request({ kind: 'play', position: run.position, channel, skim: false });
    else request({ kind: 'stop' });
    publish(run.continuous);
  };
  const skim = (position: number, channel: number | null) => {
    if (suspended || run.continuous || !(end > start)) return;
    run.skim = { position: Math.max(start, Math.min(end, position)), channel };
    if (run.skimTimer !== null) return;
    run.skimTimer = setTimeout(() => {
      run.skimTimer = null;
      const point = run.skim; run.skim = null;
      if (!point || !run.alive || run.continuous) return;
      run.position = point.position; publish(false);
      request({ kind: 'play', ...point, skim: true });
    }, 60);
  };
  const leave = () => { clearSkim(); if (!run.continuous) request({ kind: 'stop' }); };
  const keyDown = (event: KeyboardEvent) => {
    if (event.code !== 'Space' && event.key !== ' ') return;
    const element = event.target as HTMLElement;
    event.stopPropagation();
    if (element.closest('input, select, textarea, [contenteditable="true"]')) return;
    event.preventDefault();
    if (suspended || event.repeat || event.altKey || event.ctrlKey || event.metaKey) return;
    const action = element.closest<HTMLElement>('[data-audition-control]')?.dataset.auditionControl;
    if (action === 'stop') stop();
    else if (action === 'play') play();
    else toggle();
  };

  useEffect(() => {
    run.alive = true;
    return () => {
      run.alive = false; run.request = null; clearPoll(); clearSkim();
      if (run.id !== null) release(run.id);
    };
  }, [run]);
  return { ...view, play, stop, toggle, locate, skim, leave, keyDown };
}

export type FileAudition = ReturnType<typeof useFileAudition>;

export function FileTransport({ audition, disabled = false }: { audition: FileAudition; disabled?: boolean }) {
  const t = useFairlightText();
  return <div className="file-audition">
    <div role="group" aria-label={t('Source audition')}>
      <button type="button" data-audition-control="play" disabled={disabled || audition.playing} onClick={audition.play}>{t('Play source')}</button>
      <button type="button" data-audition-control="stop" disabled={disabled} onClick={audition.stop}>{t('Stop source')}</button>
      <output aria-label={t('Source position')}>{audition.position.toFixed(2)} s</output>
    </div>
    <p className="caption">{t('Audition uses source channels and channel levels only.')}</p>
    {audition.failed && <p role="alert">{t('Could not play source audio. Try again.')}</p>}
  </div>;
}

export function pointerFraction(event: Pick<PointerEvent<HTMLElement>, 'clientX' | 'currentTarget'>): number | null {
  const rect = event.currentTarget.getBoundingClientRect();
  return rect.width > 0 ? Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) : null;
}

export function AuditionCursor({ position, start, end, reverse = false }: { position: number; start: number; end: number; reverse?: boolean }) {
  if (!(end > start)) return null;
  const fraction = Math.max(0, Math.min(1, (position - start) / (end - start)));
  return <span aria-hidden="true" className="file-audition-cursor" style={{ left: `${(reverse ? 1 - fraction : fraction) * 100}%` }} />;
}
