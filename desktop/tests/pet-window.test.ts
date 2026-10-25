// @vitest-environment jsdom
/** The window of the pet: the frames that it shows, its bubble, and what the pointer does with it. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BUBBLE_MS, ROWS } from '../src/pet/animate';
import type { PetBridge, Sheet, Shown } from '../src/pet/bridge';
import { measureIn, SCALE, startPet } from '../src/pet/window';

const ALL = Object.fromEntries(ROWS.map((name, row) => [name, { row, frames: 4, fps: 10 }]));
const sheet = (given: Partial<Sheet> = {}): Sheet => ({ sheet: 'data:image/png;base64,AAAA', animations: ALL, name: 'Little Moon', ...given });
const idle: Shown = { state: 'idle', label: '', bubble: '' };

/** A shell that answers the window, and hands it what a test tells it. */
function shell(first: { pet: Sheet | null; shown: Shown }) {
  const heard: { shown: ((shown: Shown) => void)[]; sheet: ((pet: Sheet | null) => void)[] } = { shown: [], sheet: [] };
  const bridge = {
    ready: vi.fn(async () => first),
    onShown: vi.fn((callback: (shown: Shown) => void) => { heard.shown.push(callback); return () => { heard.shown = heard.shown.filter(one => one !== callback); }; }),
    onSheet: vi.fn((callback: (pet: Sheet | null) => void) => { heard.sheet.push(callback); return () => { heard.sheet = heard.sheet.filter(one => one !== callback); }; }),
    click: vi.fn(async () => {}),
    drag: vi.fn(async (_phase: 'start' | 'move' | 'end') => {}),
    menu: vi.fn(async () => {}),
  } satisfies PetBridge;
  return { bridge, heard };
}

let now = 0;
const clock = () => now;
const pet = () => document.getElementById('pet')!;
const bubble = () => document.getElementById('bubble')!;
const at = (column: number, row: number) => `${-column * 192 * SCALE}px ${-row * 208 * SCALE}px`;
const point = (type: string, x: number, y: number, button = 0) =>
  pet().dispatchEvent(new MouseEvent(type, { button, screenX: x, screenY: y, bubbles: true, cancelable: true }));

beforeEach(() => {
  now = 1000;
  document.body.innerHTML = '<div class="pet-bubble" id="bubble" hidden></div><div class="pet" id="pet" role="img"></div>';
});
afterEach(() => vi.unstubAllGlobals());

