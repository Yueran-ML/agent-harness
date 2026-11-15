/**
 * What lies over the page: a sheet, a menu, a picker, the film of the opening. The view of the
 * studio's engine is a window of the system laid over the viewer, which covers whatever of the
 * page is drawn there; so it is hidden while anything lies over the page, and shown again after.
 *
 * Every part of the page that can lie over the main area says so with `useOverlay` while it is
 * there. `document.body` carries `data-overlay` (how many there are) while any is.
 *
 * A sheet or a dialog lies over the whole page. A menu lies over a small part of it, and says
 * where (`within`): the view is then hidden only where the menu, or a submenu of it, reaches the
 * viewer, so that a menu opened on the timeline leaves the picture. Whatever cannot be told where
 * it lies is taken to lie over the viewer.
 */
import { useLayoutEffect, useSyncExternalStore } from 'react';

/** A rectangle of the window, as `getBoundingClientRect` gives it. */
export type OverlayBox = Pick<DOMRectReadOnly, 'left' | 'top' | 'right' | 'bottom'>;
/** Where a part of the page that lies over a part of it lies now: every box of it, a menu and its submenus. */
export type OverlayBounds = () => readonly OverlayBox[];

/**
 * How near the viewer, in pixels of the page, a menu is taken to lie over it: its shadow reaches
 * that far around it, and would be cut off at the edge of the view.
 */
export const OVERLAY_MARGIN_PX = 24;

let open = 0;
/** Those that say where they lie, by what each was given when it came. The others lie over the whole page. */
const bounded = new Map<symbol, OverlayBounds>();
const listeners = new Set<() => void>();

function tell(): void {
  for (const listener of [...listeners]) listener();
}

function changed(by: number): void {
  open = Math.max(0, open + by);
  if (open > 0) document.body.dataset.overlay = String(open);
  else delete document.body.dataset.overlay;
  tell();
}

/** Whether anything lies over the page now. */
export function overlayOpen(): boolean {
  return open > 0;
}

const area = (box: OverlayBox) => box.right > box.left && box.bottom > box.top;

/**
 * Whether anything lies over `box`, a part of the page (the viewer). Anything that lies over the
 * whole page does. What says where it lies does where one of its boxes comes within
 * `OVERLAY_MARGIN_PX` of `box`; and where it has no box yet, or one without an area, or `box` has
 * none, nothing can be told, and it is taken to lie over it.
 */
export function overlayCovers(box: OverlayBox): boolean {
  if (open > bounded.size) return true;
  for (const within of bounded.values()) {
    const boxes = within();
    if (boxes.length === 0 || !area(box)) return true;
    for (const each of boxes) {
      if (!area(each)) return true;
      const clear = each.right + OVERLAY_MARGIN_PX <= box.left || each.left - OVERLAY_MARGIN_PX >= box.right
        || each.bottom + OVERLAY_MARGIN_PX <= box.top || each.top - OVERLAY_MARGIN_PX >= box.bottom;
      if (!clear) return true;
    }
  }
  return false;
}

/** Say that something that told where it lies has moved, grown, or opened a submenu: whoever hears of overlays looks again. */
export function overlayMoved(): void {
  if (bounded.size > 0) tell();
}

/** Hear whenever something comes to lie over the page, or goes, or moves where it said it lies. */
export function subscribeOverlay(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/**
 * Say that this part of the page lies over the main area while it is there and `lying` holds. It
 * is said before the page is drawn, so that the view is asked to hide as soon as it can be.
 *
 * `within` is given by what lies over a part of the page only, a menu: it tells where, whenever
 * it is asked, and is the same function from one drawing to the next. Without it, the part lies
 * over the whole page.
 */
export function useOverlay(lying = true, within?: OverlayBounds): void {
  useLayoutEffect(() => {
    if (!lying) return undefined;
    const own = Symbol('overlay');
    if (within) bounded.set(own, within);
    changed(1);
    return () => { bounded.delete(own); changed(-1); };
  }, [lying, within]);
}

/** Whether anything lies over the page, as a part of the page that is drawn again with it. */
export function useOverlayOpen(): boolean {
  return useSyncExternalStore(subscribeOverlay, overlayOpen, overlayOpen);
}
