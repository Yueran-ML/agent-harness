/**
 * The keys that the menu of the window had, on Windows and Linux where the window has no menu: Ctrl
 * with plus, minus or 0 zooms the page, F11 puts the window in full screen, and from source F12
 * shows the tools of the page. The shell leaves them to the page first, as the studio uses some of
 * them (Ctrl and plus or minus zoom its timeline, F11 and F12 replace and place on top). A key that
 * nothing on the page took is handed back to the shell, which does what the menu did.
 *
 * Whether a key was taken is known once every listener has had it: one added later than this, as a
 * page of the studio adds its own, comes after it. So the answer is looked at a moment later.
 */
import { useEffect } from 'react';
import { onMac } from './system';

/** Whether a key is one of those that the menu had. The shell decides what it does. */
export function menuKey(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'metaKey'>): boolean {
  if (event.altKey || event.metaKey) return false;
  if (event.key === 'F11' || event.key === 'F12') return !event.ctrlKey;
  return event.ctrlKey && (['+', '=', '-', '0'].includes(event.key) || ['NumpadAdd', 'NumpadSubtract', 'Numpad0'].includes(event.code));
}

/** Hand the shell the keys of the menu that the page did not take. */
export function useLeftKeys(): void {
  useEffect(() => {
    const bridge = window.columbina;
    if (!bridge?.leftKey || onMac()) return undefined;
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const down = (event: KeyboardEvent) => {
      if (event.repeat && (event.key === 'F11' || event.key === 'F12') || !menuKey(event)) return;
      const timer = setTimeout(() => {
        timers.delete(timer);
        if (event.defaultPrevented) return;
        void bridge.leftKey({ key: event.key, code: event.code, control: event.ctrlKey, shift: event.shiftKey, alt: event.altKey, meta: event.metaKey })
          .catch(() => {});
      }, 0);
      timers.add(timer);
    };
    window.addEventListener('keydown', down);
    return () => {
      window.removeEventListener('keydown', down);
      for (const timer of timers) clearTimeout(timer);
    };
  }, []);
}
