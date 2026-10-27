/**
 * Where the viewer lies, told to the shell, which lays the engine's view over it, and whether
 * the view is to be shown.
 *
 * The rectangle is in pixels of the screen (`deviceRect`), measured again whenever it may have
 * changed: the viewer is resized (a panel opened, the window resized), it moves without being
 * resized, the page is zoomed (its `devicePixelRatio` changes, and the shell says `measure`).
 * It is told at most once a frame, and only when it changed (or when the shell asked).
 *
 * The view is shown once the viewer has been told, and hidden while anything lies over the page
 * (see `useOverlay`), while the page is hidden, and when the viewer is no longer shown.
 */
import { useEffect, type RefObject } from 'react';
import type { EngineBridge } from '../bridge';
import { overlayOpen, subscribeOverlay } from '../app/overlay';
import { deviceRect, type ViewRect } from './engine';

/** What of the engine the viewer needs. */
export type ViewBridge = Pick<EngineBridge, 'viewRect' | 'viewShow' | 'onEvent'>;

const same = (one: ViewRect, other: ViewRect) =>
  one.x === other.x && one.y === other.y && one.width === other.width && one.height === other.height;

/** Ask the shell something whose answer is nothing to the page: the view is the shell's to keep. */
function quietly(ask: () => Promise<void> | undefined): void {
  try {
    void ask()?.catch(() => {});
  } catch { /* A shell that is closing. */ }
}

/** The ratio of pixels of the screen to those of the page, 1 where the page does not say. */
const ratio = () => (typeof window.devicePixelRatio === 'number' && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1);

/**
 * Keep the shell told where `viewer` lies while `active` holds, and the view shown while nothing
 * lies over it. Once it is no longer active, the view is hidden.
 */
export function useViewerRect(viewer: RefObject<HTMLElement | null>, bridge: ViewBridge | null, active: boolean): void {
  useEffect(() => {
    const element = viewer.current;
    if (!active || bridge === null || element === null) return undefined;
    let ended = false;
    let told: ViewRect | null = null;
    let frame: number | null = null;
    // Whether the next measure is told even where nothing changed: the first, and those the shell asks for.
    let again = true;
    // Whether the viewer has been told once since it became active: the view is shown only after.
    let measured = false;
    let shown: boolean | null = null;

    const showAsItIs = () => {
      const show = measured && !overlayOpen() && document.visibilityState !== 'hidden';
      if (show === shown) return;
      shown = show;
      quietly(() => bridge.viewShow(show));
    };

    const measure = () => {
      frame = null;
      if (ended) return;
      const rect = deviceRect(element.getBoundingClientRect(), ratio());
      const asked = again;
      again = false;
      // Watched again from where it lies now, whether or not that is another pixel.
      watchMoves();
      if (!asked && told !== null && same(told, rect)) return;
      told = rect;
      quietly(() => bridge.viewRect(rect));
      if (!measured) {
        measured = true;
        showAsItIs();
      }
    };
    /** Measure at the next frame, once however often it is asked for before. */
    const schedule = (tell = false) => {
      if (tell) again = true;
      if (frame === null && !ended) frame = requestAnimationFrame(measure);
    };

    // A change of size.
    const sized = typeof ResizeObserver === 'function' ? new ResizeObserver(() => schedule()) : null;
    sized?.observe(element);
    const resized = () => schedule();
    window.addEventListener('resize', resized);

    // A change of the ratio of pixels: the page was zoomed, or the window went to another screen.
    let resolution: MediaQueryList | null = null;
    const ratioChanged = () => {
      watchRatio();
      schedule(true);
    };
    const watchRatio = () => {
      resolution?.removeEventListener?.('change', ratioChanged);
      resolution = typeof matchMedia === 'function' ? matchMedia(`(resolution: ${ratio()}dppx)`) : null;
      resolution?.addEventListener?.('change', ratioChanged);
    };
    watchRatio();

    // A move without a change of size: the viewer is watched in a frame of its own size, which
    // it fills; as it moves, it fills less of it.
    let moves: IntersectionObserver | null = null;
    function watchMoves(threshold = 1) {
      moves?.disconnect();
      moves = null;
      if (ended || typeof IntersectionObserver !== 'function') return;
      const box = element!.getBoundingClientRect();
      if (box.width < 1 || box.height < 1) return;
      const root = document.documentElement;
      const inset = [box.top, root.clientWidth - box.right, root.clientHeight - box.bottom, box.left]
        .map(edge => `${-Math.floor(edge)}px`).join(' ');
      let first = true;
      moves = new IntersectionObserver(([entry]) => {
        const seen = entry?.intersectionRatio ?? threshold;
        const firstSeen = first;
        first = false;
        if (seen === threshold) return;
        // The frame did not fit to the pixel: what is seen at first is where it lies.
        if (firstSeen) watchMoves(seen > 0 ? seen : 1e-7);
        else schedule();
      }, { rootMargin: inset, threshold: Math.min(1, Math.max(0, threshold)) });
      moves.observe(element!);
    }

    // The shell asks for it: the pixels of the screen that the viewer covers changed.
    const unheard = bridge.onEvent((event) => { if (event.kind === 'measure') schedule(true); });

    const overlaid = subscribeOverlay(showAsItIs);
    document.addEventListener('visibilitychange', showAsItIs);

    showAsItIs();
    schedule(true);

    return () => {
      ended = true;
      if (frame !== null) cancelAnimationFrame(frame);
      sized?.disconnect();
      moves?.disconnect();
      window.removeEventListener('resize', resized);
      resolution?.removeEventListener?.('change', ratioChanged);
      unheard();
      overlaid();
      document.removeEventListener('visibilitychange', showAsItIs);
      if (shown !== false) quietly(() => bridge.viewShow(false));
    };
  }, [active, bridge, viewer]);
}