describe('the pet in its window', () => {
  it('shows its sheet, and goes through the frames of what it does', async () => {
    const { bridge } = shell({ pet: sheet(), shown: idle });
    const measure = vi.fn();
    const window = await startPet(document, bridge, measure, clock, false);
    expect(pet().style.backgroundImage).toBe('url("data:image/png;base64,AAAA")');
    expect(pet().style.backgroundSize).toBe(`${8 * 192 * SCALE}px`);
    expect(pet().getAttribute('aria-label')).toBe('Little Moon');
    // A package that names all its rows is not measured.
    expect(measure).not.toHaveBeenCalled();
    window.draw(1000);
    expect(pet().style.backgroundPosition).toBe(at(0, 0));
    window.draw(1100);
    expect(pet().style.backgroundPosition).toBe(at(1, 0));
    expect(bubble().hidden).toBe(true);
  });

  it('turns to what the task does as it is told, from the first frame of that', async () => {
    const { bridge, heard } = shell({ pet: sheet(), shown: idle });
    const window = await startPet(document, bridge, vi.fn(), clock, false);
    now = 5000;
    heard.shown[0]({ state: 'review', label: 'read', bubble: '' });
    window.draw(5000);
    expect(pet().style.backgroundPosition).toBe(at(0, 8));
    window.draw(5250);
    expect(pet().style.backgroundPosition).toBe(at(2, 8));
    // The same state told again goes on where it was.
    now = 5300;
    heard.shown[0]({ state: 'review', label: 'read', bubble: '' });
    window.draw(5300);
    expect(pet().style.backgroundPosition).toBe(at(3, 8));
    // Another tool is a new beginning.
    heard.shown[0]({ state: 'review', label: 'grep', bubble: '' });
    window.draw(5300);
    expect(pet().style.backgroundPosition).toBe(at(0, 8));
  });

  it('says in its bubble that it waits as long as it does, and what happened for a while', async () => {
    const { bridge, heard } = shell({ pet: sheet(), shown: { state: 'waiting', label: 'shell', bubble: 'Waiting for your yes to shell.' } });
    const window = await startPet(document, bridge, vi.fn(), clock, false);
    window.draw(1000 + 60_000);
    expect([bubble().hidden, bubble().textContent]).toEqual([false, 'Waiting for your yes to shell.']);
    now = 70_000;
    heard.shown[0]({ state: 'done', label: '', bubble: 'Done.' });
    window.draw(70_000 + BUBBLE_MS - 1);
    expect([bubble().hidden, bubble().textContent]).toEqual([false, 'Done.']);
    window.draw(70_000 + BUBBLE_MS);
    expect(bubble().hidden).toBe(true);
    // Done, and nothing to say: the user was looking.
    now = 90_000;
    heard.shown[0]({ state: 'failed', label: '', bubble: '' });
    window.draw(90_000);
    expect(bubble().hidden).toBe(true);
  });

  it('measures a sheet whose package does not name all its rows', async () => {
    const { bridge } = shell({ pet: sheet({ animations: { idle: { row: 0, frames: 2, fps: 4 } } }), shown: idle });
    const measure = vi.fn(async () => (column: number) => column < 3);
    const window = await startPet(document, bridge, measure, clock, false);
    expect(measure).toHaveBeenCalledWith('data:image/png;base64,AAAA');
    window.draw(1000 + 750);
    // The idle row is as the package says: two frames, four a second.
    expect(pet().style.backgroundPosition).toBe(at(1, 0));
    const { bridge: other } = shell({ pet: sheet({ animations: null }), shown: { state: 'thinking', label: '', bubble: '' } });
    const another = await startPet(document, other, measure, clock, false);
    another.draw(1000 + 1000);
    // Three frames counted in the picture, five a second.
    expect(pet().style.backgroundPosition).toBe(at(2, 7));
  });

  it('shows nothing without a pet, and the pet that is chosen later', async () => {
    const { bridge, heard } = shell({ pet: null, shown: idle });
    const window = await startPet(document, bridge, vi.fn(), clock, false);
    window.draw(2000);
    expect([pet().style.backgroundImage, pet().style.backgroundPosition]).toEqual(['', '']);
    heard.sheet[0](sheet({ name: 'Little Cat' }));
    await Promise.resolve();
    window.draw(2000);
    expect(pet().getAttribute('aria-label')).toBe('Little Cat');
    // The pet has rested since the window began, a second at ten frames a second.
    expect(pet().style.backgroundPosition).toBe(at(2, 0));
    heard.sheet[0](null);
    await Promise.resolve();
    expect([pet().style.backgroundImage, pet().getAttribute('aria-label')]).toEqual(['', '']);
  });

  it('stands still where the system asks for less motion', async () => {
    const { bridge } = shell({ pet: sheet(), shown: idle });
    const window = await startPet(document, bridge, vi.fn(), clock, true);
    window.draw(1000 + 350);
    expect(pet().style.backgroundPosition).toBe(at(0, 0));
  });

  it('shows the main window at a click, moves at a drag, and opens its menu at the other button', async () => {
    const { bridge } = shell({ pet: sheet(), shown: idle });
    await startPet(document, bridge, vi.fn(), clock, false);
    point('pointerdown', 100, 100);
    point('pointermove', 102, 101);
    point('pointerup', 102, 101);
    expect(bridge.click).toHaveBeenCalledTimes(1);
    expect(bridge.drag.mock.calls).toEqual([['start']]);
    point('pointerdown', 100, 100);
    point('pointermove', 110, 100);
    point('pointermove', 130, 120);
    point('pointerup', 130, 120);
    expect(bridge.drag.mock.calls).toEqual([['start'], ['start'], ['move'], ['move'], ['end']]);
    expect(bridge.click).toHaveBeenCalledTimes(1);
    // Another button presses nothing, and the pointer that moves unpressed drags nothing.
    point('pointerdown', 100, 100, 2);
    point('pointermove', 150, 150);
    point('pointerup', 150, 150);
    expect(bridge.drag).toHaveBeenCalledTimes(5);
    expect(bridge.click).toHaveBeenCalledTimes(1);
    expect(pet().dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))).toBe(false);
    expect(bridge.menu).toHaveBeenCalledTimes(1);
  });

  it('hears nothing more once it is stopped', async () => {
    const { bridge, heard } = shell({ pet: sheet(), shown: idle });
    const window = await startPet(document, bridge, vi.fn(), clock, false);
    window.stop();
    expect([heard.shown, heard.sheet]).toEqual([[], []]);
  });
});

describe('the cells of a sheet that have something in them', () => {
  /** A picture that loads, or fails to, and a canvas that has something in the cells that a test says. */
  function pictures(loads: boolean, filled: (x: number, y: number) => boolean, context = true) {
    class Picture {
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      naturalWidth = 1536;
      naturalHeight = 1872;
      set src(_value: string) { queueMicrotask(() => (loads ? this.onload?.() : this.onerror?.())); }
    }
    vi.stubGlobal('Image', Picture);
    const drawn = vi.fn();
    const canvas = {
      width: 0, height: 0,
      getContext: vi.fn(() => (context ? {
        drawImage: drawn,
        getImageData: (x: number, y: number, width: number, height: number) => {
          const data = new Uint8ClampedArray(width * height * 4);
          if (filled(x, y)) data[data.length - 1] = 255;
          return { data };
        },
      } : null)),
    };
    const page = { createElement: vi.fn(() => canvas) } as unknown as Document;
    return { page, canvas, drawn };
  }

  it('are those whose pixels show anything', async () => {
    const { page, canvas, drawn } = pictures(true, (x, y) => x === 192 && y === 208);
    const filled = await measureIn(page)('data:image/png;base64,AAAA');
    expect([canvas.width, canvas.height, drawn]).toEqual([1536, 1872, drawn]);
    expect(drawn).toHaveBeenCalledTimes(1);
    expect([filled(1, 1), filled(0, 0), filled(2, 1)]).toEqual([true, false, false]);
  });

  it('are all of them where the picture cannot be read or drawn', async () => {
    const broken = pictures(false, () => false);
    expect((await measureIn(broken.page)('data:,'))(5, 5)).toBe(true);
    const flat = pictures(true, () => false, false);
    expect((await measureIn(flat.page)('data:,'))(5, 5)).toBe(true);
  });
});
