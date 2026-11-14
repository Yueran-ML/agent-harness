import { newQualifier, type HueRange, type Param, type Qualifier, type QualifierRange } from '../engine';
import type { Rgb } from './pickers';

/**
 * What the Qualifier palette's Selection Range tools make of the colours taken on the viewer
 * (Reference Manual 20, pp. 3166–3168; the interaction reference, §8.1): the Sample Eyedropper a
 * range about them, Add and Subtract Color Range the range grown to take them or shrunk to leave
 * them, Add and Subtract Softness the softness grown or shrunk. The colours are those that go into
 * the node (the qualifier keys its input), read as the engine reads them (`compose`'s `grade`
 * maths): hue a turn from red (0) by green (1/3) and blue (2/3), saturation `(max - min) / max`,
 * luma by BT.709's coefficients (the engine takes those of the timeline space; the same for
 * DaVinci YRGB and Rec.709 timelines), luma and R, G, B kept to 0..1. Every value of a qualifier
 * is a share of 100.
 *
 * The manual gives no figures for what a click makes; these are ours, kept here to be set
 * against DaVinci Resolve (T29).
 */
export const PICK = {
  /** What a click's range of hue is wide, at least, and the softness on each side of it. */
  hueWidth: 6, hueSoft: 4,
  /** How far beyond the colours taken a range of saturation, luma or R, G, B goes, and its softness. */
  margin: 3, soft: 6,
  /** Below this saturation (of 100) the hue of a colour is no hue: a grey takes every hue. */
  grey: 8,
} as const;

export type QualifierTool = 'pick' | 'add' | 'subtract' | 'soften' | 'harden';
/** BT.709's luma coefficients. */
export const LUMA: Rgb = [0.2126, 0.7152, 0.0722];

/** What a qualifier reads of a colour, each of 100 (hue too: a turn). */
export interface Measured { hue: number; sat: number; lum: number; red: number; green: number; blue: number }

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));
const round = (value: number) => Math.round(value * 1000) / 1000;
/** A hue of 100 brought into 0..100. */
const wrap = (hue: number) => ((hue % 100) + 100) % 100;

export function measure([r, g, b]: Rgb, luma: Rgb = LUMA): Measured {
  const a = r - 0.5 * (g + b);
  const c = (Math.sqrt(3) / 2) * (g - b);
  const turn = a === 0 && c === 0 ? 0 : Math.atan2(c, a) / (2 * Math.PI);
  const most = Math.max(r, g, b);
  const least = Math.min(r, g, b);
  const sat = most <= 0 ? 0 : clamp((most - least) / most, 0, 1);
  const lum = clamp(luma[0] * r + luma[1] * g + luma[2] * b, 0, 1);
  return {
    hue: wrap(turn * 100), sat: sat * 100, lum: lum * 100,
    red: clamp(r, 0, 1) * 100, green: clamp(g, 0, 1) * 100, blue: clamp(b, 0, 1) * 100,
  };
}

const value = (param: Param): number => (typeof param === 'number' ? param : param.keys[0]?.value ?? 0);

/** The smallest arc of hue that holds `hues`: where it starts and how long it is, of 100. */
export function arcOf(hues: readonly number[]): { start: number; length: number } {
  const sorted = [...hues].map(wrap).sort((x, y) => x - y);
  if (sorted.length === 0) return { start: 0, length: 0 };
  let gap = -1;
  let after = 0;
  for (let i = 0; i < sorted.length; i++) {
    const next = i + 1 < sorted.length ? sorted[i + 1] : sorted[0] + 100;
    if (next - sorted[i] > gap) { gap = next - sorted[i]; after = (i + 1) % sorted.length; }
  }
  return { start: sorted[after], length: 100 - gap };
}

/** How far forward from `from` to `to` round the hues, 0 to 100. */
const forward = (from: number, to: number) => wrap(to - from);

/** The edges of a range of hue: its left end and its right end, going round. */
const edgesOf = (hue: HueRange) => {
  const center = value(hue.center);
  const half = value(hue.width) / 2;
  return { left: wrap(center - half), width: value(hue.width) };
};
/** The softness of a range of hue on its left and on its right (as the engine shares it by Symmetry). */
const softsOf = (hue: HueRange) => {
  const soft = value(hue.soft);
  const symmetry = value(hue.symmetry) / 50;
  return { left: soft * (2 - symmetry), right: soft * symmetry };
};

/** The range of hue from `left`, `width` long, keeping the rest of `hue`. */
const hueOf = (hue: HueRange, left: number, width: number): HueRange => ({
  ...hue, center: round(wrap(left + clamp(width, 0, 100) / 2)), width: round(clamp(width, 0, 100)),
});

