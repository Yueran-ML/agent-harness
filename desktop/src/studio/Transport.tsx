import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { Layers, Pause, Play, StepBack, StepForward, Volume2, VolumeOff, VolumeX } from 'lucide-react';
import { useWords } from '../app/locale';
import type { Rate, TransportState } from './engine';
import type { TransportCommand } from './keys';
import { timecode } from './timecode';

/** How often a seek is sent while the playhead is dragged: at most every 1/30 s. */
export const SCRUB_EVERY_MS = 1000 / 30;

/** How long the frame that the playhead was let go at is shown, at most, before the transport tells it. */
export const LET_GO_HOLD_MS = 1000;

/**
 * The bar that the playhead is dragged along. While it is dragged, a seek that scrubs is sent at
 * most every 1/30 s, the last place always; as it is let go, a seek to where it was let go that
 * does not scrub, so that the frame itself is shown. A drag that the bar can no longer take (the
 * media was unloaded, the engine ended) sends nothing more.
 */
export function Scrubber({ frame, last, disabled, valueText, onSeek, onDrag }: {
  /** The frame that the bar shows: where the playhead is, or is being dragged to. */
  frame: number;
  last: number;
  disabled: boolean;
  /** The frame as it is said: its timecode. */
  valueText: string;
  onSeek: (frame: number, scrubbing: boolean) => void;
  /** Where the playhead is being dragged to, or was let go at; null where the drag was given up. */
  onDrag: (frame: number | null, dragging: boolean) => void;
}) {
  const words = useWords();
  const bar = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const sending = useRef<{ at: number; timer: ReturnType<typeof setTimeout> | null; waiting: number | null }>({ at: -Infinity, timer: null, waiting: null });
  const stopSending = () => {
    const held = sending.current;
    if (held.timer !== null) clearTimeout(held.timer);
    held.timer = null;
    held.waiting = null;
    held.at = -Infinity;
  };
  useEffect(() => stopSending, []);
  // The bar can no longer be dragged: the drag ends, and nothing more is sent.
  useEffect(() => {
    if (!disabled || !dragging) return;
    stopSending();
    setDragging(false);
    onDrag(null, false);
  }, [disabled, dragging]);

  const frameAt = (x: number) => {
    const box = bar.current!.getBoundingClientRect();
    const along = box.width > 0 ? (x - box.left) / box.width : 0;
    return Math.round(Math.max(0, Math.min(1, along)) * last);
  };
  const send = (to: number) => {
    sending.current.at = Date.now();
    onSeek(to, true);
  };
  const dragTo = (to: number) => {
    onDrag(to, true);
    const held = sending.current;
    const since = Date.now() - held.at;
    if (since >= SCRUB_EVERY_MS && held.timer === null) {
      send(to);
      return;
    }
    held.waiting = to;
    if (held.timer !== null) return;
    held.timer = setTimeout(() => {
      held.timer = null;
      const waiting = held.waiting;
      held.waiting = null;
      if (waiting !== null) send(waiting);
    }, Math.max(0, SCRUB_EVERY_MS - since));
  };
  const letGo = (to: number) => {
    stopSending();
    setDragging(false);
    onDrag(to, false);
    onSeek(to, false);
  };

  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDragging(true);
    dragTo(frameAt(event.clientX));
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (dragging && !disabled) dragTo(frameAt(event.clientX));
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    if (disabled) return;
    letGo(frameAt(event.clientX));
  };
  const cancel = () => { if (dragging && !disabled) letGo(frame); };

  const share = last > 0 ? Math.max(0, Math.min(1, frame / last)) : 0;
  return <div
    ref={bar}
    className="scrubber"
    role="slider"
    tabIndex={disabled ? -1 : 0}
    aria-label={words.label.studio.position}
    aria-valuemin={0}
    aria-valuemax={last}
    aria-valuenow={frame}
    aria-valuetext={valueText}
    aria-disabled={disabled || undefined}
    data-dragging={dragging ? '' : undefined}
    onPointerDown={down}
    onPointerMove={move}
    onPointerUp={up}
    onPointerCancel={cancel}
    data-test="scrubber"
  >
    <span className="scrubber-track" aria-hidden="true"><span className="scrubber-done" style={{ width: `${share * 100}%` }} /></span>
    <span className="scrubber-head" aria-hidden="true" style={{ left: `${share * 100}%` }} />
  </div>;
}

/** The sign of how the sound stands, which says it in full when it is pointed at. */
function Sound({ sound, playing, speed }: { sound: TransportState['sound']; playing: boolean; speed: number }) {
  const words = useWords();
  const Icon = sound === 'on' ? Volume2 : sound === 'off' ? VolumeX : VolumeOff;
  // Sound is off while paused, and while it plays at another speed than 1: which, is said.
  const why = sound === 'off' && playing && speed !== 1 ? 'offSpeed' : sound;
  return <span
    className="transport-sound"
    role="img"
    data-tone={sound === 'unavailable' ? 'warn' : sound === 'on' ? 'ok' : undefined}
    aria-label={words.label.studio.sound[sound]}
    title={words.hint.studio.sound[why]}
    data-test="sound"
    data-sound={sound}
  >
    <Icon size={15} aria-hidden="true" />
  </span>;
}

