import { describe, expect, it } from 'vitest';
import { composerKey, isYes, keyOf, pickerKey, settled, SETTLE_MS, type ComposerState, type Key } from '../src/app/keys';

const key = (name: string, given: Partial<Key> = {}): Key => ({
  key: name, shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...given,
});
const state = (given: Partial<ComposerState> = {}): ComposerState => ({
  listOpen: false, running: false, cancelling: false, armed: false, asking: false, ...given,
});
const modifiers: Partial<Key>[] = [{ shift: true }, { ctrl: true }, { alt: true }, { meta: true }];

describe('a key as the page is told of it', () => {
  const event = (given: object) => ({
    key: 'Enter', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, repeat: false,
    isComposing: false, keyCode: 13, ...given,
  });

  it('is read with what was held down with it', () => {
    expect(keyOf(event({}))).toEqual(key('Enter'));
    expect(keyOf(event({ key: 'Tab', shiftKey: true }))).toEqual(key('Tab', { shift: true }));
    expect(keyOf(event({ ctrlKey: true, altKey: true, metaKey: true, repeat: true })))
      .toEqual(key('Enter', { ctrl: true, alt: true, meta: true, repeat: true }));
  });

  it('belongs to an input method while one puts characters together', () => {
    expect(keyOf(event({ isComposing: true })).composing).toBe(true);
  });

  it('belongs to an input method where it is told of as key 229 only', () => {
    // The key that ends a composition is told of this way by some input methods.
    expect(keyOf(event({ keyCode: 229 })).composing).toBe(true);
    expect(keyOf(event({ keyCode: 229, ctrlKey: true })).composing).toBe(true);
  });

  it('is read from an event of React by what the browser said', () => {
    const react = { ...event({ ctrlKey: true }), isComposing: undefined, nativeEvent: { isComposing: true } };
    expect(keyOf(react).composing).toBe(true);
    expect(keyOf({ ...react, nativeEvent: { isComposing: false } }).composing).toBe(false);
  });
});

describe('a key in the field', () => {
  it('does nothing of its own while an input method puts characters together', () => {
    for (const name of ['Enter', 'Escape', 'Tab', 'ArrowDown', 'ArrowUp']) {
      for (const at of [state(), state({ listOpen: true }), state({ running: true }), state({ running: true, armed: true })]) {
        expect(composerKey(key(name, { composing: true }), at)).toBeNull();
      }
    }
  });

  describe('with the list of commands open', () => {
    const open = state({ listOpen: true });

    it('moves through the list with the arrows', () => {
      expect(composerKey(key('ArrowDown'), open)).toBe('next');
      expect(composerKey(key('ArrowUp'), open)).toBe('previous');
      // Held down, an arrow goes on moving.
      expect(composerKey(key('ArrowDown', { repeat: true }), open)).toBe('next');
    });

    it('takes the chosen command with Enter or Tab, and sends nothing', () => {
      expect(composerKey(key('Enter'), open)).toBe('take');
      expect(composerKey(key('Tab'), open)).toBe('take');
    });

    it('closes the list with Esc and stops nothing, also while a turn runs', () => {
      expect(composerKey(key('Escape'), open)).toBe('closeList');
      expect(composerKey(key('Escape'), state({ listOpen: true, running: true }))).toBe('closeList');
      expect(composerKey(key('Escape'), state({ listOpen: true, running: true, armed: true }))).toBe('closeList');
    });

    it('leaves a key alone that is pressed with another one held down', () => {
      for (const held of modifiers) {
        // Shift+Tab goes back to what stands before the field. Ctrl+Enter is the answer to a question.
        expect(composerKey(key('Tab', held), open)).toBeNull();
        expect(composerKey(key('ArrowDown', held), open)).toBeNull();
        expect(composerKey(key('ArrowUp', held), open)).toBeNull();
        expect(composerKey(key('Escape', held), open)).toBeNull();
      }
      for (const held of [{ ctrl: true }, { alt: true }, { meta: true }]) {
        expect(composerKey(key('Enter', held), open)).toBeNull();
      }
      // Shift+Enter is a new line, with the list open as without it.
      expect(composerKey(key('Enter', { shift: true }), open)).toBeNull();
    });
  });

  describe('with the list closed', () => {
    it('sends with Enter and starts a new line with Shift+Enter', () => {
      expect(composerKey(key('Enter'), state())).toBe('send');
      expect(composerKey(key('Enter'), state({ running: true }))).toBe('send');
      expect(composerKey(key('Enter', { shift: true }), state())).toBeNull();
    });

    it('does not send with Enter and Alt held down', () => {
      expect(composerKey(key('Enter', { alt: true }), state())).toBeNull();
      expect(composerKey(key('Enter', { alt: true }), state({ running: true }))).toBeNull();
    });

    it('sends with Ctrl+Enter, and while a turn runs has what it sends read at once', () => {
      for (const held of [{ ctrl: true }, { meta: true }]) {
        expect(composerKey(key('Enter', held), state())).toBe('send');
        expect(composerKey(key('Enter', held), state({ running: true }))).toBe('sendNow');
        expect(composerKey(key('Enter', { ...held, shift: true }), state({ running: true }))).toBeNull();
        expect(composerKey(key('Enter', { ...held, repeat: true }), state({ running: true }))).toBeNull();
        expect(composerKey(key('Enter', { ...held, composing: true }), state({ running: true }))).toBeNull();
      }
    });

    it('leaves Ctrl+Enter to a question of the agent that waits', () => {
      expect(composerKey(key('Enter', { ctrl: true }), state({ running: true, asking: true }))).toBeNull();
      expect(composerKey(key('Enter', { ctrl: true }), state({ asking: true }))).toBeNull();
    });

    it('does not send again while Enter is held down', () => {
      expect(composerKey(key('Enter', { repeat: true }), state())).toBeNull();
    });

    it('leaves the arrows and Tab to the field', () => {
      for (const name of ['ArrowDown', 'ArrowUp', 'Tab', 'a', '/']) expect(composerKey(key(name), state())).toBeNull();
    });
  });

  describe('Esc while a turn runs', () => {
    const running = state({ running: true });

    it('is kept in mind the first time and stops the turn the second time', () => {
      expect(composerKey(key('Escape'), running)).toBe('arm');
      expect(composerKey(key('Escape'), { ...running, armed: true })).toBe('stop');
    });

    it('is one press however long it is held down', () => {
      expect(composerKey(key('Escape', { repeat: true }), running)).toBeNull();
      expect(composerKey(key('Escape', { repeat: true }), { ...running, armed: true })).toBeNull();
      // Nor does a key that is held down close the list and then go on to stop the turn.
      expect(composerKey(key('Escape', { repeat: true }), state({ listOpen: true, running: true }))).toBeNull();
    });

    it('stops nothing where nothing runs, or where the turn is being stopped already', () => {
      expect(composerKey(key('Escape'), state())).toBeNull();
      expect(composerKey(key('Escape'), state({ armed: true }))).toBeNull();
      expect(composerKey(key('Escape'), state({ running: true, cancelling: true }))).toBeNull();
      expect(composerKey(key('Escape'), state({ running: true, cancelling: true, armed: true }))).toBeNull();
    });

    it('is not meant where another key is held down with it', () => {
      for (const held of modifiers) {
        expect(composerKey(key('Escape', held), running)).toBeNull();
        expect(composerKey(key('Escape', held), { ...running, armed: true })).toBeNull();
      }
    });
  });
});

