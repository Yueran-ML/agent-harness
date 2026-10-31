import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { ChevronRight } from 'lucide-react';

/** An item of a right-click menu: a command, or a submenu (▸) of more items. */
export interface MenuItem {
  id: string;
  label: string;
  /** The key that does the same, shown at the right as DaVinci Resolve's menus show it. */
  keys?: string;
  disabled?: boolean;
  /** A check at its left, for a choice of several (a composite mode). */
  checked?: boolean;
  run?: () => void;
  items?: MenuItem[];
}

/** A right-click menu where it was asked for: its items, in window coordinates. */
export interface Menu { x: number; y: number; label: string; items: MenuItem[] }

const enabledItems = (box: HTMLElement | null) => [...(box?.querySelectorAll<HTMLElement>(':scope > .color-menu-entry > [role="menuitem"]:not([disabled])') ?? [])];

function List({ items, label, onClose, onBack, style }: {
  items: MenuItem[]; label: string; onClose: () => void; onBack?: () => void; style: CSSProperties;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState<string | null>(null);
  useLayoutEffect(() => { enabledItems(box.current)[0]?.focus(); }, []);
  const key = (event: KeyboardEvent) => {
    // A key in a submenu is the submenu's.
    if (event.target instanceof Element && event.target.closest('[role="menu"]') !== box.current) return;
    const all = enabledItems(box.current);
    const at = all.indexOf(document.activeElement as HTMLElement);
    const go = (to: number) => { event.preventDefault(); event.stopPropagation(); all[(to + all.length) % all.length]?.focus(); };
    if (event.key === 'ArrowDown') go(at + 1);
    else if (event.key === 'ArrowUp') go(at < 0 ? all.length - 1 : at - 1);
    else if (event.key === 'ArrowRight' && at >= 0) {
      const item = items.find(each => each.id === all[at].dataset.item);
      if (item?.items) { event.preventDefault(); event.stopPropagation(); setOpen(item.id); }
    } else if (event.key === 'ArrowLeft' && onBack) { event.preventDefault(); event.stopPropagation(); onBack(); }
    else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose(); }
  };
  return <div ref={box} className="color-menu" role="menu" aria-label={label} style={style} onKeyDown={key}>
    {items.map((item) => {
      const sub = item.items;
      return <div key={item.id} className="color-menu-entry" role="none" onPointerEnter={() => setOpen(sub ? item.id : null)}>
        <button type="button" role="menuitem" disabled={item.disabled} data-item={item.id}
          data-test={`menu-${item.id}`} aria-label={item.label} aria-keyshortcuts={item.keys} aria-haspopup={sub ? 'menu' : undefined} aria-expanded={sub ? open === item.id : undefined}
          data-checked={item.checked || undefined}
          onClick={() => { if (sub) { setOpen(item.id); return; } onClose(); item.run?.(); }}>
          <span className="color-menu-check" aria-hidden="true">{item.checked ? '✓' : ''}</span>
          <span className="color-menu-label">{item.label}</span>
          {item.keys && <span className="color-menu-keys">{item.keys}</span>}
          {sub && <ChevronRight size={12} aria-hidden="true" />}
        </button>
        {sub && open === item.id && <List items={sub} label={item.label} onClose={onClose} onBack={() => {
          setOpen(null); box.current?.querySelector<HTMLElement>(`[data-item="${item.id}"]`)?.focus();
        }} style={{ left: '100%', top: -5 }} />}
      </div>;
    })}
  </div>;
}

/**
 * A right-click menu of the Color page, as DaVinci Resolve's: its items, each with its key, and
 * submenus (Add Node ▸, Composite Mode ▸). Escape, a choice, or a press elsewhere puts it away;
 * the arrows go along it, and into and out of a submenu. It is laid over the window (a portal of
 * the studio), so that the part of the page it was asked in does not cut it off.
 */
export function ColorMenu({ menu, onClose }: { menu: Menu; onClose: () => void }) {
  const root = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: menu.x, top: menu.y });
  useLayoutEffect(() => {
    const box = root.current?.firstElementChild?.getBoundingClientRect();
    if (!box) return;
    const width = window.innerWidth || box.width;
    const height = window.innerHeight || box.height;
    setPlace({
      left: menu.x + box.width > width ? Math.max(0, width - box.width) : menu.x,
      top: menu.y + box.height > height ? Math.max(0, height - box.height) : menu.y,
    });
  }, [menu]);
  useEffect(() => {
    const away = (event: PointerEvent) => { if (!(event.target instanceof Node) || !root.current?.contains(event.target)) onClose(); };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('blur', onClose);
    return () => { window.removeEventListener('pointerdown', away, true); window.removeEventListener('blur', onClose); };
  }, [onClose]);
  return createPortal(<div ref={root} className="color-menu-layer" data-studio-portal="" data-test="color-menu"
    onPointerDown={event => event.stopPropagation()} onContextMenu={event => event.preventDefault()}>
    <List items={menu.items} label={menu.label} onClose={onClose} style={{ left: place.left, top: place.top }} />
  </div>, document.body);
}
