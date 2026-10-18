// @vitest-environment jsdom
/**
 * Where a panel opens that stands above or below its button, and how much room it has there:
 * within what cuts it off, or else within the window, and never less than a list needs to be of
 * use.
 */
import { cleanup, render } from '@testing-library/react';
import { useRef, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { LEAST_ROOM, useRoom, type Placement } from '../src/app/room';

/** Where each element stands, by the name in its data-box, top and bottom. */
let boxes: Record<string, [number, number]> = {};
const asBrowsersHaveIt = Element.prototype.getBoundingClientRect;

beforeEach(() => {
  boxes = {};
  Element.prototype.getBoundingClientRect = function getBoundingClientRect(this: Element) {
    const [top, bottom] = boxes[(this as HTMLElement).dataset.box ?? ''] ?? [0, 0];
    return { top, bottom, left: 0, right: 0, width: 0, height: bottom - top, x: 0, y: top, toJSON() {} } as DOMRect;
  };
});

afterEach(() => {
  cleanup();
  Element.prototype.getBoundingClientRect = asBrowsersHaveIt;
});

/** A button with its panel, which says where it stands and the room that it has. */
function Picker({ open, placement }: { open: boolean; placement: Placement }) {
  const panel = useRef<HTMLDivElement>(null);
  const { room, placement: side } = useRoom(panel, open, placement);
  return <div data-box="button">
    <div ref={panel} data-box="panel" data-room={room ?? 'none'} data-placement={side} />
  </div>;
}

const panel = () => document.querySelector<HTMLElement>('[data-box="panel"]')!;
const roomOf = () => panel().dataset.room;
const sideOf = () => panel().dataset.placement;
/** The picker within an element that cuts off what stands out of it, from its top to its bottom. */
const within = (open: boolean, placement: Placement) => <div data-box="clip" style={{ overflowY: 'auto' }}>
  <Picker open={open} placement={placement} />
</div>;
const drawn = (picker: ReactNode) => render(<div data-box="page">{picker}</div>);

describe('the room of a panel', () => {
  it('is measured within the nearest element around it that cuts off what stands out of it', () => {
    boxes = { outer: [0, 1000], clip: [100, 700], panel: [400, 520], button: [528, 560] };
    drawn(<div data-box="outer" style={{ overflowY: 'hidden' }}>
      <div data-box="clip" style={{ overflowY: 'auto' }}>
        <div style={{ overflowY: 'visible' }}><Picker open placement="up" /></div>
      </div>
    </div>);
    // From the foot of the panel up to the head of what cuts it off, less the edge that it keeps.
    expect([sideOf(), roomOf()]).toEqual(['up', String(520 - 100 - 8)]);
  });

  it('is measured from its head down where it opens below its button', () => {
    boxes = { clip: [100, 700], button: [210, 246], panel: [250, 300] };
    drawn(within(true, 'down'));
    expect([sideOf(), roomOf()]).toEqual(['down', String(700 - 250 - 8)]);
  });

  it('is measured within the window where nothing cuts it off', () => {
    boxes = { button: [256, 296], panel: [300, 340] };
    drawn(<div style={{ overflowY: 'visible' }}><Picker open placement="down" /></div>);
    expect(roomOf()).toBe(String(Math.floor(window.innerHeight - 300 - 8)));
  });

  it('is never less than a list needs to be of use', () => {
    boxes = { clip: [100, 700], panel: [150, 200], button: [208, 240] };
    drawn(within(true, 'up'));
    expect([sideOf(), roomOf()]).toEqual(['up', String(LEAST_ROOM)]);
  });

  it('is none while the panel is closed, and measured again when it opens', () => {
    boxes = { clip: [0, 600], panel: [100, 500], button: [508, 540] };
    const { rerender } = drawn(within(false, 'up'));
    expect(roomOf()).toBe('none');
    rerender(<div data-box="page">{within(true, 'up')}</div>);
    expect(roomOf()).toBe(String(500 - 8));
    rerender(<div data-box="page">{within(false, 'up')}</div>);
    expect(roomOf()).toBe('none');
  });
});

describe('where a panel opens', () => {
  it('is below its button where it has too little room above and more below', () => {
    // A panel of 400 has 284 above its button, and below it 900 - 330 less the gap and the edge.
    boxes = { clip: [0, 900], button: [300, 330], panel: [-108, 292] };
    drawn(within(true, 'up'));
    expect([sideOf(), roomOf()]).toEqual(['down', String(900 - 330 - 8 - 8)]);
  });

  it('is above its button where it has too little room below and more above', () => {
    boxes = { clip: [0, 800], button: [600, 630], panel: [634, 1034] };
    drawn(within(true, 'down'));
    expect([sideOf(), roomOf()]).toEqual(['up', String(600 - 4 - 8)]);
  });

  it('is where it was asked to where it has the room there, or more there than on the other side', () => {
    boxes = { clip: [0, 900], button: [500, 530], panel: [292, 492] };
    drawn(within(true, 'up'));
    expect([sideOf(), roomOf()]).toEqual(['up', String(492 - 8)]);
    cleanup();
    // Too little room above, and less below.
    boxes = { clip: [0, 600], button: [400, 430], panel: [-8, 392] };
    drawn(within(true, 'up'));
    expect([sideOf(), roomOf()]).toEqual(['up', String(392 - 8)]);
  });

  it('is where it was asked to where nothing is laid out, and so nothing needs room', () => {
    drawn(within(true, 'up'));
    expect([sideOf(), roomOf()]).toEqual(['up', String(LEAST_ROOM)]);
  });

  it('is where it was asked to each time it opens', () => {
    boxes = { clip: [0, 900], button: [300, 330], panel: [-108, 292] };
    const { rerender } = drawn(within(true, 'up'));
    expect(sideOf()).toBe('down');
    rerender(<div data-box="page">{within(false, 'up')}</div>);
    expect(sideOf()).toBe('up');
    // Opened again, it is measured where it was asked to stand.
    boxes = { clip: [0, 900], button: [500, 530], panel: [292, 492] };
    rerender(<div data-box="page">{within(true, 'up')}</div>);
    expect([sideOf(), roomOf()]).toEqual(['up', String(492 - 8)]);
  });
});
