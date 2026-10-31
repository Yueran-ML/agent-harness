import { useCallback, useState } from 'react';
import { Ellipsis } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { ColorSettings } from '../engine';
import { hdrOffEverywhere, useViewerHdr } from '../viewerDisplay';
import type { Menu } from './ColorMenu';
import { CoveringMenu } from './CoveringMenu';
import { lutMenuItems } from './luts';
import { useFavoriteLuts, useLutLibrary } from './useLuts';
import type { Worked } from './nodeEdits';

/**
 * The options menu (…) of the viewer (Reference Manual 20, p. 2986; the interaction reference,
 * §1.2), with what the plan adds to it: Display HDR On Viewers If Available (p. 244), a preference of
 * this computer that `view.display` gives the engine; and Viewer LUT ▸, the 3D Color Viewer Lookup
 * Table of the project (p. 152), which colours the picture of the viewer only. Its other items
 * (Highlight, Viewer Channels …) come with the tasks that make them.
 */
export function ViewerOptions({ bridge, color, run }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  /** The colour management of the project, whose Color Viewer LUT the menu sets. */
  color: ColorSettings;
  /** Send a change of the project, one transaction. */
  run: (worked: Worked) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const viewer = useViewerHdr(bridge);
  const { library } = useLutLibrary(bridge);
  const favorites = useFavoriteLuts();
  const [menu, setMenu] = useState<Menu | null>(null);
  const close = useCallback(() => setMenu(null), []);
  const chosen = color.viewer_lut['3d'];
  const open = (box: DOMRect) => setMenu({ x: box.left, y: box.bottom, label: labels.viewerOptions, items: [
    { id: 'display-hdr', label: labels.displayHdr, checked: viewer.hdr, run: () => viewer.set(!viewer.hdr) },
    { id: 'viewer-lut', label: labels.viewerLut, items: lutMenuItems(library, favorites, chosen, (name) => {
      if (name !== chosen) run([{ op: 'set_color_settings', settings: { viewer_lut: { '3d': name } } }]);
    }, labels.lutPanel, 'viewer-lut', '3d') },
  ] });
  const off = viewer.hdr && hdrOffEverywhere(viewer.shows);
  return <>
    <button type="button" className="icon-btn" aria-label={labels.viewerOptions} aria-haspopup="menu" aria-expanded={menu !== null}
      title={off ? `${labels.viewerOptions}\n${words.hint.studio.settings.hdrOff}` : labels.viewerOptions}
      onClick={event => (menu ? close() : open(event.currentTarget.getBoundingClientRect()))} data-test="viewer-options"><Ellipsis size={14} /></button>
    {menu && <CoveringMenu menu={menu} onClose={close} />}
  </>;
}
