/**
 * The settings of a corrector node as the palettes of the Color page edit them (Reference Manual
 * 20, pp. 3058–3126, and the interaction reference, §3–4 and §10): their defaults and ranges as
 * the engine keeps them (`engine/crates/model/src/grade.rs`, `bounds`), the value a control shows
 * and the keyframe that an edit goes into at the playhead, what each palette has changed, the
 * maths of the wheels and of the curves as the engine draws them, and Auto Color. Pure, so that
 * the palettes and their tests work from the same rules.
 */
import { mergePatch, type Corrector, type CurvePoint, type HslCurves, type Keyframe, type Param } from '../engine';

/** The value that changes nothing, which a new node has, and the values a setting may take. */
export interface Bounds { neutral: number; low: number; high: number }
const b = (neutral: number, low: number, high: number): Bounds => ({ neutral, low, high });

/** The bounds of the settings, as the engine's model has them (PROTOCOL.md, "Grades"). */
export const BOUNDS = {
  lift: b(0, -1, 1),
  gamma: b(0, -1, 1),
  gain: b(1, 0, 16),
  offset: b(25, -75, 125),
  contrast: b(1, 0, 2),
  pivot: b(0.435, 0, 1),
  saturation: b(50, 0, 100),
  hue: b(50, 0, 100),
  lum_mix: b(100, 0, 100),
  temperature: b(0, -4000, 4000),
  tint: b(0, -100, 100),
  color_boost: b(0, -100, 100),
  midtone_detail: b(0, -100, 100),
  shadows: b(0, -100, 100),
  highlights: b(0, -100, 100),
  log_wheel: b(0, -1, 1),
  low_range: b(0.333, 0, 1),
  high_range: b(0.55, 0, 1),
  intensity: b(100, 0, 100),
  clip_low: b(0, 0, 100),
  clip_high: b(100, 0, 100),
  clip_soft: b(0, 0, 100),
} as const satisfies Record<string, Bounds>;

const wheel = (bounds: Bounds) => ({ y: bounds.neutral, r: bounds.neutral, g: bounds.neutral, b: bounds.neutral });
const rgb = (bounds: Bounds) => ({ r: bounds.neutral, g: bounds.neutral, b: bounds.neutral });
const diagonal = (): CurvePoint[] => [[0, 0], [1, 1]];

/** A corrector that changes nothing, as the engine makes a new one. */
export function neutralCorrector(): Corrector {
  return {
    primaries: {
      lift: wheel(BOUNDS.lift), gamma: wheel(BOUNDS.gamma), gain: wheel(BOUNDS.gain), offset: rgb(BOUNDS.offset),
      contrast: BOUNDS.contrast.neutral, pivot: BOUNDS.pivot.neutral, saturation: BOUNDS.saturation.neutral, hue: BOUNDS.hue.neutral,
      lum_mix: BOUNDS.lum_mix.neutral, temperature: 0, tint: 0, color_boost: 0, midtone_detail: 0, shadows: 0, highlights: 0,
    },
    log: { shadow: rgb(BOUNDS.log_wheel), midtone: rgb(BOUNDS.log_wheel), highlight: rgb(BOUNDS.log_wheel), low_range: BOUNDS.low_range.neutral, high_range: BOUNDS.high_range.neutral },
    curves: {
      ganged: true,
      y: { points: diagonal(), intensity: 100 }, r: { points: diagonal(), intensity: 100 },
      g: { points: diagonal(), intensity: 100 }, b: { points: diagonal(), intensity: 100 },
    },
    hsl: { hue_vs_hue: [], hue_vs_sat: [], hue_vs_lum: [], lum_vs_sat: [], sat_vs_sat: [], sat_vs_lum: [] },
    lut: null,
    soft_clip: { low: rgb(BOUNDS.clip_low), high: rgb(BOUNDS.clip_high), low_soft: rgb(BOUNDS.clip_soft), high_soft: rgb(BOUNDS.clip_soft) },
    key: {
      input: { invert: false, matte: true, gain: 1, offset: 0 }, output: { invert: false, gain: 1, offset: 0 },
      qualifier: { invert: false, matte: true, gain: 1, offset: 0 }, input_blur: { radius: 0, ratio: 0.5 },
    },
    qualifier: null,
    windows: [],
    shot_match: null,
  };
}

