/**
 * The keys of the studio, as DaVinci Resolve has them by default:
 *
 * - Space plays forward, or stops.
 * - L plays forward, and faster at each press (1×, 2×, 4×, 8×); J plays backward the same way;
 *   K stops. With K held, L and J go a frame forward and back.
 * - ← and → go a frame back and forward; with Shift, a second.
 * - Home and End go to the first frame and the last.
 *
 * They are taken while the studio shows, where the keyboard is in the studio or nowhere in
 * particular (the page itself), and not while it writes into a field, while an input method puts
 * characters together, or while something lies over the page. A key that goes to a button or a
 * list of the conversation does what it does there.
 */
import { useEffect, useRef } from 'react';
import { keyOf } from '../app/keys';
import { overlayOpen } from '../app/overlay';

/** What a key asks of the transport: a method of the engine and its params. */
export type TransportCommand =
  | { method: 'transport.play'; params: { speed: number } }
  | { method: 'transport.pause'; params?: undefined }
  | { method: 'transport.step'; params: { frames: number } }
  | { method: 'transport.seek'; params: { frame: number; scrubbing: false } };

/** What the transport is doing as the keys know it, and what they need to know of the media. */
export interface KeyContext {
  playing: boolean;
  speed: number;
  /** The frames of a second of the media's timecode. */
  fps: number;
  /** The last frame of the media, where it is known. */
  last: number | null;
}

/** A key as it was pressed. */
export interface Pressed {
  key: string;
  shiftKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
  repeat?: boolean;
}

/** The fastest that the transport plays, forward and backward. */
export const FASTEST = 8;

/** The speed that L asks for: 1 where it does not play forward, else twice as fast, up to 8. */
export function faster(context: Pick<KeyContext, 'playing' | 'speed'>, forward: boolean): number {
  const sign = forward ? 1 : -1;
  if (!context.playing || Math.sign(context.speed) !== sign) return sign;
  return sign * Math.min(FASTEST, Math.abs(context.speed) * 2);
}

/**
 * What a key asks of the transport, or null where it asks nothing of it. `held` is whether K
 * is held down.
 */
export function commandOf(pressed: Pressed, context: KeyContext, held = false): TransportCommand | null {
  if (pressed.ctrlKey || pressed.metaKey || pressed.altKey) return null;
  const key = pressed.key.length === 1 ? pressed.key.toLowerCase() : pressed.key;
  // A key held down goes on stepping; one that plays or stops is taken once.
  if (pressed.repeat && key !== 'ArrowLeft' && key !== 'ArrowRight') return null;
  switch (key) {
    case ' ':
      return context.playing ? { method: 'transport.pause' } : { method: 'transport.play', params: { speed: 1 } };
    case 'k':
      return { method: 'transport.pause' };
    case 'l':
      return held ? { method: 'transport.step', params: { frames: 1 } }
        : { method: 'transport.play', params: { speed: faster(context, true) } };
    case 'j':
      return held ? { method: 'transport.step', params: { frames: -1 } }
        : { method: 'transport.play', params: { speed: faster(context, false) } };
    case 'ArrowLeft':
    case 'ArrowRight': {
      const by = pressed.shiftKey ? Math.max(1, context.fps) : 1;
      return { method: 'transport.step', params: { frames: key === 'ArrowLeft' ? -by : by } };
    }
    case 'Home':
      return { method: 'transport.seek', params: { frame: 0, scrubbing: false } };
    case 'End':
      return context.last === null ? null : { method: 'transport.seek', params: { frame: context.last, scrubbing: false } };
    default:
      return null;
  }
}

/** What the transport does once it has done a command, as far as the keys are concerned. */
export function afterCommand(context: Pick<KeyContext, 'playing' | 'speed'>, command: TransportCommand): { playing: boolean; speed: number } {
  if (command.method === 'transport.play') return { playing: true, speed: command.params.speed };
  if (command.method === 'transport.pause' || command.method === 'transport.step') return { playing: false, speed: context.speed };
  return { playing: context.playing, speed: context.speed };
}

/** How long a command waits for the transport to tell that it is done, before what it tells is taken as it is. */
export const AWAIT_MS = 500;

/**
 * What the transport does as the keys know it: what they asked for last, until the transport
 * tells that it does it; then what it tells. A state that comes meanwhile was on its way before
 * the command (a playhead that moved, at 30 a second) and is passed over, for a while.
 */
export class Intent {
  playing = false;
  speed = 1;
  private awaited: { playing: boolean; speed: number; at: number } | null = null;

  /** A command was sent. */
  sent(command: TransportCommand, now = Date.now()): void {
    const next = afterCommand(this, command);
    this.playing = next.playing;
    this.speed = next.speed;
    // A seek changes nothing of what the keys know.
    if (command.method !== 'transport.seek') this.awaited = { ...next, at: now };
  }

  /** The transport told how it stands. */
  told(state: { playing: boolean; speed: number }, now = Date.now()): void {
    const awaited = this.awaited;
    if (awaited !== null) {
      const done = state.playing === awaited.playing && (!awaited.playing || state.speed === awaited.speed);
      if (!done && now - awaited.at < AWAIT_MS) return;
      this.awaited = null;
    }
    this.playing = state.playing;
    this.speed = state.speed;
  }
}

/** Whether a key goes to the studio: the keyboard is in it, or on the page itself. */
export function inStudio(target: EventTarget | null): boolean {
  if (target === null || target === document.body || target === document.documentElement || target === document) return true;
  return target instanceof Element && target.closest('.studio') !== null;
}

/** Whether the keyboard writes into something: a field, a list to choose from, or text that can be edited. */
export function writing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.closest('[contenteditable=""], [contenteditable="true"]')) return true;
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
}

/**
 * Take the keys of the studio while `enabled` holds. `context` is read at each key: what the
 * transport does now and what the media is. `onCommand` is given what a key asks for.
 */
export function useStudioKeys(enabled: boolean, context: () => KeyContext, onCommand: (command: TransportCommand) => void): void {
  const now = useRef({ context, onCommand });
  now.current = { context, onCommand };
  useEffect(() => {
    if (!enabled) return undefined;
    // Whether K is held down, for a step with J or L.
    let held = false;
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || keyOf(event).composing || !inStudio(event.target) || writing(event.target)
        || overlayOpen() || document.querySelector('dialog[open]')) return;
      const command = commandOf(event, now.current.context(), held);
      if (event.key.toLowerCase() === 'k' && !event.ctrlKey && !event.metaKey && !event.altKey) held = true;
      if (command === null) return;
      event.preventDefault();
      now.current.onCommand(command);
    };
    const up = (event: KeyboardEvent) => { if (event.key.toLowerCase() === 'k') held = false; };
    const away = () => { held = false; };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', away);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('blur', away);
    };
  }, [enabled]);
}
