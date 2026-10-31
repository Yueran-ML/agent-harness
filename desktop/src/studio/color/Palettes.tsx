import type { KeyboardEvent } from 'react';
import { useWords } from '../../app/locale';

/**
 * The palettes of the Color page, in DaVinci Resolve's order (Reference Manual 20, pp. 2975–2977):
 * the left ones (of which this studio has Primaries), the center ones (Curves, Qualifier, Window,
 * Tracker, Blur, Key), and at the right the Keyframe Editor or the scopes. Palettes this studio
 * does not have are not shown, not even greyed (the interaction reference, §1.1).
 */
export const LEFT_PALETTES = ['primaries'] as const;
export const CENTER_PALETTES = ['curves', 'qualifier', 'window', 'tracker', 'blur', 'key'] as const;
export const RIGHT_PANELS = ['keyframes', 'scopes'] as const;
export type LeftPalette = typeof LEFT_PALETTES[number];
export type CenterPalette = typeof CENTER_PALETTES[number];
export type RightPanel = typeof RIGHT_PANELS[number];
export type Palette = LeftPalette | CenterPalette | RightPanel;

/** One group of palettes: its buttons along its top, and the palette chosen beneath them, which T17–T19 fill. */
function PaletteGroup<T extends Palette>({ name, palettes, chosen, onChoose, side }: {
  name: string; palettes: readonly T[]; chosen: T; onChoose: (palette: T) => void; side: 'left' | 'center' | 'right';
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage.palette;
  // The buttons as a keyboard knows them: one takes the Tab key, the arrows go from one to the other.
  const keys = (event: KeyboardEvent) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    event.stopPropagation();
    const at = palettes.indexOf(chosen);
    const next = palettes[(at + (event.key === 'ArrowRight' ? 1 : palettes.length - 1)) % palettes.length];
    onChoose(next);
    event.currentTarget.querySelector<HTMLElement>(`[data-palette="${next}"]`)?.focus();
  };
  return <section className="color-palette-group" data-side={side} aria-label={name} data-test={`palettes-${side}`}>
    <div className="color-palette-tabs" role="tablist" aria-label={name} onKeyDown={keys}>
      {palettes.map(each => <button key={each} type="button" role="tab" aria-selected={each === chosen} tabIndex={each === chosen ? 0 : -1}
        data-palette={each} onClick={() => onChoose(each)}>{labels[each]}</button>)}
    </div>
    <div className="color-palette" role="tabpanel" aria-label={labels[chosen]} data-palette-shown={chosen}>
      <p className="caption color-empty">{words.hint.studio.colorPage.palette}</p>
    </div>
  </section>;
}

/**
 * The palette area at the foot of the page. Where the page is narrow (DaVinci Resolve merges them
 * below 1920×1080, p. 2976; here by the width of the page), the left and center palettes are one
 * group, in one row of buttons, which shows the one of the two chosen last.
 */
export function Palettes({ merged, left, center, last, right, onLeft, onCenter, onRight }: {
  merged: boolean;
  left: LeftPalette; center: CenterPalette;
  /** Which of the two groups was chosen from last. */
  last: 'left' | 'center';
  right: RightPanel;
  onLeft: (palette: LeftPalette) => void; onCenter: (palette: CenterPalette) => void; onRight: (panel: RightPanel) => void;
}) {
  const labels = useWords().label.studio.colorPage;
  const both: readonly (LeftPalette | CenterPalette)[] = [...LEFT_PALETTES, ...CENTER_PALETTES];
  const isLeft = (palette: Palette): palette is LeftPalette => (LEFT_PALETTES as readonly Palette[]).includes(palette);
  return <div className="color-palettes" data-merged={merged || undefined} role="region" aria-label={labels.palettes} data-test="palettes">
    {merged
      ? <PaletteGroup name={labels.palettes} palettes={both} chosen={last === 'left' ? left : center}
        onChoose={each => (isLeft(each) ? onLeft(each) : onCenter(each))} side="center" />
      : <>
        <PaletteGroup name={labels.leftPalettes} palettes={LEFT_PALETTES} chosen={left} onChoose={onLeft} side="left" />
        <PaletteGroup name={labels.centerPalettes} palettes={CENTER_PALETTES} chosen={center} onChoose={onCenter} side="center" />
      </>}
    <PaletteGroup name={labels.rightPanel} palettes={RIGHT_PANELS} chosen={right} onChoose={onRight} side="right" />
  </div>;
}
