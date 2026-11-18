/**
 * The runs of the text of a title that are drawn otherwise than its style says (`titleSpans.ts`): counted in Unicode scalar
 * values as the engine counts them, kept as the engine takes them, styled over a selection, read back over one, and moved
 * with the characters that they are on where the text is edited.
 */
import { describe, expect, it } from 'vitest';
import type { TitleSpan, TitleStyle } from '../src/studio/engine';
import {
  clipped, lengthOf, LONGEST_TEXT, lookOver, MOST_SPANS, runsOf, scalarAt, shifted, styled, tidied, withSpans,
} from '../src/studio/inspector/titleSpans';

const run = (from: number, to: number, look: Partial<TitleSpan> = {}): TitleSpan => ({ from, to, ...look });
const RED = '#ff0000ff';
const GREEN = '#00ff00ff';
const PLAIN: TitleStyle = { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' };

describe('the characters of a title', () => {
  it('are counted as the engine counts them, in Unicode scalar values, and a selection in units of UTF-16 is turned into them', () => {
    const text = 'Hi😀!';
    expect([text.length, lengthOf(text), lengthOf(''), lengthOf('漢字')]).toEqual([5, 4, 0, 2]);
    expect([0, 1, 2, 4, 5, 99].map(unit => scalarAt(text, unit))).toEqual([0, 1, 2, 3, 4, 4]);
    // A place within the two units of the face is after it.
    expect(scalarAt(text, 3)).toBe(3);
  });

  it('are cut to the longest text that the engine takes, and never within a character', () => {
    expect(LONGEST_TEXT).toBe(1000);
    expect(clipped('short')).toBe('short');
    expect(lengthOf(clipped('a'.repeat(1200)))).toBe(1000);
    const faces = clipped(`${'a'.repeat(999)}😀😀`);
    expect([lengthOf(faces), faces.endsWith('a😀')]).toEqual([1000, true]);
  });
});

describe('the runs of a title, as the engine takes them', () => {
  it('are in order, within the text, and none says nothing or is empty', () => {
    expect(tidied([run(5, 9, { bold: true }), run(0, 3, { color: RED })], 20)).toEqual([run(0, 3, { color: RED }), run(5, 9, { bold: true })]);
    // Bold and italic off, a share of 1 and no family say nothing; a run past the end ends with the text.
    expect(tidied([run(2, 2, { bold: true }), run(3, 6), run(4, 30, { italic: true }), run(8, 9, { bold: false, italic: false, scale: 1, font: '' })], 10))
      .toEqual([run(4, 10, { italic: true })]);
    expect(tidied([run(0, 1, { scale: 50 }), run(1, 2, { scale: 0.01 })], 5)).toEqual([run(0, 1, { scale: 10 }), run(1, 2, { scale: 0.1 })]);
  });

  it('are one where two side by side say the same, and the later of two that overlap begins where the earlier ends', () => {
    expect(tidied([run(0, 2, { bold: true }), run(2, 5, { bold: true }), run(5, 6, { bold: true, italic: true })], 10))
      .toEqual([run(0, 5, { bold: true }), run(5, 6, { bold: true, italic: true })]);
    expect(tidied([run(0, 4, { bold: true }), run(2, 6, { italic: true }), run(3, 4, { color: RED })], 10))
      .toEqual([run(0, 4, { bold: true }), run(4, 6, { italic: true })]);
  });
});

describe('styling a selection of the text', () => {
  it('puts a run over characters that had none', () => {
    expect(styled([], 2, 5, { bold: true }, 10)).toEqual([run(2, 5, { bold: true })]);
    expect(styled([], 0, 3, { font: 'Georgia' }, 10)).toEqual([run(0, 3, { font: 'Georgia' })]);
  });

  it('cuts a run where the selection ends within it, and keeps what is outside as it was', () => {
    expect(styled([run(0, 10, { color: RED })], 3, 6, { bold: true }, 10))
      .toEqual([run(0, 3, { color: RED }), run(3, 6, { color: RED, bold: true }), run(6, 10, { color: RED })]);
  });

  it('changes every run and gap that the selection covers, each keeping what the change does not say', () => {
    expect(styled([run(0, 3, { italic: true }), run(5, 8, { color: GREEN })], 1, 7, { scale: 2 }, 10)).toEqual([
      run(0, 1, { italic: true }), run(1, 3, { italic: true, scale: 2 }), run(3, 5, { scale: 2 }), run(5, 7, { color: GREEN, scale: 2 }), run(7, 8, { color: GREEN }),
    ]);
  });

  it('takes away what a change gives as null, and the run with it where it says nothing more', () => {
    const both = [run(0, 4, { bold: true, color: RED })];
    expect(styled(both, 0, 4, { bold: null }, 4)).toEqual([run(0, 4, { color: RED })]);
    expect(styled([run(0, 4, { color: RED })], 0, 4, { color: null }, 4)).toEqual([]);
    // Clear in the middle of a run leaves its two ends.
    const clear = { color: null, scale: null, bold: null, italic: null, font: null };
    expect(styled([run(0, 6, { bold: true })], 2, 4, clear, 6)).toEqual([run(0, 2, { bold: true }), run(4, 6, { bold: true })]);
    // A share of 1 is the size of the title: it is no share.
    expect(styled([run(0, 4, { scale: 2 })], 0, 4, { scale: 1 }, 4)).toEqual([]);
  });

  it('makes one run of runs that come to say the same', () => {
    expect(styled([run(0, 2, { bold: true }), run(4, 6, { bold: true })], 2, 4, { bold: true }, 6)).toEqual([run(0, 6, { bold: true })]);
  });

  it('keeps a selection within the text, and changes nothing for a selection of nothing', () => {
    expect(styled([], 8, 20, { italic: true }, 10)).toEqual([run(8, 10, { italic: true })]);
    const runs = [run(0, 2, { bold: true })];
    expect(styled(runs, 3, 3, { bold: true }, 10)).toEqual(runs);
    expect(styled([], 12, 20, { bold: true }, 10)).toEqual([]);
  });

  it('is not made where the title would have more runs than the engine takes', () => {
    expect(MOST_SPANS).toBe(256);
    const many = Array.from({ length: 256 }, (_, at) => run(2 * at, 2 * at + 1, { bold: true }));
    expect(styled(many, 513, 514, { italic: true }, 600)).toBeNull();
    // Changing a run there is, or joining two, leaves no more than there were.
    expect(styled(many, 0, 1, { italic: true }, 600)).toHaveLength(256);
    expect(styled(many, 1, 2, { bold: true }, 600)).toHaveLength(255);
  });
});

describe('what a selection of the text says', () => {
  const runs = [run(0, 4, { bold: true, color: RED }), run(4, 8, { bold: true, scale: 2 })];

  it('is what every character of it says, and null for what they say apart', () => {
    expect(lookOver(runs, 0, 4)).toEqual({ bold: true, italic: false, color: RED, scale: undefined, font: undefined });
    expect(lookOver(runs, 1, 6)).toEqual({ bold: true, italic: false, color: null, scale: null, font: undefined });
    // Bold where every character is: two of these four are not.
    expect(lookOver(runs, 6, 10)).toEqual({ bold: false, italic: false, color: undefined, scale: null, font: undefined });
  });

  it('is nothing for a selection of nothing', () => {
    expect(lookOver(runs, 3, 3)).toEqual({ bold: false, italic: false, color: undefined, scale: undefined, font: undefined });
  });
});

describe('the runs of a text that is edited', () => {
  it('move with the characters after them where something is put in before them', () => {
    expect(shifted([run(6, 11, { bold: true })], 'Hello world', 'Oh, Hello world')).toEqual([run(10, 15, { bold: true })]);
  });

  it('grow where something is put in within them, and do not where it is put in at their ends', () => {
    expect(shifted([run(0, 5, { bold: true })], 'Hello', 'Hel-lo')).toEqual([run(0, 6, { bold: true })]);
    expect(shifted([run(0, 5, { bold: true }), run(6, 11, { italic: true })], 'Hello world', 'Hello, world'))
      .toEqual([run(0, 5, { bold: true }), run(7, 12, { italic: true })]);
  });

  it('shrink where something within them is taken away, and keep what is left where the edit cuts into them', () => {
    expect(shifted([run(6, 11, { italic: true })], 'Hello world', 'Hello wd')).toEqual([run(6, 8, { italic: true })]);
    // "lo w" taken away: the end of the first word goes, and its beginning stays.
    expect(shifted([run(0, 5, { bold: true })], 'Hello world', 'Helorld')).toEqual([run(0, 3, { bold: true })]);
  });

  it('are gone where the edit took away all of their characters, and those after them move back', () => {
    expect(shifted([run(6, 9, { bold: true }), run(10, 15, { italic: true })], 'Hello big world', 'Hello world')).toEqual([run(6, 11, { italic: true })]);
    // Characters typed over the characters of a run are not of it.
    expect(shifted([run(0, 1, { color: RED })], 'Hello', 'Jello')).toEqual([]);
    expect(shifted([run(0, 5, { bold: true }), run(6, 11, { italic: true })], 'Hello world', 'world')).toEqual([run(0, 5, { italic: true })]);
  });

  it('are counted in characters, a face of two units of UTF-16 being one', () => {
    expect(shifted([run(1, 2, { bold: true })], 'a😀b', 'xa😀b')).toEqual([run(2, 3, { bold: true })]);
  });

  it('end with the text, and are as they were where the text is the same', () => {
    expect(shifted([run(0, 4, { bold: true })], 'Hello', 'Hello'.slice(0, 2))).toEqual([run(0, 2, { bold: true })]);
    expect(shifted([run(2, 4, { bold: true }), run(0, 1, { color: RED })], 'Hello', 'Hello')).toEqual([run(0, 1, { color: RED }), run(2, 4, { bold: true })]);
  });
});

describe('the parts of a text that a title is drawn in', () => {
  it('are its runs, each with what it says, and the characters between them, which say nothing', () => {
    expect(runsOf('Hello world', [run(6, 11, { bold: true }), run(0, 1, { color: RED })])).toEqual([
      { text: 'H', look: { color: RED } }, { text: 'ello ', look: {} }, { text: 'world', look: { bold: true } },
    ]);
    expect(runsOf('a😀b', [run(1, 2, { italic: true })])).toEqual([{ text: 'a', look: {} }, { text: '😀', look: { italic: true } }, { text: 'b', look: {} }]);
    expect(runsOf('plain', [])).toEqual([{ text: 'plain', look: {} }]);
  });
});

describe('the style of a title with its runs', () => {
  it('says nothing of runs where it has none, as the engine writes it', () => {
    const none = withSpans(PLAIN, []);
    expect(none).toEqual(PLAIN);
    expect('spans' in none).toBe(false);
    expect(withSpans(PLAIN, [run(0, 2, { bold: true })])).toEqual({ ...PLAIN, spans: [run(0, 2, { bold: true })] });
    expect('spans' in withSpans({ ...PLAIN, spans: [run(0, 2, { bold: true })] }, [])).toBe(false);
  });
});
