import { useCallback, useLayoutEffect, useRef } from 'react';
import { overlayMoved, useOverlay } from '../../app/overlay';
import { ColorMenu, type Menu } from './ColorMenu';

/**
 * A right-click menu that may open over the viewer: the engine's view is a window of the system
 * laid over the viewer, which would cover it, so the view is hidden while the menu lies over it.
 *
 * The menu says where it lies, and its submenus with it, so that the view is hidden only where
 * one of them reaches the viewer: a menu opened on the timeline leaves the picture. It says so
 * again whenever it is fitted into the window or a submenu comes or goes. The keys of the page
 * are held back while it is open wherever it lies, as before.
 */
export function CoveringMenu({ menu, onClose }: { menu: Menu; onClose: () => void }) {
  const layer = useRef<HTMLDivElement>(null);
  const boxes = useCallback(() => [...(layer.current?.querySelectorAll('[role="menu"]') ?? [])].map(each => each.getBoundingClientRect()), []);
  useOverlay(true, boxes);
  useLayoutEffect(() => {
    const element = layer.current;
    if (!element || typeof MutationObserver !== 'function') return undefined;
    // A menu is moved by its style as it is fitted into the window, and a submenu is a child of the layer that comes and goes.
    const watcher = new MutationObserver(overlayMoved);
    watcher.observe(element, { childList: true, subtree: true, attributes: true, attributeFilter: ['style'] });
    return () => watcher.disconnect();
  }, []);
  return <ColorMenu menu={menu} onClose={onClose} layer={layer} />;
}
