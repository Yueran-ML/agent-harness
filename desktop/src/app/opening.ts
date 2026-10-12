/**
 * When the opening plays, and what a key does while it is on the screen,
 * worked out apart from the page so that every case can be tried.
 */
import type { Key } from './keys';

/**
 * The name under which it is kept. An older version kept the state of a
 * switch under another name, and that switch began as on: nobody chose it.
 */
export const openingStorageKey = 'columbina-opening';

/** What is kept: that the opening has been on the screen, or that it is wanted at every start. */
export type KeptOpening = 'seen' | 'always';

export interface Opening {
  /** Whether this start shows the opening. */
  show: boolean;
  /** Whether the user chose to see it at every start. */
  always: boolean;
}

/**
 * What a start does with the opening, from what the starts before it kept.
 * The first start, of which nothing is kept, shows it once.
 */
export function openingFor(kept: string | null): Opening {
  if (kept === 'always') return { show: true, always: true };
  return { show: kept !== 'seen', always: false };
}

/** What is kept once the opening has been on the screen. */
export function keptFor(always: boolean): KeptOpening {
  return always ? 'always' : 'seen';
}

export function storedOpening(): Opening {
  try {
    return openingFor(localStorage.getItem(openingStorageKey));
  } catch {
    // Without a place to keep it, every start is a first one.
    return openingFor(null);
  }
}

export function storeOpening(always: boolean): void {
  try {
    localStorage.setItem(openingStorageKey, keptFor(always));
  } catch {
    // See above.
  }
}

/** Where the opening stands when a key comes: before the film, during it, or on the page after it. */
export type OpeningPlace = 'waiting' | 'film' | 'end';

/** `leave` goes on into the application, and `play` starts a film that waits to be started. */
export type OpeningAction = 'leave' | 'play';

// The keys beside the ones that write a character, which every keyboard has and a hand lands on.
const beside = new Set([
  'Enter', 'Escape', 'Backspace', 'Delete', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End',
  'PageUp', 'PageDown',
]);

/**
 * What a key does while the opening is on the screen, or null for a key that
 * is left to whatever it is for.
 *
 * Any key leaves the opening. Left alone are the keys that are no answer to
 * it: Shift and its like, Tab, which moves among the controls, the keys for
 * the sound and the function keys, a key that is pressed with Ctrl or Alt,
 * and a key that is still down from before the opening came.
 *
 * Args:
 *   key: The key.
 *   place: Where the opening stands.
 *   onControl: Whether a control of the opening has the keyboard. Enter and
 *     the space bar press it then, as they do everywhere.
 */
export function openingKey(key: Key, place: OpeningPlace, onControl: boolean): OpeningAction | null {
  if (key.composing || key.repeat || key.ctrl || key.alt || key.meta) return null;
  // A key that writes a character is named by that character.
  if ([...key.key].length !== 1 && !beside.has(key.key)) return null;
  const presses = key.key === 'Enter' || key.key === ' ';
  if (onControl && presses) return null;
  return place === 'waiting' && key.key === 'Enter' ? 'play' : 'leave';
}