const rangeOf = (range: QualifierRange, low: number, high: number, soft?: [number, number]): QualifierRange => ({
  ...range, low: round(clamp(low, 0, 100)), high: round(clamp(high, 0, 100)),
  ...(soft ? { low_soft: round(clamp(soft[0], 0, 100)), high_soft: round(clamp(soft[1], 0, 100)) } : {}),
});

type Linear = 'sat' | 'lum' | 'red' | 'green' | 'blue';
/** The components that limit a qualifier of each mode (p. 3175: LUM is HSL with H and S off). */
const linearOf = (q: Qualifier): Linear[] => (q.mode === 'rgb' ? ['red', 'green', 'blue'] : q.mode === 'lum' ? ['lum'] : ['sat', 'lum']);
const usesHue = (q: Qualifier) => q.mode === 'hsl';

/**
 * The Sample Eyedropper (p. 3166): a range about the colours taken, from nothing; its mode, its
 * switches, Invert and Matte Finesse kept from `base` (a new qualifier where the node has none).
 */
export function pickRange(samples: readonly Rgb[], base: Qualifier | null | undefined): Qualifier {
  const q = base ?? newQualifier();
  const seen = samples.map(each => measure(each));
  const next: Qualifier = { ...q };
  if (usesHue(q)) {
    const coloured = seen.filter(each => each.sat >= PICK.grey);
    if (coloured.length === 0) next.hue = hueOf(q.hue, 0, 100);
    else {
      const arc = arcOf(coloured.map(each => each.hue));
      const width = Math.max(arc.length + PICK.hueWidth, PICK.hueWidth);
      next.hue = { ...hueOf(q.hue, arc.start - (width - arc.length) / 2, width), soft: PICK.hueSoft, symmetry: 50 };
    }
  }
  for (const name of linearOf(q)) {
    const values = seen.map(each => each[name]);
    next[name] = rangeOf(q[name], Math.min(...values) - PICK.margin, Math.max(...values) + PICK.margin, [PICK.soft, PICK.soft]);
  }
  return next;
}

/** Add Color Range (p. 3166): the ranges grown to take the colours, as little as they must. */
export function addRange(samples: readonly Rgb[], base: Qualifier): Qualifier {
  const next: Qualifier = { ...base };
  for (const each of samples.map(rgb => measure(rgb))) {
    if (usesHue(next) && each.sat >= PICK.grey && value(next.hue.width) < 100) {
      const { left, width } = edgesOf(next.hue);
      const into = forward(left, each.hue);
      if (into > width) {
        // Grown on the side nearer the hue.
        const beyondRight = into - width;
        const beforeLeft = 100 - into;
        next.hue = beyondRight <= beforeLeft
          ? hueOf(next.hue, left, width + beyondRight + PICK.margin)
          : hueOf(next.hue, left - beforeLeft - PICK.margin, width + beforeLeft + PICK.margin);
      }
    }
    for (const name of linearOf(next)) {
      const range = next[name];
      const [low, high] = [value(range.low), value(range.high)];
      if (each[name] < low) next[name] = rangeOf(range, each[name] - PICK.margin, high);
      else if (each[name] > high) next[name] = rangeOf(range, low, each[name] + PICK.margin);
    }
  }
  return next;
}

/** One way to leave a colour out of a qualifier: its cost (how far a range moves) and the change. */
interface Way { cost: number; apply: (q: Qualifier) => Qualifier }

/** Where a hue lies from the left end of a range of hue: within it or its softness (`at`, negative left of it), or beyond. */
function hueWithin(hue: HueRange, h: number): { at: number } | null {
  const { left, width } = edgesOf(hue);
  if (width >= 100) return { at: 0 };
  const soft = softsOf(hue);
  const into = forward(left, h);
  if (into <= width + soft.right) return { at: into };
  if (100 - into <= soft.left) return { at: into - 100 };
  return null;
}

/** Whether a colour is keyed at all: within each component that limits the qualifier, softness and all. */
function keyed(q: Qualifier, m: Measured): boolean {
  if (usesHue(q) && q.hue.on && m.sat >= PICK.grey && hueWithin(q.hue, m.hue) === null) return false;
  return linearOf(q).every((name) => {
    const range = q[name];
    return !range.on || (m[name] >= value(range.low) - value(range.low_soft) && m[name] <= value(range.high) + value(range.high_soft));
  });
}

/**
 * Subtract Color Range (p. 3166): each colour taken left out of the key, by moving the one edge,
 * of all the components that limit it, that has least to move to leave it beyond its softness.
 */
