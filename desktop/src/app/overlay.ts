/**
 * What lies over the page: a sheet, a menu, a picker, the film of the opening. The view of the
 * studio's engine is a window of the system laid over the viewer, which covers whatever of the
 * page is drawn there; so it is hidden while anything lies over the page, and shown again after.
 *
 * Every part of the page that can lie over the main area says so with `useOverlay` while it is
 * there. `document.body` carries `data-overlay` (how many there are) while any is.
 */
import { useLayoutEffect, useSyncExternalStore } from 'react';

let open = 0;
const listeners = new Set<() => void>();

function changed(by: number): void {
  open = Math.max(0, open + by);
  if (open > 0) document.body.dataset.overlay = String(open);
  else delete document.body.dataset.overlay;
  for (const listener of [...listeners]) listener();
}

/** Whether anything lies over the page now. */
export function overlayOpen(): boolean {
  return open > 0;
}

/** Hear whenever something comes to lie over the page, or goes. */
export function subscribeOverlay(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/**
 * Say that this part of the page lies over the main area while it is there and `lying` holds. It
 * is said before the page is drawn, so that the view is asked to hide as soon as it can be.
 */
export function useOverlay(lying = true): void {
  useLayoutEffect(() => {
    if (!lying) return undefined;
    changed(1);
    return () => changed(-1);
  }, [lying]);
}

/** Whether anything lies over the page, as a part of the page that is drawn again with it. */
export function useOverlayOpen(): boolean {
  return useSyncExternalStore(subscribeOverlay, overlayOpen, overlayOpen);
}
