import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Key } from '../src/app/keys';
import {
  keptFor, openingFor, openingKey, openingStorageKey, storedOpening, storeOpening,
} from '../src/app/opening';

const key = (name: string, more: Partial<Key> = {}): Key => ({
  key: name, shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...more,
});

/** A place to keep things, as a window has one. */
function storage(kept: Record<string, string> = {}) {
  return {
    getItem: (name: string) => (Object.hasOwn(kept, name) ? kept[name] : null),
    setItem: (name: string, value: string) => { kept[name] = value; },
    kept,
  };
}
const broken = {
  getItem: () => { throw new Error('no storage'); },
  setItem: () => { throw new Error('no storage'); },
};

afterEach(() => vi.unstubAllGlobals());

describe('when the opening plays', () => {
  it('plays at the first start, of which nothing is kept', () => {
    expect(openingFor(null)).toEqual({ show: true, always: false });
  });

  it('does not play again once it has been on the screen', () => {
    expect(openingFor('seen')).toEqual({ show: false, always: false });
  });

  it('plays at every start where that was chosen', () => {
    expect(openingFor('always')).toEqual({ show: true, always: true });
  });

  it('takes what it does not know for nothing', () => {
    // `true` and `false` are what an older version kept, under another name.
    for (const kept of ['true', 'false', '', 'SEEN', 'Always', 'toString']) {
      expect(openingFor(kept), kept).toEqual({ show: true, always: false });
    }
  });

  it('keeps that it was seen, or that it is wanted every time', () => {
    expect(keptFor(false)).toBe('seen');
    expect(keptFor(true)).toBe('always');
    expect(openingFor(keptFor(false))).toEqual({ show: false, always: false });
    expect(openingFor(keptFor(true))).toEqual({ show: true, always: true });
  });

  it('is kept under a name that the older version did not use', () => {
    // What stood under the old name was the state of a switch that began as on. Nobody chose it.
    expect(openingStorageKey).toBe('columbina-opening');
    expect(openingStorageKey).not.toBe('columbina-play-intro');
  });
});

describe('what is kept of the opening', () => {
  it('is nothing at the first start, and that it was seen after it', () => {
    const kept = storage({ 'columbina-play-intro': 'true' });
    vi.stubGlobal('localStorage', kept);
    expect(storedOpening()).toEqual({ show: true, always: false });
    storeOpening(false);
    expect(kept.kept[openingStorageKey]).toBe('seen');
    expect(storedOpening()).toEqual({ show: false, always: false });
    // What the older version kept is left as it is.
    expect(kept.kept['columbina-play-intro']).toBe('true');
  });

  it('is the choice to see it every time, until that is taken back', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    storeOpening(true);
    expect(storedOpening()).toEqual({ show: true, always: true });
    storeOpening(false);
    expect(storedOpening()).toEqual({ show: false, always: false });
  });

  it('does without a place to keep it: then every start is a first one', () => {
    vi.stubGlobal('localStorage', broken);
    expect(storedOpening()).toEqual({ show: true, always: false });
    expect(() => storeOpening(true)).not.toThrow();
  });
});

describe('a key while the opening is on the screen', () => {
  const anyKey = ['a', 'Z', '1', ' ', '/', '中', 'Enter', 'Escape', 'Backspace', 'Delete', 'ArrowDown', 'ArrowLeft', 'Home', 'PageDown'];

  it.each(anyKey)('%j leaves the film', (name) => {
    expect(openingKey(key(name), 'film', false)).toBe('leave');
  });

  it.each(anyKey)('%j leaves the page that follows the film', (name) => {
    expect(openingKey(key(name), 'end', false)).toBe('leave');
  });

  it('leaves with a capital letter, which is typed with Shift', () => {
    expect(openingKey(key('A', { shift: true }), 'film', false)).toBe('leave');
  });

  it('leaves with a character that takes two units to write', () => {
    // A face from the panel of emoji is one character, and its name is that character.
    const face = String.fromCodePoint(0x1f600);
    expect(face.length).toBe(2);
    expect(openingKey(key(face), 'film', false)).toBe('leave');
  });

  it.each([
    'Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'NumLock', 'Tab', 'F5', 'F11', 'F12', 'AudioVolumeUp', 'AudioVolumeMute',
    'MediaPlayPause', 'PrintScreen', 'ContextMenu', 'Dead', 'Process', 'Unidentified',
  ])('%s is left to whatever it is for', (name) => {
    for (const where of ['waiting', 'film', 'end'] as const) expect(openingKey(key(name), where, false), where).toBeNull();
  });

  it('leaves a key alone that is pressed with Ctrl, Alt or the key of the system', () => {
    for (const more of [{ ctrl: true }, { alt: true }, { meta: true }, { ctrl: true, shift: true }]) {
      expect(openingKey(key('r', more), 'film', false)).toBeNull();
      expect(openingKey(key('Enter', more), 'end', false)).toBeNull();
    }
  });

  it('leaves a key alone that is held down from before, and one of an input method', () => {
    // Enter that started the application is still down when the film comes.
    expect(openingKey(key('Enter', { repeat: true }), 'film', false)).toBeNull();
    expect(openingKey(key('a', { composing: true }), 'film', false)).toBeNull();
  });

  it('lets Enter and the space bar press the control that has the keyboard', () => {
    for (const where of ['waiting', 'film', 'end'] as const) {
      expect(openingKey(key('Enter'), where, true), where).toBeNull();
      expect(openingKey(key(' '), where, true), where).toBeNull();
      // Every other key leaves, wherever the keyboard is.
      expect(openingKey(key('Escape'), where, true), where).toBe('leave');
      expect(openingKey(key('x'), where, true), where).toBe('leave');
    }
  });

  it('plays the film with Enter where it waits to be played, and leaves with every other key', () => {
    expect(openingKey(key('Enter'), 'waiting', false)).toBe('play');
    expect(openingKey(key(' '), 'waiting', false)).toBe('leave');
    expect(openingKey(key('Escape'), 'waiting', false)).toBe('leave');
  });
});
