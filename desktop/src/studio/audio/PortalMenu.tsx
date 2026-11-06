import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject, type SyntheticEvent } from 'react';
import { createPortal } from 'react-dom';
import { useOverlay } from '../../app/overlay';
import '../fairlight/routing.css';

/** Where a menu opens: under an element (a button), or at a point of the window (a right-click). */
export type MenuAnchor = RefObject<HTMLElement | null> | { x: number; y: number };

const MARGIN = 4;
const items = (box: HTMLElement) => [...box.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
const stop = (event: SyntheticEvent) => event.stopPropagation();

/**
 * A small menu of the mixer, laid over the window so that a strip that scrolls or clips its
 * overflow does not cut it off. It stays by its anchor as the page scrolls or the window is
 * resized, and within the window. It takes the focus to its first item; the arrows, Home and End
 * go along it, Enter and Space choose (with their modifiers), and Escape or a press elsewhere
 * puts it away, giving the focus back to where it was.
 */
export function PortalMenu({ anchor, label, onClose, children, className = 'route-menu' }: {
  anchor: MenuAnchor; label: string; onClose: () => void; children: ReactNode; className?: string;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: MARGIN, top: MARGIN });
  const now = useRef({ anchor, onClose });
  now.current = { anchor, onClose };
  const [opener] = useState(() => document.activeElement instanceof HTMLElement && document.activeElement !== document.body
    ? document.activeElement : null);
  useOverlay(true);
  useLayoutEffect(() => {
    const popup = menu.current;
    if (!popup) return;
    const place = () => {
      const at = now.current.anchor, box = popup.getBoundingClientRect();
      const point = 'current' in at ? at.current?.getBoundingClientRect() : { left: at.x, bottom: at.y };
      if (!point) return;
      const left = Math.max(MARGIN, Math.min(point.left, window.innerWidth - box.width - MARGIN));
      const top = Math.max(MARGIN, Math.min(point.bottom, window.innerHeight - box.height - MARGIN));
      setPosition(old => old.left === left && old.top === top ? old : { left, top });
    };
    place();
    items(popup)[0]?.focus();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
      const at = now.current.anchor, button = 'current' in at ? at.current : opener;
      const focus = document.activeElement;
      if (!(focus === null || focus === document.body || popup.contains(focus))) return;
      const restore = () => {
        const now = document.activeElement;
        if (button?.isConnected && (now === null || now === document.body || popup.contains(now))) button.focus({ preventScroll: true });
      };
      // An anchor disabled while the choice was sent is enabled again by the commit that puts
      // the menu away, after this: it takes the focus once that commit is done.
      if (button && 'disabled' in button && button.disabled) queueMicrotask(restore);
      else restore();
    };
  }, [opener]);
  useEffect(() => {
    const away = (event: PointerEvent) => {
      const target = event.target as Node, at = now.current.anchor;
      if (menu.current?.contains(target) || ('current' in at && at.current?.contains(target))) return;
      now.current.onClose();
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, []);
  return createPortal(<div ref={menu} className={className} data-studio-portal="" role="menu" aria-label={label}
    style={{ ...position, maxHeight: 'min(250px, calc(100vh - 8px))', maxWidth: 'calc(100vw - 8px)' }}
    onPointerDown={stop} onMouseDown={stop} onClick={stop} onDoubleClick={stop} onContextMenu={event => { event.preventDefault(); stop(event); }}
    onKeyDown={event => {
      if (event.nativeEvent.isComposing) return;
      const buttons = items(menu.current!), at = buttons.indexOf(document.activeElement as HTMLButtonElement);
      if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
        const step = event.key === 'ArrowDown' ? 1 : -1;
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (at + step + buttons.length) % buttons.length;
        buttons[next]?.focus();
      } else if (event.key === 'Escape') now.current.onClose();
      else if (['Enter', ' '].includes(event.key) && at >= 0) {
        buttons[at].dispatchEvent(new MouseEvent('click', {
          bubbles: true, ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey,
        }));
      } else {
        // Tab leaves the menu, which is put away; any other key is not the menu's.
        if (event.key === 'Tab') now.current.onClose();
        return;
      }
      event.preventDefault();
      event.stopPropagation();
    }}>{children}</div>, document.body);
}
