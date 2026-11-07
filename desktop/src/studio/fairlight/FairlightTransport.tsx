import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowRightToLine, FastForward, Play, Repeat, Rewind, Square, Volume2, VolumeX } from 'lucide-react';
import type { EngineBridge } from '../../bridge';
import { keyOf } from '../../app/keys';
import { callEngine, type MonitorOutput } from '../engine';
import { editingAllowed, FASTEST, inStudio } from '../keys';
import type { MonitorControls } from '../useMonitor';
import { enteredFrame, nominalFps, outFrame, timecode } from '../timecode';
import type { Marks } from '../timeline/snap';
import { useFairlightText } from './words';

/** The speeds that Fast Forward and Fast Reverse go through, press by press (Reference Manual 20, p. 3744). */
export const SHUTTLE_SPEEDS = [8, 24, 60, 150, 360] as const;
/** How often a shuttle faster than the transport plays moves the playhead: 15 times a second. */
export const SHUTTLE_EVERY_MS = 1000 / 15;

/** The next speed of Fast Forward (`direction` 1) or Fast Reverse (-1) after `speed`. */
export function nextShuttle(speed: number, direction: 1 | -1): number {
  const now = Math.sign(speed) === direction ? Math.abs(speed) : 0;
  return direction * (SHUTTLE_SPEEDS.find(each => each > now) ?? SHUTTLE_SPEEDS[SHUTTLE_SPEEDS.length - 1]);
}

/** The lowest level of the monitor's slider, which stands for −∞ (C-M2: −∞ to 0 dB). */
export const MONITOR_FLOOR_DB = -60;

/**
 * The monitor controls at the right of the transport (Reference Manual 20, p. 3668): the speaker
 * mutes what is heard (red while it does), the slider sets its level and says it in dB while it
 * moves, and DIM takes 20 dB off it while it is on (the slider yellow). They change what every
 * output of the engine plays, never a render, the project or its undo.
 */
function MonitorLevel({ bridge, session, onFailure }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; onFailure: (error: unknown) => void;
}) {
  const t = useFairlightText();
  const [monitor, setMonitor] = useState<MonitorOutput | null>(null);
  const [moving, setMoving] = useState(false);
  const now = useRef({ onFailure, live: true });
  now.current.onFailure = onFailure;
  const send = useCallback((change: Partial<MonitorOutput>) => {
    void callEngine(bridge, 'audio.monitor', change, { expectedSession: session })
      .then(next => { if (now.current.live) setMonitor(next); })
      .catch(error => now.current.onFailure(error));
  }, [bridge, session]);
  useEffect(() => {
    now.current.live = true;
    send({});
    return () => { now.current.live = false; };
  }, [send]);
  const level = monitor?.level ?? null;
  const said = `${level === null ? '−∞' : level.toFixed(1)} dB`;
  return <div className="fairlight-monitor-controls">
    <button type="button" className="icon-btn" disabled={!monitor} aria-label={t('Monitor mute')} title={t('Monitor mute')}
      aria-pressed={monitor?.mute ?? false} data-muted={monitor?.mute || undefined} onClick={() => monitor && send({ mute: !monitor.mute })}>
      {monitor?.mute ? <VolumeX size={15} /> : <Volume2 size={15} />}</button>
    <input type="range" aria-label={t('Monitor level')} title={said} aria-valuetext={said} disabled={!monitor}
      min={MONITOR_FLOOR_DB} max={0} step={0.5} value={level ?? MONITOR_FLOOR_DB} data-dim={monitor?.dim || undefined}
      onPointerDown={() => setMoving(true)} onPointerUp={() => setMoving(false)} onBlur={() => setMoving(false)}
      onChange={event => {
        const value = Number(event.target.value);
        const next = value <= MONITOR_FLOOR_DB ? null : value;
        setMonitor(old => old && { ...old, level: next });
        send({ level: next });
      }} />
    {moving && <output className="mono" aria-label={t('Monitor level')}>{said}</output>}
    <button type="button" disabled={!monitor} aria-pressed={monitor?.dim ?? false} title="DIM"
      onClick={() => monitor && send({ dim: !monitor.dim })}>DIM</button>
  </div>;
}