/** The params of a node read whole: what they leave out is as a new node has it. */
export function fullCorrector(params: unknown): Corrector {
  return mergePatch(neutralCorrector(), params ?? {}) as Corrector;
}

/** What is at a path of dotted names in an object, or undefined. */
export function getAt(value: unknown, path: string): unknown {
  let at = value;
  for (const name of path.split('.')) {
    if (typeof at !== 'object' || at === null) return undefined;
    at = (at as Record<string, unknown>)[name];
  }
  return at;
}

/** A patch that puts `value` at a path of dotted names, as `set_node` takes it. */
export function patchAt(path: string, value: unknown): Record<string, unknown> {
  return path.split('.').reduceRight<unknown>((inner, name) => ({ [name]: inner }), value) as Record<string, unknown>;
}

/** Several values put at their paths, in one patch. */
export function patchOf(values: ReadonlyMap<string, unknown> | Iterable<[string, unknown]>): Record<string, unknown> {
  let patch: unknown = {};
  for (const [path, value] of values) patch = mergePatch(patch, patchAt(path, value));
  return patch as Record<string, unknown>;
}

// Keyframes (pp. 3384, 3391; the interaction reference, §10).

/**
 * The key that an edit at `frame` goes into, and whose value the controls show (p. 3391): the
 * first, where the playhead is on it or before it; else the last at or before the playhead, which
 * is the one at its left between two keys (the one to its right is left as it is) and the last
 * where the playhead is beyond it. The controls do not follow the values in between (p. 3384).
 */
export function keyAt(keys: readonly Keyframe[], frame: number): number {
  let at = 0;
  for (let i = 1; i < keys.length; i++) if (keys[i].frame <= frame) at = i;
  return at;
}

/** Whether a setting has keyframes. */
export const isKeyed = (param: Param | undefined): param is { keys: Keyframe[] } => typeof param === 'object' && param !== null;

/** The value that a control shows of a setting at `frame` (the frame of its grade's keys). */
export function shownValue(param: Param | undefined, frame: number, fallback = 0): number {
  if (param === undefined) return fallback;
  if (!isKeyed(param)) return param;
  return param.keys.length === 0 ? fallback : param.keys[keyAt(param.keys, frame)].value;
}

/** A setting with `value` put where an edit at `frame` puts it: in place of a value, or into its key (p. 3391). */
export function withValue(param: Param | undefined, frame: number, value: number): Param {
  if (!isKeyed(param) || param.keys.length === 0) return value;
  const at = keyAt(param.keys, frame);
  return { keys: param.keys.map((key, i) => (i === at ? { ...key, value } : key)) };
}

/** A number kept to the three decimals that DaVinci Resolve keeps (p. 3062) and within its bounds. */
export function fit(value: number, bounds: Pick<Bounds, 'low' | 'high'>): number {
  const within = Math.min(bounds.high, Math.max(bounds.low, value));
  return Math.round(within * 1000) / 1000;
}

/** Whether a setting is at `neutral` at every key. */
export function isNeutral(param: Param | undefined, neutral: number): boolean {
  if (param === undefined) return true;
  return isKeyed(param) ? param.keys.every(key => key.value === neutral) : param === neutral;
}

const sameCurve = (a: readonly CurvePoint[], c: readonly CurvePoint[]) => a.length === c.length && a.every((point, i) => point[0] === c[i][0] && point[1] === c[i][1]);

