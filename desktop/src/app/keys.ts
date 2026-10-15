/**
 * What a key means, worked out apart from the page so that every key can be
 * tried in every state.
 */

/** A key as the page is told of it. */
export interface Key {
  key: string;
  shift: boolean;
  ctrl: boolean;
  alt: boolean;
  meta: boolean;
  /** Whether the key is held down and this is not its first report. */
  repeat: boolean;
  /** Whether the key belongs to an input method that is putting characters together. */
  composing: boolean;
}

/** An event of the keyboard, as the browser or React hands it over. */
interface KeyEvent {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  repeat: boolean;
  keyCode: number;
  isComposing?: boolean;
  nativeEvent?: { isComposing?: boolean };
}

export function keyOf(event: KeyEvent): Key {
  return {
    key: event.key,
    shift: event.shiftKey,
    ctrl: event.ctrlKey,
    alt: event.altKey,
    meta: event.metaKey,
    repeat: event.repeat,
    // Some input methods tell of the key that ends a composition as key 229 and nothing more.
    composing: (event.isComposing ?? event.nativeEvent?.isComposing ?? false) || event.keyCode === 229,
  };
}

const plain = (key: Key) => !key.shift && !key.ctrl && !key.alt && !key.meta;

/** Where the field stands when a key comes. */
export interface ComposerState {
  /** Whether the list of commands is open. */
  listOpen: boolean;
  running: boolean;
  /** Whether the turn is being stopped already. */
  cancelling: boolean;
  /** Whether Esc was pressed once a moment ago. */
  armed: boolean;
}

/**
 * What a key does in the field, or null for a key that the field leaves to the
 * browser: `next` and `previous` move through the list of commands, `take`
 * writes the chosen command into the field, `arm` keeps a first Esc in mind,
 * and `stop` stops the turn.
 */
export type ComposerAction = 'next' | 'previous' | 'take' | 'closeList' | 'send' | 'arm' | 'stop';

export function composerKey(key: Key, state: ComposerState): ComposerAction | null {
  // A key with another one held down is meant for something else: Shift+Tab goes back,
  // Shift+Enter is a new line, Ctrl+Enter answers a question.
  if (key.composing || !plain(key)) return null;
  if (state.listOpen) {
    if (key.key === 'ArrowDown') return 'next';
    if (key.key === 'ArrowUp') return 'previous';
    if (key.repeat) return null;
    if (key.key === 'Tab' || key.key === 'Enter') return 'take';
    if (key.key === 'Escape') return 'closeList';
    return null;
  }
  // One press is one deed, however long the key is held down.
  if (key.repeat) return null;
  if (key.key === 'Enter') return 'send';
  if (key.key === 'Escape' && state.running && !state.cancelling) return state.armed ? 'stop' : 'arm';
  return null;
}

/**
 * What a key does in the filter of the list of models, or null for a key that the filter leaves
 * to the browser: `next` and `previous` move through the models, `take` chooses the one that is
 * marked, and `close` closes the list.
 */
export type PickerAction = 'next' | 'previous' | 'take' | 'close';

export function pickerKey(key: Key): PickerAction | null {
  if (key.composing || !plain(key)) return null;
  if (key.key === 'ArrowDown') return 'next';
  if (key.key === 'ArrowUp') return 'previous';
  if (key.repeat) return null;
  if (key.key === 'Enter') return 'take';
  if (key.key === 'Escape') return 'close';
  return null;
}

/** Whether a key says yes, for this once, to a question of the agent. */
export function isYes(key: Key): boolean {
  return key.key === 'Enter' && (key.ctrl || key.meta) && !key.shift && !key.alt && !key.repeat && !key.composing;
}

/** For this long after a question appears, nothing answers it. */
export const SETTLE_MS = 500;

/**
 * Whether a question has been on the screen long enough to be answered.
 *
 * A key or a click that was on its way when the question appeared was not
 * meant for it: the second half of a double click, a chord that was pressed
 * for something else.
 */
export function settled(shownAt: number, now: number): boolean {
  return now - shownAt >= SETTLE_MS;
}
