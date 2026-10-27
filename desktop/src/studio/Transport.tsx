import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { Pause, Play, StepBack, StepForward, Volume2, VolumeOff, VolumeX } from 'lucide-react';
import { useWords } from '../app/locale';
import type { Rate, TransportState } from './engine';
import type { TransportCommand } from './keys';
import { timecode } from './timecode';

/** How often a seek is sent while the playhead is dragged: at most every 1/30 s. */
export const SCRUB_EVERY_MS = 1000 / 30;

/**
 * The bar that the playhead is dragged along. While it is dragged, a seek that scrubs is sent at
 * most every 1/30 s, the last place always; as it is let go, a seek to where it was let go that
 * does not scrub, so that the frame itself is shown.
 */
export function Scrubber({ frame, last, disabled, onSeek, onDrag }: {
  frame: number;
  last: number;
  disabled: boolean;
  onSeek: (frame: number, scrubbing: boolean) => void;
  /** Where the playhead is being dragged to, or null once it is let go. */
  onDrag: (frame: number | null) => void;
}) {
  const words = useWords();
  const bar = useRef<HTMLDivElement>(null);
  const [dragged, setDragged] = useState<number | null>(null);
  const sending = useRef<{ at: number; timer: ReturnType<typeof setTimeout> | null; waiting: number | null }>({ at: -Infinity, timer: null, waiting: null });
  useEffect(() => () => { if (sending.current.timer !== null) clearTimeout(sending.current.timer); }, []);

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
    setDragged(to);
    onDrag(to);
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
    const held = sending.current;
    if (held.timer !== null) clearTimeout(held.timer);
    held.timer = null;
    held.waiting = null;
    held.at = -Infinity;
    setDragged(null);
    onDrag(null);
    onSeek(to, false);
  };

  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    dragTo(frameAt(event.clientX));
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (dragged !== null) dragTo(frameAt(event.clientX));
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    if (dragged === null) return;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    letGo(frameAt(event.clientX));
  };
  const cancel = () => { if (dragged !== null) letGo(dragged); };

  const at = dragged ?? frame;
  const share = last > 0 ? Math.max(0, Math.min(1, at / last)) : 0;
  return <div
    ref={bar}
    className="scrubber"
    role="slider"
    tabIndex={-1}
    aria-label={words.label.studio.position}
    aria-valuemin={0}
    aria-valuemax={last}
    aria-valuenow={at}
    aria-disabled={disabled || undefined}
    data-dragging={dragged !== null ? '' : undefined}
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
function Sound({ sound }: { sound: TransportState['sound'] }) {
  const words = useWords();
  const Icon = sound === 'on' ? Volume2 : sound === 'off' ? VolumeX : VolumeOff;
  return <span
    className="transport-sound"
    role="img"
    data-tone={sound === 'unavailable' ? 'warn' : sound === 'on' ? 'ok' : undefined}
    aria-label={words.label.studio.sound[sound]}
    title={words.hint.studio.sound[sound]}
    data-test="sound"
    data-sound={sound}
  >
    <Icon size={15} aria-hidden="true" />
  </span>;
}

/**
 * The controls of the source monitor: back a frame, play or pause, forward a frame, the timecode
 * of the playhead, the bar that it is dragged along, whether proxies are used and what the picture
 * is read from, and how the sound stands. What is shown follows what the transport tells.
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
  const [dragged, setDragged] = useState<number | null>(null);
  const loaded = state !== null && state.media !== null;
  const frame = dragged ?? state?.frame ?? 0;
  const playing = loaded && state.playing;
  const speed = state?.speed ?? 1;
  return <div className="transport" data-test="transport">
    <div className="transport-row">
      <span
        className="timecode mono"
        data-exact={!loaded || state.exact ? undefined : 'no'}
        title={loaded && !state.exact ? words.hint.studio.inexact : undefined}
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
          title={`${playing ? words.label.studio.pause : words.label.studio.play} (Space)`}
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
          title={words.hint.studio.useProxies}
          onClick={() => onProxies(!(state?.proxies ?? true))}
          data-test="use-proxies"
        >
          {words.label.studio.useProxies}
        </button>
        {loaded && <Sound sound={state.sound} />}
        <span className="timecode timecode-last mono" aria-hidden="true">{last === null ? '' : timecode(last, rate)}</span>
      </div>
    </div>
    <Scrubber frame={state?.frame ?? 0} last={last ?? 0} disabled={!loaded || last === null} onSeek={onSeek} onDrag={setDragged} />
  </div>;
}