/** Whether the Primaries palette, in any of its modes (Wheels, Bars, Log), has changed the node. */
export function primariesChanged(corrector: Corrector): boolean {
  const p = corrector.primaries;
  const l = corrector.log;
  const wheelChanged = (w: object, bounds: Bounds) => (Object.values(w) as Param[]).some(each => !isNeutral(each, bounds.neutral));
  return wheelChanged(p.lift, BOUNDS.lift) || wheelChanged(p.gamma, BOUNDS.gamma) || wheelChanged(p.gain, BOUNDS.gain)
    || wheelChanged(p.offset, BOUNDS.offset)
    || (['contrast', 'pivot', 'saturation', 'hue', 'lum_mix', 'temperature', 'tint', 'color_boost', 'midtone_detail', 'shadows', 'highlights'] as const)
      .some(name => !isNeutral(p[name], BOUNDS[name].neutral))
    || wheelChanged(l.shadow, BOUNDS.log_wheel) || wheelChanged(l.midtone, BOUNDS.log_wheel) || wheelChanged(l.highlight, BOUNDS.log_wheel)
    || !isNeutral(l.low_range, BOUNDS.low_range.neutral) || !isNeutral(l.high_range, BOUNDS.high_range.neutral);
}

/** Whether the Curves palette, its custom curves (and Soft Clip) or its HSL curves, has changed the node. */
export function curvesChanged(corrector: Corrector): boolean {
  const c = corrector.curves;
  const custom = ([c.y, c.r, c.g, c.b]).some(curve => !sameCurve(curve.points, diagonal()) || !isNeutral(curve.intensity, BOUNDS.intensity.neutral));
  const hsl = Object.values(corrector.hsl).some(points => points.length > 0);
  const s = corrector.soft_clip;
  const clip = (['r', 'g', 'b'] as const).some(ch => !isNeutral(s.low[ch], BOUNDS.clip_low.neutral) || !isNeutral(s.high[ch], BOUNDS.clip_high.neutral)
    || !isNeutral(s.low_soft[ch], BOUNDS.clip_soft.neutral) || !isNeutral(s.high_soft[ch], BOUNDS.clip_soft.neutral));
  return custom || hsl || clip;
}

// The wheels (pp. 3061–3062).

/**
 * How the wheels answer the pointer: DaVinci Resolve does not say (the interaction reference,
 * §3.2), so these are ours, kept here in one place to be set against it on a real machine.
 *
 * - `reach`: how far each colour channel goes, at most, with the colour balance indicator at the
 *   edge of its ring, in the units the wheel's numbers show.
 * - `follow`: how far the indicator goes for each pixel that the pointer goes, as a share of the
 *   pixel, in a plain drag (a trackball's relative move); with Shift it jumps to the pointer.
 * - `master`: what a pixel of dragging a master wheel (or the ring with Ctrl) left or right adds
 *   to each of its numbers.
 * - `field`: what a pixel of dragging a number box adds.
 */
export const WHEEL_FEEL = {
  lift: { reach: 0.5, master: 0.002, field: 0.002 },
  gamma: { reach: 0.5, master: 0.002, field: 0.002 },
  gain: { reach: 0.5, master: 0.004, field: 0.004 },
  offset: { reach: 50, master: 0.2, field: 0.2 },
  log: { reach: 0.5, master: 0.002, field: 0.002 },
  follow: 0.5,
} as const;

/**
 * The shared settings of the Primaries palette (pp. 3062–3064, 3073), as their controls show
 * them: the decimals that DaVinci Resolve shows (the interaction reference, §3.4), and what a pixel
 * of dragging adds (ours, to be set against it). Also the Bars mode's sliders: the range they show
 * (the manual gives none, p. 3068) and what a pixel of dragging one adds.
 */
export const SETTING_FEEL = {
  temperature: { digits: 1, perPx: 10 },
  tint: { digits: 2, perPx: 0.5 },
  contrast: { digits: 3, perPx: 0.005 },
  pivot: { digits: 3, perPx: 0.002 },
  midtone_detail: { digits: 2, perPx: 0.5 },
  color_boost: { digits: 2, perPx: 0.5 },
  shadows: { digits: 2, perPx: 0.5 },
  highlights: { digits: 2, perPx: 0.5 },
  saturation: { digits: 2, perPx: 0.25 },
  hue: { digits: 2, perPx: 0.25 },
  lum_mix: { digits: 2, perPx: 0.5 },
  low_range: { digits: 3, perPx: 0.002 },
  high_range: { digits: 3, perPx: 0.002 },
  intensity: { digits: 2, perPx: 0.5 },
  clip: { digits: 0, perPx: 0.1 },
  clip_soft: { digits: 2, perPx: 0.25 },
} as const;
export const BAR_FEEL = {
  lift: { low: -1, high: 1, perPx: 0.01 },
  gamma: { low: -1, high: 1, perPx: 0.01 },
  gain: { low: 0, high: 2, perPx: 0.01 },
  offset: { low: -25, high: 75, perPx: 0.5 },
} as const;

