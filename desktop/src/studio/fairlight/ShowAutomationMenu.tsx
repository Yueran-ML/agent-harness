import { useState, type MouseEvent } from 'react';
import { PortalMenu } from '../audio/PortalMenu';
import { useAutomationTouch } from './AutomationTouch';
import type { AutomationTarget } from './automation';
import { useFairlightText } from './words';

/**
 * The right-click menu of a control of the mixer that shows its automation on the timeline
 * (p. 3896, 3909; interaction reference §11.1): a switch, a module's enable, a discrete button.
 * `open(target)` is the control's `onContextMenu`; `menu` is drawn anywhere in the control.
 * Without the Fairlight automation (the Edit page), the browser's menu is left alone.
 */
export function useShowAutomation() {
  const t = useFairlightText(), automation = useAutomationTouch();
  const [menu, setMenu] = useState<{ x: number; y: number; target: AutomationTarget } | null>(null);
  const open = (target: AutomationTarget | null) => (event: MouseEvent) => {
    if (!automation || !target)
      return;
    event.preventDefault();
    event.stopPropagation();
    setMenu({ x: event.clientX, y: event.clientY, target });
  };
  const element = menu && <PortalMenu anchor={menu} label={t('Show automation')} onClose={() => setMenu(null)}>
    <button type="button" role="menuitem" onClick={() => {
      automation?.show(menu.target);
      setMenu(null);
    }}>
      {t('Show automation in timeline')}
    </button>
    <button type="button" role="menuitem" onClick={() => setMenu(null)}>{t('Cancel')}</button>
  </PortalMenu>;
  return { open, menu: element };
}
