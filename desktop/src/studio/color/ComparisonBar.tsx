import type { ReactNode } from 'react';
import { Columns2, Grid2x2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { WipeStyle } from '../engine';
import type { MenuItem } from './ColorMenu';
import { SPLIT_MODES, WIPE_STYLES, type ReferenceMode, type SplitMode } from './comparison';
import type { Comparison } from './useComparison';

/**
 * The second row of the viewer's toolbar (Reference Manual 20, pp. 2987, 3000, 3003; the
 * interaction reference, §1.2): the Image Wipe and Split Screen buttons, and at its right the
 * settings of the mode that is on: of a wipe its reference mode, its style and Invert Wipe; of a
 * split screen its mode.
 */
export function ComparisonBar({ comparison, children }: {
  comparison: Comparison;
  /** The other buttons of the row (Highlight, p. 2987), after Image Wipe and Split Screen. */
  children?: ReactNode;
}) {
  const labels = useWords().label.studio.colorPage.compare;
  return <div className="color-viewer-modes" role="toolbar" aria-label={labels.modes} data-test="viewer-modes">
    <button type="button" className="icon-btn" aria-pressed={comparison.wiping} aria-label={labels.imageWipe} title={`${labels.imageWipe} (Ctrl+W)`}
      onClick={comparison.toggleWipe} data-test="image-wipe"><Columns2 size={14} /></button>
    <button type="button" className="icon-btn" aria-pressed={comparison.splitting} aria-label={labels.splitScreen} title={`${labels.splitScreen} (Ctrl+Alt+W)`}
      onClick={comparison.toggleSplit} data-test="split-screen"><Grid2x2 size={14} /></button>
    {children}
    <span className="color-gap" />
    {comparison.wiping && <>
      <select className="field color-head-kind" aria-label={labels.referenceMode} value={comparison.referenceMode}
        onChange={(event) => { comparison.setReferenceMode(event.target.value as ReferenceMode); event.currentTarget.blur(); }} data-test="reference-mode">
        {(['gallery', 'timeline'] as const).map(each => <option key={each} value={each}>{labels.referenceModes[each]}</option>)}
      </select>
      <select className="field color-head-kind" aria-label={labels.wipeStyle} value={comparison.wipe.style}
        onChange={(event) => { comparison.setStyle(event.target.value as WipeStyle); event.currentTarget.blur(); }} data-test="wipe-style">
        {WIPE_STYLES.map(each => <option key={each} value={each}>{labels.styles[each]}</option>)}
      </select>
      <button type="button" className="color-tool" aria-pressed={comparison.wipe.invert} title={`${labels.invert} (Alt+W)`} onClick={comparison.invert}
        data-test="invert-wipe">{labels.invert}</button>
    </>}
    {comparison.splitting && <select className="field color-head-kind" aria-label={labels.splitMode} value={comparison.splitMode}
      onChange={(event) => { comparison.setSplitMode(event.target.value as SplitMode); event.currentTarget.blur(); }} data-test="split-mode">
      {SPLIT_MODES.map(each => <option key={each} value={each}>{labels.splitModes[each]}</option>)}
    </select>}
  </div>;
}

/** Split Screen ▸, as the viewer's options menu and its right-click menu have it (p. 3003): the modes, and Show Outline (p. 3004). */
export function splitMenu(comparison: Comparison, labels: ReturnType<typeof useWords>['label']['studio']['colorPage']['compare']): MenuItem {
  return { id: 'split-screen', label: labels.splitScreen, keys: 'Ctrl+Alt+W', items: [
    ...SPLIT_MODES.map(mode => ({
      id: `split-${mode}`, label: labels.splitModes[mode], checked: comparison.splitting && comparison.splitMode === mode,
      run: () => (comparison.splitting && comparison.splitMode === mode ? comparison.toggleSplit() : comparison.setSplitMode(mode)),
    })),
    { id: 'split-outline', label: labels.showOutline, checked: comparison.outline, run: () => comparison.setOutline(!comparison.outline) },
  ] };
}

/**
 * The items of the viewer's options menu (…) for comparisons (p. 2986; the interaction reference,
 * §1.2): Split Screen ▸, Wipe On/Off, Reference Wipe Mode ▸, Wipe Style ▸, Invert Wipe and Gang
 * Timeline Wipe With Current Clip.
 */
export function comparisonMenuItems(comparison: Comparison, labels: ReturnType<typeof useWords>['label']['studio']['colorPage']['compare']): MenuItem[] {
  return [
    splitMenu(comparison, labels),
    { id: 'wipe-on-off', label: labels.wipeOnOff, keys: 'Ctrl+W', checked: comparison.wiping, run: comparison.toggleWipe },
    { id: 'reference-mode', label: labels.referenceMode, items: (['gallery', 'timeline'] as const).map(mode => ({
      id: `reference-${mode}`, label: labels.referenceModes[mode], checked: comparison.referenceMode === mode, run: () => comparison.setReferenceMode(mode),
    })) },
    { id: 'wipe-style', label: labels.wipeStyle, items: WIPE_STYLES.map(style => ({
      id: `style-${style}`, label: labels.styles[style], checked: comparison.wipe.style === style, run: () => comparison.setStyle(style),
    })) },
    { id: 'invert-wipe', label: labels.invert, keys: 'Alt+W', checked: comparison.wipe.invert, run: comparison.invert },
    { id: 'gang', label: labels.gang, checked: comparison.gang, run: () => comparison.setGang(!comparison.gang) },
  ];
}
