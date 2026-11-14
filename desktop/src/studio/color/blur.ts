/**
 * The Blur palette of a corrector (Reference Manual 20, pp. 3454–3456) as its controls edit it:
 * its three modes and which settings each uses, the bounds of the settings as the engine keeps
 * them (`engine/crates/model/src/grade/blur.rs`), the feel of its controls, the edits of a ganged
 * column, and whether the palette has changed the node. Pure, so that the palette and its tests
 * work from the same rules.
 */
import type { BlurMode, BlurSettings, Corrector, Param } from '../engine';
import { getAt, isNeutral, shownValue, type Bounds } from './corrector';

export const BLUR_MODES: readonly BlurMode[] = ['blur', 'sharpen', 'mist'];
export const BLUR_CHANNELS = ['r', 'g', 'b'] as const;
export type BlurChannel = typeof BLUR_CHANNELS[number];

/** The three ganged columns (p. 3454). */
export const BLUR_COLUMNS = ['radius', 'ratio', 'scaling'] as const;
export type BlurColumn = typeof BLUR_COLUMNS[number];
/** The single settings beneath them. */
export const BLUR_SINGLES = ['coring_softness', 'level', 'mix'] as const;
export type BlurSingle = typeof BLUR_SINGLES[number];

const b = (neutral: number, low: number, high: number): Bounds => ({ neutral, low, high });
/** The bounds of the settings, as the pages show them at first (pp. 3454–3456; PROTOCOL.md, "Grades"). */
export const BLUR_BOUNDS = {
  radius: b(0.5, 0, 1),
  ratio: b(0.5, 0, 1),
  scaling: b(0.25, 0, 1),
  coring_softness: b(0, 0, 1),
  level: b(0, 0, 1),
  mix: b(100, 0, 100),
} as const satisfies Record<BlurColumn | BlurSingle, Bounds>;

/**
 * Which settings each mode uses; the others are grey (pp. 3454–3456): the Blur mode, Radius and
 * H/V Ratio; Sharpen, with Scaling, Coring Softness and Level; Mist, with Scaling and Mix.
 */
export function blurUses(mode: BlurMode, setting: BlurColumn | BlurSingle): boolean {
  switch (setting) {
    case 'radius': case 'ratio': return true;
    case 'scaling': return mode !== 'blur';
    case 'coring_softness': case 'level': return mode === 'sharpen';
    case 'mix': return mode === 'mist';
  }
}

/**
 * How the controls answer the pointer (ours, to be set against DaVinci Resolve): the decimals the
 * pages show (0.50, 100.00), what a pixel of dragging a bar or a number adds, what a notch of the
 * wheel over a bar of Radius adds (the only wheel of the palette, p. 3454: "use the scroll wheel on
 * your mouse while hovering over one of the bars to increase the radius"), and how long the wheel
 * rests before the notches of a turn are one step to undo (well within the 10 seconds after which
 * the engine ends a gesture that it hears nothing of, PROTOCOL.md, "Gestures").
 */
export const BLUR_FEEL = {
  digits: 2,
  barPerPx: 0.005,
  fieldPerPx: 0.002,
  mixPerPx: 0.25,
  wheelStep: 0.01,
  wheelRestMs: 1000,
} as const;

/** The settings of the palette as a new node has them. */
export function neutralBlur(): BlurSettings {
  const rgb = (bounds: Bounds) => ({ r: bounds.neutral, g: bounds.neutral, b: bounds.neutral });
  return {
    mode: 'blur',
    radius: rgb(BLUR_BOUNDS.radius), ratio: rgb(BLUR_BOUNDS.ratio), scaling: rgb(BLUR_BOUNDS.scaling),
    coring_softness: 0, level: 0, mix: 100,
    linked: { radius: true, ratio: true, scaling: true },
  };
}

/** The palette of a corrector, as at first where an older file leaves it out. */
export const blurOf = (corrector: Corrector): BlurSettings => corrector.blur ?? neutralBlur();

/** Whether the palette has changed the node: a setting of it, in any mode, is not as at first. */
export function blurChanged(corrector: Corrector): boolean {
  const blur = blurOf(corrector);
  return BLUR_COLUMNS.some(column => BLUR_CHANNELS.some(ch => !isNeutral(blur[column][ch], BLUR_BOUNDS[column].neutral)))
    || BLUR_SINGLES.some(single => !isNeutral(blur[single], BLUR_BOUNDS[single].neutral));
}

const keep = (value: number, bounds: Bounds) => Math.round(Math.min(bounds.high, Math.max(bounds.low, value)) * 1000) / 1000;

/** The paths of a column that an edit of channel `ch` changes: all three where it is ganged, its own otherwise. */
export function columnPaths(corrector: Corrector, column: BlurColumn, ch: BlurChannel): string[] {
  return blurOf(corrector).linked[column] ? BLUR_CHANNELS.map(each => `blur.${column}.${each}`) : [`blur.${column}.${ch}`];
}

/**
 * The edits that put channel `ch` of a column at `value`: ganged, the others move by as much as it
 * does from where they all were (`base`, the node as it was before the edit), each kept within the
 * bounds, so that ganging again does not make them alike (the integration notes); not ganged, it alone.
 */
export function columnEdits(base: Corrector, frame: number, column: BlurColumn, ch: BlurChannel, value: number): [string, number][] {
  const bounds = BLUR_BOUNDS[column];
  const at = (each: BlurChannel) => shownValue(getAt(base, `blur.${column}.${each}`) as Param | undefined, frame, bounds.neutral);
  const delta = keep(value, bounds) - at(ch);
  return columnPaths(base, column, ch).map(path => {
    const each = path.slice(-1) as BlurChannel;
    return [path, each === ch ? keep(value, bounds) : keep(at(each) + delta, bounds)];
  });
}
