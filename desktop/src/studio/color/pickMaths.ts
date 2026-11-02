import type { CurvePoint } from '../engine';
import { BOUNDS, VECTOR_SPREAD, addPoint, curveAt, fit, isHueCurve, type HslCurve } from './corrector';
import type { Rgb } from './pickers';
import { LUMA, arcOf, measure } from './qualifierPick';
import type { Edits } from './useNodeEdit';

/**
 * What the pickers of Primaries and Curves make of a colour taken on the viewer (Reference Manual
 * 20, pp. 3044–3045, 3110, 3122; the interaction reference, §3.5, §4.1, §4.3): the colour that
 * goes into the node, as `view.sample` gives it.
 */

/** How far Gamma bends, as the engine has it (`compose`'s `layout::GAMMA_POWER`): the exponent of a Gamma of `g` is `2^(-2g)`. */
const GAMMA_POWER = 2;
const exponentOf = (gamma: number) => 2 ** (-GAMMA_POWER * gamma);
const spow = (x: number, e: number) => Math.sign(x) * Math.abs(x) ** e;
const luma = (rgb: Rgb) => LUMA[0] * rgb[0] + LUMA[1] * rgb[1] + LUMA[2] * rgb[2];

/**
 * Pick Black Point (p. 3044): the Lift that makes the colour black, of each of R, G and B and of
 * Y (whose luma the master mixes in by Lum Mix, p. 3064), where the Lift of `x` is
 * `x + lift · (1 − x)`, 0 at `lift = −x / (1 − x)`. The colour is the node's input, which is what
 * Lift is done to where Color Boost, Temp and Tint and Mid/Detail are at their neutral (else near
 * it). A channel already at white or beyond cannot be made black by Lift: its Lift is kept.
 */
export function blackPointEdits(input: Rgb, current: Record<'y' | 'r' | 'g' | 'b', number>): Edits {
  const liftOf = (x: number, now: number) => (x >= 1 ? now : fit(-x / (1 - x), BOUNDS.lift));
  const [r, g, b] = input;
  return [
    ['primaries.lift.r', liftOf(r, current.r)], ['primaries.lift.g', liftOf(g, current.g)], ['primaries.lift.b', liftOf(b, current.b)],
    ['primaries.lift.y', liftOf(luma(input), current.y)],
  ];
}

/**
 * Pick White Point (p. 3045): the Gain that makes the colour white, of each channel and of Y, with
 * its Lift and Gamma as they are: `gain · (x + lift · (1 − x))^e` is 1 at `gain = 1 / (…)^e`. A
 * channel that Lift and Gamma leave at black or below cannot be made white by Gain: its Gain is
 * kept.
 */
export function whitePointEdits(input: Rgb, lift: Record<'y' | 'r' | 'g' | 'b', number>, gamma: Record<'y' | 'r' | 'g' | 'b', number>,
  current: Record<'y' | 'r' | 'g' | 'b', number>): Edits {
  const gainOf = (x: number, channel: 'y' | 'r' | 'g' | 'b') => {
    const lifted = spow(x + lift[channel] * (1 - x), exponentOf(gamma[channel]));
    return lifted > 1e-4 ? fit(1 / lifted, BOUNDS.gain) : current[channel];
  };
  const [r, g, b] = input;
  return [
    ['primaries.gain.r', gainOf(r, 'r')], ['primaries.gain.g', gainOf(g, 'g')], ['primaries.gain.b', gainOf(b, 'b')],
    ['primaries.gain.y', gainOf(luma(input), 'y')],
  ];
}

/**
 * Sampling the viewer with the custom curves open (p. 3110): "adds a control point to the
 * currently open curve at the position corresponding to the picked pixel", on the curve as it is
 * (it does not change it): ganged, on the curve of Y at the colour's luma (the four curves
 * alike); else on the channel's curve at that channel. A point too near another is not added.
 */
export function customPickPoints(points: readonly CurvePoint[], input: Rgb, channel: 'y' | 'r' | 'g' | 'b'): CurvePoint[] | null {
  const x = channel === 'y' ? luma(input) : input[{ r: 0, g: 1, b: 2 }[channel]];
  const at = Math.min(1, Math.max(0, x));
  return addPoint(points, [at, curveAt(points, at, 'custom')], true)?.points ?? null;
}

/** What an HSL curve reads of a colour, 0 to 1: its hue, its saturation or its luma (p. 3122). */
export function hslInputOf(curve: HslCurve, input: Rgb): number {
  const seen = measure(input);
  if (isHueCurve(curve)) return seen.hue / 100;
  return (curve === 'lum_vs_sat' ? seen.lum : seen.sat) / 100;
}

const round3 = (value: number) => Math.round(value * 1000) / 1000;
const GAP = 0.005;

/**
 * Sampling the viewer with an HSL curve open (p. 3122): "three control points" on the curve as it
 * is, two at the edges of the colours taken and one between, which the middle one adjusts. A
 * click puts the edges as far from it as the vector patches do (`VECTOR_SPREAD`, 30° of hue);
 * a drag puts them half that beyond the colours it went over. The points they would crowd go.
 */
export function hslPickPoints(curve: HslCurve, points: readonly CurvePoint[], samples: readonly Rgb[]): CurvePoint[] {
  const round = isHueCurve(curve);
  const xs = samples.map(each => hslInputOf(curve, each));
  let [low, high] = [Math.min(...xs), Math.max(...xs)];
  if (round) {
    const arc = arcOf(xs.map(x => x * 100));
    low = arc.start / 100; high = low + arc.length / 100;
  }
  const spread = high - low > GAP ? VECTOR_SPREAD / 2 : VECTOR_SPREAD;
  let wanted = [low - spread, (low + high) / 2, high + spread];
  wanted = round ? wanted.map(x => ((x % 1) + 1) % 1) : wanted.map(x => Math.min(1, Math.max(0, x)));
  const kind = round ? 'round' : 'flat';
  const distance = (x: number, w: number) => (round ? Math.min(Math.abs(x - w), 1 - Math.abs(x - w)) : Math.abs(x - w));
  const kept = points.filter(([x]) => wanted.every(w => distance(x, w) >= 2 * GAP));
  const added = [...new Set(wanted.map(round3))].map((x): CurvePoint => [x, round3(curveAt(points, x, kind))]);
  return [...kept, ...added].sort((a, c) => a[0] - c[0]);
}