/**
 * The controls of the source monitor: back a frame, play or pause, forward a frame, the timecode
 * of the playhead, the bar that it is dragged along, whether proxies are used and what the picture
 * is read from, and how the sound stands. What is shown follows what the transport tells; the
 * frame the playhead is dragged to is shown at once, and, once it is let go, until the transport
 * tells it (or a second has passed), so that the playhead does not jump back meanwhile.
 */
export function Transport({ state, rate, last, onCommand, onSeek, onProxies }: {
  state: TransportState | null;
  rate: Rate;
  /** The last frame of the media loaded, where it is known. */
  last: number | null;
  onCommand: (command: TransportCommand) => void;
  onSeek: (frame: number, scrubbing: boolean) => void;
  onProxies: (on: boolean) => void;
}) {
  const words = useWords();
  const [held, setHeld] = useState<{ frame: number; dragging: boolean; media: string | null } | null>(null);
  const loaded = state !== null && state.media !== null;
  // The frame let go at is shown until the transport is there, or has gone elsewhere.
  useEffect(() => {
    if (held === null || held.dragging) return;
    if (!loaded || state.media !== held.media || state.frame === held.frame || state.playing) setHeld(null);
  }, [held, state, loaded]);
  useEffect(() => {
    if (held === null || held.dragging) return undefined;
    const timer = setTimeout(() => setHeld(null), LET_GO_HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  const frame = held?.frame ?? state?.frame ?? 0;
  const playing = loaded && state.playing;
  const speed = state?.speed ?? 1;
  const space = words.label.studio.space;
  return <div className="transport" data-test="transport">
    <div className="transport-row">
      <span
        className="timecode mono"
        data-exact={!loaded || state.exact || held !== null ? undefined : 'no'}
        title={loaded && !state.exact && held === null ? words.hint.studio.inexact : undefined}
        data-test="timecode"
      >
        {timecode(frame, rate)}
      </span>
      <div className="transport-buttons">
        <button type="button" className="icon-btn" disabled={!loaded} aria-label={words.label.studio.back} title={`${words.label.studio.back} (←)`}
          onClick={() => onCommand({ method: 'transport.step', params: { frames: -1 } })} data-test="step-back">
          <StepBack size={16} />
        </button>
        <button type="button" className="icon-btn transport-play" disabled={!loaded}
          aria-label={playing ? words.label.studio.pause : words.label.studio.play}
          title={`${playing ? words.label.studio.pause : words.label.studio.play} (${space})`}
          onClick={() => onCommand(playing ? { method: 'transport.pause' } : { method: 'transport.play', params: { speed: 1 } })}
          data-test="play">
          {playing ? <Pause size={18} /> : <Play size={18} />}
        </button>
        <button type="button" className="icon-btn" disabled={!loaded} aria-label={words.label.studio.forward} title={`${words.label.studio.forward} (→)`}
          onClick={() => onCommand({ method: 'transport.step', params: { frames: 1 } })} data-test="step-forward">
          <StepForward size={16} />
        </button>
        {playing && speed !== 1 && <span className="transport-speed mono" data-test="speed">{`${speed < 0 ? '−' : ''}${Math.abs(speed)}×`}</span>}
      </div>
      <div className="transport-ends">
        {loaded && state.source && <span className="source-badge" data-source={state.source} title={words.hint.studio.source[state.source]} data-test="source">
          {words.label.studio.source[state.source]}
        </span>}
        <button
          type="button"
          className="btn btn-quiet transport-proxies"
          aria-pressed={state?.proxies ?? true}
          aria-label={words.label.studio.useProxies}
          title={words.hint.studio.useProxies}
          onClick={() => onProxies(!(state?.proxies ?? true))}
          data-test="use-proxies"
        >
          {/* Where the bar is narrow the button is its sign alone, and its name stays with it for a reader. */}
          <Layers size={15} className="transport-proxies-sign" aria-hidden="true" />
          <span className="transport-proxies-words">{words.label.studio.useProxies}</span>
        </button>
        {loaded && <Sound sound={state.sound} playing={state.playing} speed={state.speed} />}
        <span className="timecode timecode-last mono" aria-hidden="true">{last === null ? '' : timecode(last, rate)}</span>
      </div>
    </div>
    <Scrubber
      frame={frame}
      last={last ?? 0}
      disabled={!loaded || last === null}
      valueText={timecode(frame, rate)}
      onSeek={onSeek}
      onDrag={(to, dragging) => setHeld(to === null ? null : { frame: to, dragging, media: state?.media ?? null })}
    />
  </div>;
}
