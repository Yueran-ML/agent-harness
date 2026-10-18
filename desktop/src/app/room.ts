/**
 * Where a panel opens that stands above or below its button, and how much room it has there:
 * within the nearest element around it that cuts off what stands out of it, such as the part of
 * the page that scrolls, or else within the window.
 */
import { useLayoutEffect, useState, type RefObject } from 'react';

/** Above or below the button. */
export type Placement = 'up' | 'down';

/** How far a panel keeps from the edge of what holds it, in pixels. */
const EDGE = 8;
/** The least height that a panel is given where it has less room. Its list scrolls then. */
export const LEAST_ROOM = 160;

/** The box of the nearest element around this one that cuts off what stands out of it, or of the window. */
function clipOf(element: HTMLElement): { top: number; bottom: number } {
  for (let around = element.parentElement; around !== null; around = around.parentElement) {
    if (getComputedStyle(around).overflowY !== 'visible') return around.getBoundingClientRect();
  }
  return { top: 0, bottom: window.innerHeight };
}

/**
 * Where an open panel stands, and the height that it has room for, measured as it opens.
 *
 * The panel opens where it is asked to, unless it has less room there than it needs and more on
 * the other side of what it is placed by: then it opens there. The room is from its foot up to
 * what holds it where it opens above, and from its head down where it opens below. The room is
 * null while the panel is closed.
 */
export function useRoom(
  panel: RefObject<HTMLElement | null>, open: boolean, asked: Placement,
): { room: number | null; placement: Placement } {
  const [room, setRoom] = useState<number | null>(null);
  const [placement, setPlacement] = useState<Placement>(asked);
  useLayoutEffect(() => {
    const element = panel.current;
    if (!open || element === null) {
      setRoom(null);
      setPlacement(asked);
      return;
    }
    // The panel stands where it was asked to, as high as all that it holds.
    const box = element.getBoundingClientRect();
    const clip = clipOf(element);
    // What the panel is placed by: above or below it, as far from it on either side.
    const by = (element.offsetParent ?? element.parentElement ?? element).getBoundingClientRect();
    const up = asked === 'up';
    const gap = up ? by.top - box.bottom : box.top - by.bottom;
    const here = Math.max(0, up ? box.bottom - clip.top - EDGE : clip.bottom - box.top - EDGE);
    const there = Math.max(0, up ? clip.bottom - by.bottom - gap - EDGE : by.top - gap - clip.top - EDGE);
    const turned = box.height > here && there > here;
    setPlacement(turned ? (up ? 'down' : 'up') : asked);
    setRoom(Math.max(LEAST_ROOM, Math.floor(turned ? there : here)));
  }, [open, asked]);
  return { room, placement };
}