/**
 * The luma coefficients of Rec.709, which the engine's grades use until the colour settings of the
 * project say otherwise (T9, `GradeOptions::luma`).
 */
export const LUMA = [0.2126, 0.7152, 0.0722] as const;
const KB = 2 * (1 - LUMA[2]);
const KR = 2 * (1 - LUMA[0]);

/**
 * Where the colour balance indicator of a ring lies for a colour change of `r`, `g` and `b`: its
 * Cb right and its Cr up, as the vectorscope has them, so that red lies up and a little left
 * (103°) and blue right (347°). The part common to the three, the master wheel's, has none.
 */
export function chromaOf(r: number, g: number, bl: number): [number, number] {
  const y = LUMA[0] * r + LUMA[1] * g + LUMA[2] * bl;
  return [(bl - y) / KB, (r - y) / KR];
}

/** The change of R, G and B that moves the indicator by `cb` and `cr`: one that leaves the luma as it is. */
export function rgbOfChroma(cb: number, cr: number): [number, number, number] {
  const r = KR * cr;
  const bl = KB * cb;
  const g = -(LUMA[0] * r + LUMA[2] * bl) / LUMA[1];
  return [r, g, bl];
}

/**
 * The chroma of the edge of a ring: the indicator at the edge moves the channel that moves most by
 * `reach`. Red moves most along Cr, blue along Cb; their scale is the larger of the two.
 */
export const ringChroma = (reach: number): number => reach / Math.max(KR, KB);

/**
 * The colour of R, G and B without the colour balance: the luma of them, put in each (a double
 * click on a ring, p. 3061: "Resets the color adjustment without resetting the corresponding
 * contrast adjustment").
 */
export const withoutColor = (r: number, g: number, bl: number): number => LUMA[0] * r + LUMA[1] * g + LUMA[2] * bl;

// The curves (pp. 3108–3126), drawn as the engine makes them (`compose/src/grade/layout.rs`, `maths.rs`).

/** The slopes of a curve through its points: of the line through the two points about each, and at each end of its segment. */
function slopes(points: readonly CurvePoint[]): number[] {
  const n = points.length;
  if (n < 2) return points.map(() => 0);
  const slope = (a: CurvePoint, c: CurvePoint) => (c[0] > a[0] ? (c[1] - a[1]) / (c[0] - a[0]) : 0);
  return points.map((_, i) => (i === 0 ? slope(points[0], points[1]) : i === n - 1 ? slope(points[n - 2], points[n - 1]) : slope(points[i - 1], points[i + 1])));
}

/**
 * The value of a curve at `x`: a cubic of Hermite between each two points, with the slopes above
 * (not held to rise: p. 3114 warns of the overshoot). A custom curve goes on beyond its ends along
 * their slopes; an HSL curve is flat beyond them, 0 where it has no point, and a curve of hue goes
 * round, its left end meeting its right (p. 3122).
 */
export function curveAt(points: readonly CurvePoint[], x: number, kind: 'custom' | 'flat' | 'round'): number {
  if (points.length === 0) return kind === 'custom' ? x : 0;
  let list: CurvePoint[] = [...points];
  if (kind === 'round') list = [[points[points.length - 1][0] - 1, points[points.length - 1][1]], ...points, [points[0][0] + 1, points[0][1]]];
  const m = slopes(list);
  const first = list[0];
  const last = list[list.length - 1];
  if (x <= first[0] || list.length === 1) return kind === 'custom' ? first[1] + m[0] * (x - first[0]) : first[1];
  if (x >= last[0]) return kind === 'custom' ? last[1] + m[list.length - 1] * (x - last[0]) : last[1];
  let i = 0;
  while (i + 2 < list.length && list[i + 1][0] <= x) i++;
  const [p0, p1] = [list[i], list[i + 1]];
  const dx = p1[0] - p0[0];
  const dy = p1[1] - p0[1];
  const t = (x - p0[0]) / dx;
  const h10 = t * t * t - 2 * t * t + t;
  const h11 = t * t * t - t * t;
  return p0[1] + dy * t + h10 * (dx * m[i] - dy) + h11 * (dx * m[i + 1] - dy);
}