/**
 * Fairlight's transport above its timeline (Reference Manual 20, pp. 3667–3668, 3744–3747): the
 * playhead's timecode, which Go To Timecode writes into (=, or a digit of the keypad); Range In and
 * Range Out, which mark the playhead and show the range and its duration; Fast Reverse, Stop,
 * Play, Fast Forward; Loop (Ctrl+/) and Play In to Out (Alt+/).
 *
 * The transport plays up to 8 times as fast. The faster speeds of Fast Forward and Fast Reverse
 * move the playhead through the timeline 15 times a second without sound, as the reference
 * assumes high speeds are silent; any other command or key ends them.
 */
export function FairlightTransport({ bridge, session, monitor, marks, onMark, onFailure }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; monitor: MonitorControls; marks: Marks;
  onMark: (edge: 'in' | 'out') => void; onFailure: (error: unknown) => void;
}) {
  const t = useFairlightText();
  const state = monitor.state?.monitor === 'timeline' ? monitor.state : null;
  const loaded = monitor.loaded && state !== null;
  const frame = monitor.frame ?? 0;
  const looping = state?.loop === true;
  const [shuttle, setShuttle] = useState<number | null>(null);
  const [entry, setEntry] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  const now = useRef({ monitor, frame, marks, looping, loaded, shuttle, onFailure });
  now.current = { monitor, frame, marks, looping, loaded, shuttle, onFailure };

  const ask = useCallback((method: 'transport.loop' | 'transport.play_range', params: Record<string, unknown>) => {
    void callEngine(bridge, method, { monitor: 'timeline', ...params }, { expectedSession: session })
      .catch(error => now.current.onFailure(error));
  }, [bridge, session]);
  const toggleLoop = useCallback(() => {
    if (now.current.loaded) ask('transport.loop', { on: !now.current.looping });
  }, [ask]);
  const range = marks.in !== null && marks.out !== null && outFrame(marks.out) > marks.in
    ? { from: marks.in, to: outFrame(marks.out) } : null;
  const playRange = useCallback(() => {
    const { marks: shown, loaded: ready } = now.current;
    if (!ready || shown.in === null || shown.out === null || outFrame(shown.out) <= shown.in) return;
    setShuttle(null);
    ask('transport.play_range', { from: shown.in, to: outFrame(shown.out) });
  }, [ask]);

  // A shuttle faster than the transport plays: the playhead moved by the clock, silently.
  useEffect(() => {
    if (shuttle === null || Math.abs(shuttle) <= FASTEST) return undefined;
    let at = now.current.frame;
    const timer = setInterval(() => {
      const { monitor: controls } = now.current;
      const last = controls.last ?? 0;
      at = Math.max(0, Math.min(last, at + shuttle * nominalFps(controls.rate) / 15));
      controls.seek(Math.round(at), false);
      if (at <= 0 || at >= last) setShuttle(null);
    }, SHUTTLE_EVERY_MS);
    return () => clearInterval(timer);
  }, [shuttle]);
  // Playing started elsewhere ends a shuttle of the clock.
  useEffect(() => { if (state?.playing) setShuttle(old => old !== null && Math.abs(old) > FASTEST ? null : old); }, [state?.playing]);

  const fast = (direction: 1 | -1) => {
    const playing = state?.playing === true ? state.speed : 0;
    const speed = nextShuttle(shuttle ?? playing, direction);
    if (Math.abs(speed) <= FASTEST) {
      setShuttle(null);
      monitor.run({ method: 'transport.play', params: { speed } });
      return;
    }
    if (state?.playing) monitor.run({ method: 'transport.pause' });
    setShuttle(speed);
  };
  const stop = () => { setShuttle(null); monitor.run({ method: 'transport.pause' }); };
  const play = () => { setShuttle(null); monitor.run({ method: 'transport.play', params: { speed: 1 } }); };
  const go = () => {
    const to = entry === null ? null : enteredFrame(entry, frame, monitor.rate);
    setEntry(null);
    if (to !== null && loaded) monitor.seek(Math.min(monitor.last ?? to, to), false);
  };
  useEffect(() => { if (entry !== null) field.current?.focus(); }, [entry !== null]);

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (!inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      // Any key ends a shuttle of the clock, one that the studio's keys took (K, Space) too.
      if (now.current.shuttle !== null) setShuttle(null);
      // A key held down toggles or opens once.
      if (event.defaultPrevented || event.repeat) return;
      const slash = event.code === 'Slash' || event.key === '/';
      const bare = !key.ctrl && !key.alt && !key.meta;
      if (slash && (key.ctrl || key.meta) && !key.alt && !key.shift) { event.preventDefault(); toggleLoop(); }
      else if (slash && key.alt && !key.ctrl && !key.meta && !key.shift) { event.preventDefault(); playRange(); }
      else if (bare && !key.shift && event.key === '=') { event.preventDefault(); setEntry(''); }
      else if (bare && event.code?.startsWith('Numpad') && /^[0-9]$/.test(event.key)) { event.preventDefault(); setEntry(event.key); }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [toggleLoop, playRange]);

  const rate = monitor.rate;
  const shownSpeed = shuttle ?? (state?.playing && state.speed !== 1 ? state.speed : null);
  return <section className="fairlight-transport" aria-label={t('Fairlight transport')}>
    <div className="fairlight-timecodes">
      {entry === null
        ? <button type="button" className="fairlight-big-timecode mono" aria-label={t('Playhead timecode')} title={`${t('Go To Timecode')} (=)`}
          disabled={!loaded} onClick={() => setEntry('')}>{timecode(frame, rate)}</button>
        : <input ref={field} className="fairlight-big-timecode mono" aria-label={t('Go To Timecode')} value={entry} placeholder={timecode(frame, rate)}
          onChange={event => setEntry(event.target.value)} onBlur={() => setEntry(null)}
          onKeyDown={event => {
            if (event.key === 'Enter') { event.preventDefault(); go(); }
            else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setEntry(null); }
          }} />}
      <button type="button" disabled={!loaded} onClick={() => onMark('in')}>{t('Range In')}</button>
      <output aria-label={t('Range In')} className="mono">{marks.in === null ? '--:--:--:--' : timecode(marks.in, rate)}</output>
      <button type="button" disabled={!loaded} onClick={() => onMark('out')}>{t('Range Out')}</button>
      <output aria-label={t('Range Out')} className="mono">{marks.out === null ? '--:--:--:--' : timecode(outFrame(marks.out), rate)}</output>
      <output aria-label={t('Range Duration')} className="mono" title={t('Range Duration')}>
        {marks.in !== null && marks.out !== null && marks.out > marks.in ? timecode(marks.out - marks.in, rate) : '--:--:--:--'}</output>
    </div>
    <div className="fairlight-transport-buttons">
      <button type="button" className="icon-btn" disabled={!loaded} aria-label={t('Fast Reverse')} title={`${t('Fast Reverse')} (Shift+J: −8×)`}
        aria-pressed={shownSpeed !== null && shownSpeed < 0} onClick={() => fast(-1)}><Rewind size={15} /></button>
      <button type="button" className="icon-btn" disabled={!loaded} aria-label={t('Stop')} title={`${t('Stop')} (K)`} onClick={stop}><Square size={14} /></button>
      <button type="button" className="icon-btn" disabled={!loaded} aria-label={t('Play')} title={`${t('Play')} (L)`}
        aria-pressed={state?.playing === true && state.speed === 1 && shuttle === null} onClick={play}><Play size={15} /></button>
      <button type="button" className="icon-btn" disabled={!loaded} aria-label={t('Fast Forward')} title={`${t('Fast Forward')} (Shift+L: 8×)`}
        aria-pressed={shownSpeed !== null && shownSpeed > 1} onClick={() => fast(1)}><FastForward size={15} /></button>
      <button type="button" className="icon-btn" disabled={!loaded} aria-label={t('Loop')} title={`${t('Loop')} (Ctrl+/)`} aria-pressed={looping}
        onClick={toggleLoop}><Repeat size={15} /></button>
      <button type="button" className="icon-btn" disabled={!loaded || range === null} aria-label={t('Play In to Out')} title={`${t('Play In to Out')} (Alt+/)`}
        aria-pressed={state?.range !== undefined} onClick={playRange}><ArrowRightToLine size={15} /></button>
      {shownSpeed !== null && <span className="fairlight-shuttle mono" data-test="shuttle">{`${shownSpeed < 0 ? '−' : ''}${Math.abs(shownSpeed)}×`}</span>}
    </div>
    <MonitorLevel bridge={bridge} session={session} onFailure={onFailure} />
  </section>;
}