describe('a key in the filter of the models', () => {
  it('moves through the models with the arrows, also while it is held down', () => {
    expect(pickerKey(key('ArrowDown'))).toBe('next');
    expect(pickerKey(key('ArrowUp'))).toBe('previous');
    expect(pickerKey(key('ArrowDown', { repeat: true }))).toBe('next');
    expect(pickerKey(key('ArrowUp', { repeat: true }))).toBe('previous');
  });

  it('chooses with Enter and closes with Esc, once for each press', () => {
    expect(pickerKey(key('Enter'))).toBe('take');
    expect(pickerKey(key('Escape'))).toBe('close');
    expect(pickerKey(key('Enter', { repeat: true }))).toBeNull();
    expect(pickerKey(key('Escape', { repeat: true }))).toBeNull();
  });

  it('leaves every other key to the filter, and Tab to the browser', () => {
    for (const name of ['a', ' ', 'Tab', 'Home', 'End', 'ArrowLeft', 'ArrowRight', 'Backspace']) {
      expect(pickerKey(key(name)), name).toBeNull();
    }
  });

  it('does nothing of its own while an input method puts characters together, or with another key held down', () => {
    for (const name of ['Enter', 'Escape', 'ArrowDown', 'ArrowUp']) {
      expect(pickerKey(key(name, { composing: true })), name).toBeNull();
      for (const held of modifiers) expect(pickerKey(key(name, held)), `${name} ${JSON.stringify(held)}`).toBeNull();
    }
  });
});

describe('a yes by the keyboard', () => {
  it('is Ctrl+Enter, or the same with the key of a Mac', () => {
    expect(isYes(key('Enter', { ctrl: true }))).toBe(true);
    expect(isYes(key('Enter', { meta: true }))).toBe(true);
  });

  it('is never Enter alone', () => {
    expect(isYes(key('Enter'))).toBe(false);
  });

  it('is no other key, and not Enter with more held down', () => {
    expect(isYes(key(' ', { ctrl: true }))).toBe(false);
    expect(isYes(key('y', { ctrl: true }))).toBe(false);
    expect(isYes(key('Enter', { ctrl: true, shift: true }))).toBe(false);
    expect(isYes(key('Enter', { ctrl: true, alt: true }))).toBe(false);
    expect(isYes(key('Enter', { shift: true }))).toBe(false);
    expect(isYes(key('Enter', { alt: true }))).toBe(false);
  });

  it('is not a key that is held down', () => {
    // One yes is one press. A key held down from before the question is no yes to it.
    expect(isYes(key('Enter', { ctrl: true, repeat: true }))).toBe(false);
  });

  it('is not a key of an input method', () => {
    // Ctrl+Enter ends a composition in some input methods.
    expect(isYes(key('Enter', { ctrl: true, composing: true }))).toBe(false);
    expect(isYes(key('Enter', { meta: true, composing: true }))).toBe(false);
  });
});

describe('the first moment of a question', () => {
  it('lasts half a second, in which nothing answers it', () => {
    expect(SETTLE_MS).toBe(500);
    expect(settled(1000, 1000)).toBe(false);
    expect(settled(1000, 1499)).toBe(false);
    expect(settled(1000, 1500)).toBe(true);
    expect(settled(1000, 60_000)).toBe(true);
  });

  it('is not over where the clock went back', () => {
    expect(settled(1000, 900)).toBe(false);
  });
});
