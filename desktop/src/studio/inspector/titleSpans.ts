/**
 * The runs of the text of a title that are drawn otherwise than its style says (`TitleSpan`): what
 * styling a part of the text leaves of them, and what an edit of the text leaves of them. Pure:
 * nothing here changes what it is given.
 *
 * A run is counted in Unicode scalar values, as the engine counts it, and not in the units of
 * UTF-16 that a field of text counts its selection in (`scalarAt`). The runs that come out are as
 * the engine takes them: in order, none over another, none empty and none past the end of the
 * text, each saying something (one that says nothing is left out), and two side by side that say
 * the same are one. A run says a colour, a share of the size, bold, italic or a family; what it
 * leaves out is as the style of the title says, so bold and italic are said only where they are
 * on, a share of 1 and an empty family not at all.
 */
import type { TitleSpan, TitleStyle } from '../engine';

/** The longest text of a title that the engine takes, in characters. */
export const LONGEST_TEXT = 1000;
/** The most runs that the engine takes in a title. */
export const MOST_SPANS = 256;
/** The least and the most share of the size of a title that a run may be drawn at. */
export const LEAST_SCALE = 0.1;
export const MOST_SCALE = 10;

/** What a run says of its characters. */
export type SpanLook = Omit<TitleSpan, 'from' | 'to'>;
/** A change of what runs say: a field given is set, one given as null is taken away, and one left out is kept. */
export type SpanPatch = { [K in keyof SpanLook]?: Exclude<SpanLook[K], undefined> | null };
/**
 * What the characters of a part of a text say in common. Bold and italic, whether every one of them is; the others, what every
 * one of them says, undefined where none says anything of it (the style of the title holds), or null where they differ.
 */
export interface LookOver {
  bold: boolean; italic: boolean; color: string | undefined | null; scale: number | undefined | null; font: string | undefined | null;
}

/** How many characters a text has, as the engine counts them: its Unicode scalar values. */
export function lengthOf(text: string): number {
  let count = 0;
  for (const _character of text) count += 1;
  return count;
}

/** A text cut to the longest that the engine takes. */
export const clipped = (text: string): string => (text.length <= LONGEST_TEXT ? text : [...text].slice(0, LONGEST_TEXT).join(''));

/**
 * The place, in Unicode scalar values, of a place in the units of UTF-16 of a text, as a field of text gives its selection: the
 * characters before it. A place within a character that takes two units is after it.
 */
export function scalarAt(text: string, unit: number): number {
  let count = 0;
  let at = 0;
  for (const character of text) {
    if (at >= unit) break;
    at += character.length;
    count += 1;
  }
  return count;
}

/** A share of the size kept within those that a run may have. */
const scaleIn = (scale: number) => Math.min(MOST_SCALE, Math.max(LEAST_SCALE, scale));

/** What a run says, as the engine is told it: bold and italic only where on, no share of 1 and no empty family. */
function lookOf(span: SpanLook): SpanLook {
  const look: SpanLook = {};
  if (typeof span.color === 'string' && span.color !== '') look.color = span.color;
  if (typeof span.scale === 'number' && Number.isFinite(span.scale) && scaleIn(span.scale) !== 1) look.scale = scaleIn(span.scale);
  if (span.bold === true) look.bold = true;
  if (span.italic === true) look.italic = true;
  if (typeof span.font === 'string' && span.font !== '') look.font = span.font;
  return look;
}

const saysNothing = (look: SpanLook) => look.color === undefined && look.scale === undefined && look.bold === undefined
  && look.italic === undefined && look.font === undefined;

/** Whether two runs say the same: both as `lookOf` has them. */
const sameLook = (a: SpanLook, b: SpanLook) => a.color === b.color && a.scale === b.scale && a.bold === b.bold && a.italic === b.italic && a.font === b.font;

/** What a run says with a change made to it. */
function patched(look: SpanLook, patch: SpanPatch): SpanLook {
  const next: Record<string, unknown> = { ...look };
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) delete next[key];
    else if (value !== undefined) next[key] = value;
  }
  return lookOf(next as SpanLook);
}

/**
 * Runs as the engine takes them, in a text of `length` characters: each within the text, in order, the empty ones and those that
 * say nothing left out, two side by side that say the same made one, and of two that would overlap the later cut to begin where
 * the earlier ends.
 */
export function tidied(spans: readonly TitleSpan[], length: number): TitleSpan[] {
  const within = (at: number) => Math.max(0, Math.min(Math.round(at), length));
  const sorted = spans
    .map(span => ({ from: within(span.from), to: within(span.to), look: lookOf(span) }))
    .filter(each => each.from < each.to && !saysNothing(each.look))
    .sort((a, b) => a.from - b.from || a.to - b.to);
  const runs: TitleSpan[] = [];
  for (const each of sorted) {
    const last = runs.length > 0 ? runs[runs.length - 1] : undefined;
    const from = last ? Math.max(each.from, last.to) : each.from;
    if (from >= each.to) continue;
    if (last && last.to === from && sameLook(last, each.look)) last.to = each.to;
    else runs.push({ from, to: each.to, ...each.look });
  }
  return runs;
}