export function subtractRange(samples: readonly Rgb[], base: Qualifier): Qualifier {
  let next: Qualifier = { ...base };
  for (const each of samples.map(rgb => measure(rgb))) {
    if (!keyed(next, each)) continue;
    const ways: Way[] = [];
    if (usesHue(next) && next.hue.on && each.sat >= PICK.grey) {
      const { left, width } = edgesOf(next.hue);
      const soft = softsOf(next.hue);
      if (width >= 100) {
        // A range of every hue: all but the hues about the colour.
        const kept = 100 - soft.left - soft.right - 2 * PICK.margin;
        if (kept > 0) ways.push({ cost: 100 - kept, apply: q => ({ ...q, hue: hueOf(q.hue, each.hue + soft.left + PICK.margin, kept) }) });
      } else {
        const { at } = hueWithin(next.hue, each.hue)!;
        // The left edge moved past it, or the right edge back before it.
        const leftBy = at + soft.left + PICK.margin;
        const rightBy = width - at + soft.right + PICK.margin;
        if (leftBy < width) ways.push({ cost: leftBy, apply: q => ({ ...q, hue: hueOf(q.hue, left + leftBy, width - leftBy) }) });
        if (rightBy < width) ways.push({ cost: rightBy, apply: q => ({ ...q, hue: hueOf(q.hue, left, width - rightBy) }) });
      }
    }
    for (const name of linearOf(next)) {
      const range = next[name];
      if (!range.on) continue;
      const [low, high, lowSoft, highSoft] = [value(range.low), value(range.high), value(range.low_soft), value(range.high_soft)];
      const v = each[name];
      const raised = v + lowSoft + PICK.margin;
      const lowered = v - highSoft - PICK.margin;
      if (raised <= high) ways.push({ cost: raised - low, apply: q => ({ ...q, [name]: rangeOf(q[name], raised, value(q[name].high)) }) });
      if (lowered >= low) ways.push({ cost: high - lowered, apply: q => ({ ...q, [name]: rangeOf(q[name], value(q[name].low), lowered) }) });
    }
    const best = ways.sort((x, y) => x.cost - y.cost)[0];
    if (best) next = best.apply(next);
  }
  return next;
}

/**
 * Add Softness and Subtract Softness (pp. 3166, 3168): where a colour lies beyond the core of a
 * component, the softness on its side made to reach it (and a little more), or brought in short
 * of it, so that it is keyed in part, or not at all.
 */
export function softenRange(samples: readonly Rgb[], base: Qualifier, more: boolean): Qualifier {
  const next: Qualifier = { ...base };
  for (const each of samples.map(rgb => measure(rgb))) {
    if (usesHue(next) && each.sat >= PICK.grey && value(next.hue.width) < 100) {
      const hue = next.hue;
      const { left, width } = edgesOf(hue);
      const into = forward(left, each.hue);
      if (into > width) {
        const soft = softsOf(hue);
        const symmetry = value(hue.symmetry) / 50;
        const [right, before] = [into - width, 100 - into];
        const onRight = right <= before;
        const reach = onRight ? right : before;
        const now = onRight ? soft.right : soft.left;
        const wanted = more ? Math.max(now, reach + PICK.margin) : Math.min(now, Math.max(0, reach - PICK.margin));
        // Hue Soft is shared by Symmetry: the side's softness is soft · symmetry (right) or soft · (2 − symmetry) (left).
        const share = onRight ? symmetry : 2 - symmetry;
        if (share > 0) next.hue = { ...hue, soft: round(clamp(wanted / share, 0, 100)) };
      }
    }
    for (const name of linearOf(next)) {
      const range = next[name];
      const [low, high] = [value(range.low), value(range.high)];
      const v = each[name];
      if (v >= low && v <= high) continue;
      const below = v < low;
      const reach = below ? low - v : v - high;
      const now = value(below ? range.low_soft : range.high_soft);
      const wanted = more ? Math.max(now, reach + PICK.margin) : Math.min(now, Math.max(0, reach - PICK.margin));
      next[name] = { ...range, [below ? 'low_soft' : 'high_soft']: round(clamp(wanted, 0, 100)) };
    }
  }
  return next;
}

/** What a tool makes of the colours taken; Alt, held as the drag begins, makes Add and the Eyedropper subtract (§8.1). */
export function qualifierAfter(tool: QualifierTool, samples: readonly Rgb[], base: Qualifier | null | undefined, alt = false): Qualifier {
  const q = base ?? newQualifier();
  if (alt && (tool === 'pick' || tool === 'add')) return subtractRange(samples, q);
  switch (tool) {
    case 'pick': return pickRange(samples, base);
    case 'add': return addRange(samples, q);
    case 'subtract': return subtractRange(samples, q);
    case 'soften': return softenRange(samples, q, true);
    case 'harden': return softenRange(samples, q, false);
  }
}
