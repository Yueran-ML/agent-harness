import { useOverlay } from '../../app/overlay';
import { ColorMenu, type Menu } from './ColorMenu';

/**
 * A right-click menu that may open over the viewer: the engine's view is a window of the system
 * laid over the viewer, which would cover it, so the view is hidden while the menu is open.
 */
export function CoveringMenu({ menu, onClose }: { menu: Menu; onClose: () => void }) {
  useOverlay();
  return <ColorMenu menu={menu} onClose={onClose} />;
}