/** The places where what a text says may change between `from` and `to`: those two, and the ends of the runs between them. */
function cutsOf(spans: readonly TitleSpan[], from: number, to: number): number[] {
  const inside = spans.flatMap(span => [span.from, span.to]).filter(at => at > from && at < to);
  return [...new Set([from, to, ...inside])].sort((a, b) => a - b);
}

/** What the characters from `from` to the one before `to` say: of the run that covers them, or nothing. */
const lookBetween = (spans: readonly TitleSpan[], from: number, to: number): SpanLook => {
  const over = spans.find(span => span.from <= from && to <= span.to);
  return over ? lookOf(over) : {};
};

/**
 * The runs of a text of `length` characters with a change made to what the characters from `from` to the one before `to` say,
 * as styling a selection of the text does: a run that goes past an end of the selection is cut there, the part within changed and
 * the part without kept; the characters of the selection that no run covered get one where the change says something of them.
 * Null where they would be more runs than the engine takes. A selection of nothing changes nothing.
 */
export function styled(spans: readonly TitleSpan[], from: number, to: number, patch: SpanPatch, length: number): TitleSpan[] | null {
  const runs = tidied(spans, length);
  const start = Math.max(0, Math.min(from, length));
  const end = Math.max(start, Math.min(to, length));
  if (start === end) return runs;
  const cuts = cutsOf(runs, 0, length);
  if (!cuts.includes(start)) cuts.push(start);
  if (!cuts.includes(end)) cuts.push(end);
  cuts.sort((a, b) => a - b);
  const pieces: TitleSpan[] = [];
  for (let at = 0; at + 1 < cuts.length; at += 1) {
    const [a, b] = [cuts[at], cuts[at + 1]];
    const before = lookBetween(runs, a, b);
    const look = a >= start && b <= end ? patched(before, patch) : before;
    if (!saysNothing(look)) pieces.push({ from: a, to: b, ...look });
  }
  const made = tidied(pieces, length);
  return made.length > MOST_SPANS ? null : made;
}

/** What the characters from `from` to the one before `to` say in common (`LookOver`); nothing of a selection of nothing. */
export function lookOver(spans: readonly TitleSpan[], from: number, to: number): LookOver {
  if (from >= to) return { bold: false, italic: false, color: undefined, scale: undefined, font: undefined };
  const cuts = cutsOf(spans, from, to);
  const looks: SpanLook[] = [];
  for (let at = 0; at + 1 < cuts.length; at += 1) looks.push(lookBetween(spans, cuts[at], cuts[at + 1]));
  const shared = <K extends 'color' | 'scale' | 'font'>(key: K): SpanLook[K] | null => {
    const first = looks[0][key];
    return looks.every(look => look[key] === first) ? first : null;
  };
  return {
    bold: looks.every(look => look.bold === true), italic: looks.every(look => look.italic === true),
    color: shared('color'), scale: shared('scale'), font: shared('font'),
  };
}

/**
 * The runs of a text as an edit of it leaves them, on the characters that they were on: the edit is the part between what the
 * two texts begin and end with alike, which was taken away and put in. A run after it moves with the characters after it, so
 * that what is put in before a run moves it; a run that the edit is within grows or shrinks by what was put in and taken away;
 * one that the edit cuts into keeps what is left of it; one that the edit took away whole is gone. What is put in where a run
 * begins or ends is not of the run.
 */
export function shifted(spans: readonly TitleSpan[], before: string, after: string): TitleSpan[] {
  const was = [...before];
  const now = [...after];
  let start = 0;
  while (start < was.length && start < now.length && was[start] === now[start]) start += 1;
  let tail = 0;
  while (tail < was.length - start && tail < now.length - start && was[was.length - 1 - tail] === now[now.length - 1 - tail]) tail += 1;
  // The characters from `start` to the one before `taken` were taken away, and those to the one before `put` put in their place.
  const taken = was.length - tail;
  const put = now.length - tail;
  const by = now.length - was.length;
  const head = (at: number) => (at < start ? at : at >= taken ? at + by : put);
  const end = (at: number) => (at <= start ? at : at >= taken ? at + by : start);
  return tidied(spans.map(span => ({ ...span, from: head(span.from), to: end(span.to) })), now.length);
}

/** The text of a title cut into the parts that it is drawn in: each with what its run says, a part that no run covers saying nothing. */
export function runsOf(text: string, spans: readonly TitleSpan[]): { text: string; look: SpanLook }[] {
  const characters = [...text];
  const parts: { text: string; look: SpanLook }[] = [];
  let at = 0;
  for (const run of tidied(spans, characters.length)) {
    if (run.from > at) parts.push({ text: characters.slice(at, run.from).join(''), look: {} });
    parts.push({ text: characters.slice(run.from, run.to).join(''), look: lookOf(run) });
    at = run.to;
  }
  if (at < characters.length) parts.push({ text: characters.slice(at).join(''), look: {} });
  return parts;
}

/** The style of a title with these runs: with none, it says nothing of them, as the engine writes it. */
export function withSpans(style: TitleStyle, spans: readonly TitleSpan[]): TitleStyle {
  const { spans: _before, ...rest } = style;
  return spans.length === 0 ? rest : { ...rest, spans: [...spans] };
}