/** How near two points of a curve may be along x: the engine takes them rising, never two at one x. */
export const CURVE_GAP = 0.002;

/** The range of `out` of a curve: 0 to 1 for a custom curve, -1 to 1 for an HSL curve (0 changes nothing). */
export const outRange = (custom: boolean): [number, number] => (custom ? [0, 1] : [-1, 1]);

const round3 = (value: number) => Math.round(value * 1000) / 1000;

/**
 * A point added to a curve, where `x` falls between two of its points (for a custom curve, between
 * its black and white points). Returns the points and where the new one is, or null where it has
 * no place (at the x of another, or beyond the ends of a custom curve).
 */
export function addPoint(points: readonly CurvePoint[], point: CurvePoint, custom: boolean): { points: CurvePoint[]; index: number } | null {
  const [low, high] = outRange(custom);
  const x = round3(Math.min(1, Math.max(0, point[0])));
  const y = round3(Math.min(high, Math.max(low, point[1])));
  if (points.some(each => Math.abs(each[0] - x) < CURVE_GAP)) return null;
  if (custom && points.length > 0 && (x <= points[0][0] || x >= points[points.length - 1][0])) return null;
  const index = points.findIndex(each => each[0] > x);
  const at = index < 0 ? points.length : index;
  return { points: [...points.slice(0, at), [x, y], ...points.slice(at)], index: at };
}

/** A point of a curve moved to `point`, kept between its neighbours along x and within the range of `out`. */
export function movePoint(points: readonly CurvePoint[], index: number, point: CurvePoint, custom: boolean): CurvePoint[] {
  const [low, high] = outRange(custom);
  const left = index > 0 ? points[index - 1][0] + CURVE_GAP : 0;
  const right = index < points.length - 1 ? points[index + 1][0] - CURVE_GAP : 1;
  const x = round3(Math.min(right, Math.max(left, point[0])));
  const y = round3(Math.min(high, Math.max(low, point[1])));
  return points.map((each, i) => (i === index ? [x, y] : each));
}

/** A point taken away (a right click, p. 3109); the black and white points of a custom curve stay. */
export function removePoint(points: readonly CurvePoint[], index: number, custom: boolean): CurvePoint[] | null {
  if (index < 0 || index >= points.length) return null;
  if (custom && (index === 0 || index === points.length - 1)) return null;
  return points.filter((_, i) => i !== index);
}

/** The hues of the six vector patches beneath a curve of hue (p. 3123): red, yellow, green, cyan, blue, magenta. */
export const VECTORS = { red: 0, yellow: 1 / 6, green: 1 / 3, cyan: 1 / 2, blue: 2 / 3, magenta: 5 / 6 } as const;
export type Vector = keyof typeof VECTORS;
/**
 * How far the two outer points of a patch lie from its middle one: the manual does not say
 * (the interaction reference, §4.3); we take half of a sixth of the hues, 30°.
 */
export const VECTOR_SPREAD = 1 / 12;

/** The three points of a vector patch put on a curve of hue, on the curve as it is; the points they take the place of go. */
export function addVector(points: readonly CurvePoint[], hue: number): CurvePoint[] {
  const centre = hue;
  const wanted = [centre - VECTOR_SPREAD, centre, centre + VECTOR_SPREAD].map(x => (x + 1) % 1);
  const kept = points.filter(([x]) => wanted.every(w => Math.min(Math.abs(x - w), 1 - Math.abs(x - w)) >= CURVE_GAP));
  const added = wanted.map((x): CurvePoint => [round3(x), round3(curveAt(points, x, 'round'))]);
  return [...kept, ...added].sort((a, c) => a[0] - c[0]);
}

