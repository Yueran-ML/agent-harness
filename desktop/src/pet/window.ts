/**
 * The window of the pet: the pet, moved through the frames of its sheet as the task goes on, its
 * bubble, and what the pointer does with it. A click shows the main window, a drag moves the pet,
 * and the menu is under the other button.
 */
import { animationsOf, bubbleShows, cellOf, COLUMNS, rowOf, type Animation, type RowName } from './animate';
import type { PetBridge, Sheet, Shown } from './bridge';

/**
 * How large the pet is shown: its cells are 192 by 208 pixels, and it stands at this part of that.
 * A pixel of a pet that was drawn four times larger is three pixels of the screen then.
 */
export const SCALE = 0.75;
const CELL_WIDTH = 192;
const CELL_HEIGHT = 208;
/** How far the pointer moves before a press is a drag and no click. */
const DRAG_FROM = 4;

/** Whether a cell of a sheet has something in it, by its column and row. */
export type Filled = (column: number, row: number) => boolean;

export interface PetWindow {
  /** Draw the frame for this moment. The window draws itself at every frame of the screen. */
  draw(now: number): void;
  stop(): void;
}

/**
 * Start the pet in a page.
 *
 * Args:
 *   page: The page, with the pet and its bubble in it.
 *   bridge: What the window can ask of the shell.
 *   measure: Find which cells of a sheet have something in them. It is asked only for a sheet whose
 *     package does not name its rows.
 *   clock: The time, in milliseconds.
 *   still: Whether nothing is to move, as the system asks.
 */
export async function startPet(
  page: Document, bridge: PetBridge, measure: (sheet: string) => Promise<Filled>, clock: () => number, still: boolean,
): Promise<PetWindow> {
  const pet = page.getElementById('pet')!;
  const bubble = page.getElementById('bubble')!;
  let rows: Record<RowName, Animation> | null = null;
  let shown: Shown = { state: 'idle', label: '', bubble: '' };
  let since = clock();

  const showSheet = async (sheet: Sheet | null) => {
    if (sheet === null) {
      rows = null;
      pet.style.backgroundImage = '';
      pet.setAttribute('aria-label', '');
      return;
    }
    const filled = sheet.animations && Object.keys(sheet.animations).length === 9 ? () => true : await measure(sheet.sheet);
    rows = animationsOf(sheet.animations, filled);
    pet.style.backgroundImage = `url("${sheet.sheet}")`;
    pet.style.backgroundSize = `${COLUMNS * CELL_WIDTH * SCALE}px auto`;
    pet.setAttribute('aria-label', sheet.name);
  };
  const show = (next: Shown) => {
    if (next.state !== shown.state || next.label !== shown.label) since = clock();
    shown = next;
    bubble.textContent = next.bubble;
  };

  const first = await bridge.ready();
  show(first.shown);
  await showSheet(first.pet);
  const unsubscribe = [bridge.onShown(show), bridge.onSheet(sheet => { void showSheet(sheet); })];

  // A press is a click until the pointer has gone far enough to be a drag.
  let pressed: { x: number; y: number; dragging: boolean } | null = null;
  pet.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    pressed = { x: event.screenX, y: event.screenY, dragging: false };
    void bridge.drag('start');
  });
  pet.addEventListener('pointermove', (event) => {
    if (!pressed) return;
    if (!pressed.dragging && Math.hypot(event.screenX - pressed.x, event.screenY - pressed.y) < DRAG_FROM) return;
    pressed.dragging = true;
    void bridge.drag('move');
  });
  pet.addEventListener('pointerup', () => {
    if (!pressed) return;
    const dragged = pressed.dragging;
    pressed = null;
    void (dragged ? bridge.drag('end') : bridge.click());
  });
  pet.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    void bridge.menu();
  });

  return {
    draw(now: number) {
      if (rows === null) return;
      const elapsed = now - since;
      const { column, row } = cellOf(rows[rowOf(shown.state, elapsed)], elapsed, still);
      pet.style.backgroundPosition = `${-column * CELL_WIDTH * SCALE}px ${-row * CELL_HEIGHT * SCALE}px`;
      bubble.hidden = shown.bubble === '' || !bubbleShows(shown.state, elapsed);
    },
    stop() {
      for (const one of unsubscribe) one();
    },
  };
}

/** Which cells of a sheet have something in them, as the picture shows it. */
export function measureIn(page: Document): (sheet: string) => Promise<Filled> {
  return (sheet) => new Promise((resolve) => {
    const picture = new Image();
    picture.onload = () => {
      const canvas = page.createElement('canvas');
      canvas.width = picture.naturalWidth;
      canvas.height = picture.naturalHeight;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      if (!context) { resolve(() => true); return; }
      context.drawImage(picture, 0, 0);
      resolve((column, row) => {
        const { data } = context.getImageData(column * CELL_WIDTH, row * CELL_HEIGHT, CELL_WIDTH, CELL_HEIGHT);
        for (let at = 3; at < data.length; at += 4) if (data[at] > 0) return true;
        return false;
      });
    };
    // A picture that cannot be read is taken as full, so that the pet shows what it can.
    picture.onerror = () => resolve(() => true);
    picture.src = sheet;
  });
}