/** Three points put on a custom curve, on it as it is, at a quarter, a half and three quarters (Default Anchors, p. 3115). */
export function defaultAnchors(points: readonly CurvePoint[]): CurvePoint[] {
  let now: CurvePoint[] = [...points];
  for (const x of [0.25, 0.5, 0.75]) {
    const added = addPoint(now, [x, curveAt(now, x, 'custom')], true);
    if (added) now = added.points;
  }
  return now;
}

/** Which HSL curves go round, and which are flat beyond their ends. */
export const HSL_CURVES = ['hue_vs_hue', 'hue_vs_sat', 'hue_vs_lum', 'lum_vs_sat', 'sat_vs_sat', 'sat_vs_lum'] as const satisfies readonly (keyof HslCurves)[];
export type HslCurve = typeof HSL_CURVES[number];
export const isHueCurve = (curve: HslCurve): boolean => curve.startsWith('hue_');

// Auto Color (pp. 3045–3046).

/** The figures of a frame that `scopes.statistics` answers (PROTOCOL.md, "Scopes"), as far as Auto Color reads them. */
export interface FrameStatistics {
  percentiles: Record<'red' | 'green' | 'blue' | 'luma', { '1': number; '50': number; '99': number }>;
}

/**
 * Auto Color: how far it may go, so that it does no harm where the frame is no fair test (the
 * manual says of its own that it is "somewhat hit-or-miss", p. 3046). Ours, to be set against it.
 */
export const AUTO_COLOR = { leastRange: 0.02, mostGain: 2, leastGain: 0.5, mostLift: 0.25 } as const;

/**
 * What Auto Color puts into the Lift and Gain of each channel, from the figures of the frame shown
 * and the lift and gain that the node has now: the black and the white of each channel (the
 * levels of 1% and 99% of its pixels) are brought to the luma-weighted mean of the three, so the
 * blacks and the whites are neutral and their lightness is kept. The frame shown is already
 * through the node, so what it does is put after what the node does, which it can be only where
 * Gamma is neutral: `gain · (x + lift (1 − x))` is `B + A x`, with `A = gain (1 − lift)` and
 * `B = gain · lift`, and a line after a line is a line. Null where the frame has too little range
 * to tell anything by.
 */
export function autoColor(stats: FrameStatistics, now: { lift: [number, number, number]; gain: [number, number, number] }):
  { lift: [number, number, number]; gain: [number, number, number] } | null {
  const channels = ['red', 'green', 'blue'] as const;
  const blacks = channels.map(c => stats.percentiles[c]['1']);
  const whites = channels.map(c => stats.percentiles[c]['99']);
  const black = LUMA[0] * blacks[0] + LUMA[1] * blacks[1] + LUMA[2] * blacks[2];
  const white = LUMA[0] * whites[0] + LUMA[1] * whites[1] + LUMA[2] * whites[2];
  if (white - black < AUTO_COLOR.leastRange || channels.some((_, i) => whites[i] - blacks[i] < AUTO_COLOR.leastRange)) return null;
  const lift: number[] = [];
  const gain: number[] = [];
  for (let i = 0; i < 3; i++) {
    const a = Math.min(AUTO_COLOR.mostGain, Math.max(AUTO_COLOR.leastGain, (white - black) / (whites[i] - blacks[i])));
    const shift = Math.min(AUTO_COLOR.mostLift, Math.max(-AUTO_COLOR.mostLift, black - a * blacks[i]));
    const slope = now.gain[i] * (1 - now.lift[i]);
    const base = now.gain[i] * now.lift[i];
    const newSlope = a * slope;
    const newBase = a * base + shift;
    const g = newSlope + newBase;
    if (!(g > 0)) return null;
    gain.push(fit(g, BOUNDS.gain));
    lift.push(fit(newBase / g, BOUNDS.lift));
  }
  return { lift: lift as [number, number, number], gain: gain as [number, number, number] };
}
