import { useContext, useRef, useState, type PointerEvent, type ReactNode } from 'react';
import { Minus, Pipette, Plus, RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import { newQualifier, type Morph, type Param, type Qualifier, type QualifierMode, type ViewPointer } from '../engine';
import { shownValue } from './corrector';
import { NumberControl, useDrag } from './controls';
import { PickerContext, usePalettePicker, usePickerDrags, type Rgb } from './pickers';
import { qualifierAfter, type QualifierTool } from './qualifierPick';
import type { EditValue, Edits, NodeEdit } from './useNodeEdit';
import './qualifier.css';

/**
 * The Qualifier palette (Reference Manual 20, pp. 3157–3184; the interaction reference, §8.1): the
 * key of the picture that comes into the current node, by hue, saturation and luma (HSL, the
 * default), by red, green and blue (RGB), or by luma alone (LUM, p. 3175); 3D is not done.
 *
 * - Its head: the Selection Range tools (p. 3166), the Sample Eyedropper, Subtract and Add Color
 *   Range, Subtract and Add Softness, one chosen until another is; Invert (p. 3176); the modes;
 *   Reset (the manual names it without its place, p. 3176; it takes the qualifier away, as the
 *   node was before a colour was picked).
 * - On the viewer, while the palette is open, its on-screen control is Qualifier (p. 2988): a click
 *   takes the colour of a pixel, a drag the colours it goes over, by the tool chosen; Alt held as
 *   the drag begins takes away instead (the manual says so of the 3D qualifier; the interaction
 *   reference takes it for these too). What the drag makes is shown as it goes and is one step to
 *   undo.
 * - Each component: its switch (pp. 3173, 3175), a bar of its range whose edges are dragged (with
 *   Alt, its softness) and whose middle moves it (p. 3172), and its numbers.
 * - Matte Finesse (pp. 3178–3181) beneath.
 */
const MODES: readonly QualifierMode[] = ['hsl', 'rgb', 'lum'];
const TOOLS: readonly QualifierTool[] = ['pick', 'subtract', 'add', 'harden', 'soften'];
type Linear = 'sat' | 'lum' | 'red' | 'green' | 'blue';
const MORPHS: readonly Morph[] = ['shrink', 'grow', 'opening', 'closing'];

/** The qualifier at the playhead, each setting the value its control shows; a new one where the node has none. */
export function qualifierAt(q: Qualifier | null | undefined, frame: number): Qualifier {
  const at = (param: Param) => shownValue(param, frame);
  const base = q ?? newQualifier();
  const range = (r: Qualifier['sat']) => ({ ...r, low: at(r.low), high: at(r.high), low_soft: at(r.low_soft), high_soft: at(r.high_soft) });
  const f = base.finesse;
  return {
    ...base,
    hue: { ...base.hue, center: at(base.hue.center), width: at(base.hue.width), soft: at(base.hue.soft), symmetry: at(base.hue.symmetry) },
    sat: range(base.sat), lum: range(base.lum), red: range(base.red), green: range(base.green), blue: range(base.blue),
    finesse: {
      ...f, pre_filter: at(f.pre_filter), clean_black: at(f.clean_black), black_clip: at(f.black_clip), clean_white: at(f.clean_white),
      white_clip: at(f.white_clip), blur_radius: at(f.blur_radius), in_out_ratio: at(f.in_out_ratio), morph_radius: at(f.morph_radius),
    },
  };
}

/** The leaves of an object, by dotted path. */
function leaves(value: unknown, path = ''): [string, unknown][] {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return [[path, value]];
  return Object.entries(value).flatMap(([name, inner]) => leaves(inner, path ? `${path}.${name}` : name));
}

/**
 * The edits that make the node's qualifier `next` from `shown` (its values at the playhead): the
 * qualifier whole where the node has none (the engine takes a qualifier whole the first time, and
 * in part after); else each number that changed, into its keyframe where it has keys (p. 3391),
 * and each switch or choice whole.
 */
export function qualifierEdits(has: boolean, shown: Qualifier, next: Qualifier): Edits {
  if (!has) return [['qualifier', { whole: next }]];
  const before = new Map(leaves(shown));
  return leaves(next).filter(([path, value]) => before.get(path) !== value)
    .map(([path, value]): readonly [string, EditValue] => [`qualifier.${path}`, typeof value === 'number' ? value : { whole: value }]);
}

/** The gradient of a component's bar. */
const GRADIENTS: Record<'hue' | Linear, string> = {
  hue: 'linear-gradient(to right, #f00, #ff0 16.7%, #0f0 33.3%, #0ff 50%, #00f 66.7%, #f0f 83.3%, #f00)',
  sat: 'linear-gradient(to right, #888, #e33)',
  lum: 'linear-gradient(to right, #000, #fff)',
  red: 'linear-gradient(to right, #000, #f00)',
  green: 'linear-gradient(to right, #000, #0f0)',
  blue: 'linear-gradient(to right, #000, #00f)',
};
/** How near an edge of a bar a press takes the edge, in pixels. */
const EDGE_PX = 6;

/** What a drag of a bar changes, from what it began at. */
type BarChange = Partial<Record<'low' | 'high' | 'low_soft' | 'high_soft' | 'center' | 'width' | 'soft', number>>;

/**
 * The bar of a component (p. 3172): its range over a gradient of what it reads, softness shaded on
 * each side. Its edges dragged change Low and High (Width, of hue), with Alt its softness on that
 * side; its middle dragged moves it (Center, of hue). Shown as it goes, sent when let go, Escape
 * puts it back.
 */
function RangeBar({ kind, values, disabled, name, onPreview, onCommit, onCancel }: {
  kind: 'hue' | Linear;
  values: Record<string, number>;
  disabled: boolean;
  name: string;
  onPreview: (change: BarChange) => void; onCommit: (change: BarChange) => void; onCancel: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const grip = useRef<{ part: 'low' | 'high' | 'middle'; soft: boolean; start: Record<string, number> } | null>(null);
  const [live, setLive] = useState<BarChange | null>(null);
  const drag = useDrag(() => { grip.current = null; setLive(null); onCancel(); });
  const now: Record<string, number> = { ...values, ...live };
  const hue = kind === 'hue';
  const [left, right] = hue ? [now.center - now.width / 2, now.center + now.width / 2] : [now.low, now.high];
  const symmetry = (now.symmetry ?? 50) / 50;
  const [softLeft, softRight] = hue ? [now.soft * (2 - symmetry), now.soft * symmetry] : [now.low_soft, now.high_soft];
  const changeOf = (dx: number): BarChange => {
    const held = grip.current!;
    const s = held.start;
    if (hue) {
      if (held.part === 'middle') return { center: ((s.center + dx) % 100 + 100) % 100 };
      const outward = held.part === 'low' ? -dx : dx;
      if (held.soft) {
        const share = held.part === 'low' ? 2 - symmetry : symmetry;
        return share > 0 ? { soft: Math.max(0, Math.min(100, s.soft + outward / share)) } : {};
      }
      return { width: Math.max(0, Math.min(100, s.width + 2 * outward)) };
    }
    const clamp = (v: number) => Math.max(0, Math.min(100, v));
    if (held.part === 'middle') {
      const by = Math.max(-s.low, Math.min(100 - s.high, dx));
      return { low: s.low + by, high: s.high + by };
    }
    if (held.soft) return held.part === 'low' ? { low_soft: clamp(s.low_soft - dx) } : { high_soft: clamp(s.high_soft + dx) };
    return held.part === 'low' ? { low: Math.min(s.high, clamp(s.low + dx)) } : { high: Math.max(s.low, clamp(s.high + dx)) };
  };
  const round = (change: BarChange): BarChange => Object.fromEntries(Object.entries(change).map(([key, value]) => [key, Math.round(value * 100) / 100]));
  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    const rect = box.current!.getBoundingClientRect();
    const at = ((event.clientX - rect.left) / Math.max(1, rect.width)) * 100;
    const px = (share: number) => (share / 100) * rect.width;
    const near = (edge: number) => Math.abs(px(at - edge)) <= EDGE_PX || (hue && Math.abs(px(at - edge + 100)) <= EDGE_PX) || (hue && Math.abs(px(at - edge - 100)) <= EDGE_PX);
    const part = near(left) ? 'low' : near(right) ? 'high' : 'middle';
    grip.current = { part, soft: event.altKey && part !== 'middle', start: { ...values } };
    drag.down(event, 0);
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const moved = drag.move(event);
    if (!moved || !grip.current) return;
    const rect = box.current!.getBoundingClientRect();
    const change = round(changeOf((moved.dx / Math.max(1, rect.width)) * 100));
    setLive(change); onPreview(change);
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    const moved = drag.up(event);
    const change = live;
    grip.current = null; setLive(null);
    if (moved && change) onCommit(change);
  };
  // The core and its softness, drawn once for a range, and again a turn over for a range of hue that goes round.
  const segments = (offset: number): ReactNode => <>
    <span className="color-qualifier-soft" data-side="low" style={{ left: `${left - softLeft + offset}%`, width: `${softLeft}%` }} />
    <span className="color-qualifier-core" style={{ left: `${left + offset}%`, width: `${right - left}%` }} />
    <span className="color-qualifier-soft" data-side="high" style={{ left: `${right + offset}%`, width: `${softRight}%` }} />
  </>;
  return <div ref={box} className="color-qualifier-bar" role="slider" aria-label={name} aria-disabled={disabled || undefined}
    aria-valuetext={`${left.toFixed(1)}–${right.toFixed(1)}`} data-component={kind} data-test={`qualifier-bar-${kind}`}
    style={{ background: GRADIENTS[kind] }} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => drag.cancel()}>
    {segments(0)}
    {hue && segments(-100)}
    {hue && segments(100)}
  </div>;
}

/** A number of the qualifier, by its path within it. */
interface Field { path: string; name: string; low: number; high: number; neutral: number; digits?: number; perPx?: number }

export function QualifierPalette({ edit }: { edit: NodeEdit }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.qualifierPalette;
  const disabled = !edit.enabled;
  const has = edit.corrector.qualifier != null;
  const shown = qualifierAt(edit.corrector.qualifier, edit.frame);
  // What a drag of a bar changes, from the qualifier as the engine has it (not as the drag shows it).
  const hasBase = edit.base.qualifier != null;
  const committed = qualifierAt(edit.base.qualifier, edit.frame);
  const [tool, setTool] = useState<QualifierTool>('pick');
  const keyedTitle = words.hint.studio.colorPage.keyed;
  const set = (next: Qualifier, end: boolean) => {
    const edits = qualifierEdits(hasBase, committed, next);
    if (end) edit.commit(edits); else if (edits.length) edit.preview(edits);
  };
  /** One number changed. */
  const numberEdits = (path: string, value: number): Edits => {
    if (has) return [[`qualifier.${path}`, value]];
    const whole = qualifierAt(null, 0) as unknown as Record<string, unknown>;
    const names = path.split('.');
    let at = whole;
    for (const name of names.slice(0, -1)) at = at[name] as Record<string, unknown>;
    at[names[names.length - 1]] = value;
    return [['qualifier', { whole }]];
  };
  const wholeEdits = (path: string, value: unknown): Edits => {
    if (has) return [[`qualifier.${path}`, { whole: value }]];
    const next = qualifierAt(null, 0);
    const names = path.split('.');
    let at = next as unknown as Record<string, unknown>;
    for (const name of names.slice(0, -1)) at = at[name] as Record<string, unknown>;
    at[names[names.length - 1]] = value;
    return [['qualifier', { whole: next }]];
  };

  // The pointer on the picture: the colours taken go into the qualifier by the tool chosen.
  const base = useRef<{ shown: Qualifier; has: boolean } | null>(null);
  const hub = useContext(PickerContext);
  const drags = usePickerDrags(edit.enabled ? hub?.sample ?? null : null, (samples: readonly Rgb[], ended: boolean, alt: boolean) => {
    const from = base.current;
    if (!from) return;
    const next = qualifierAfter(tool, samples, from.has ? from.shown : null, alt);
    const edits = qualifierEdits(from.has, from.shown, next);
    if (ended) { base.current = null; edit.commit(edits); } else if (edits.length) edit.preview(edits);
  }, () => { base.current = null; edit.cancel(); }, failure => { base.current = null; edit.cancel(); hub?.onFailure(failure); });
  usePalettePicker(!disabled, (pointer: ViewPointer) => {
    if (pointer.kind === 'down' && pointer.button === 'left') base.current = { shown, has };
    drags(pointer);
  });

  const number = ({ path, name, low, high, neutral, digits = 2, perPx = 0.2 }: Field) => {
    const full = `qualifier.${path}`;
    const value = path.split('.').reduce<unknown>((at, part) => (at as Record<string, unknown>)[part], shown) as number;
    return <NumberControl key={edit.controlKey([full])} name={name} field={full} value={value} neutral={neutral} low={low} high={high}
      digits={digits} perPx={perPx} keyed={has && edit.keyed(full)} keyedTitle={keyedTitle} disabled={disabled}
      onPreview={next => edit.preview(numberEdits(path, next))} onCommit={next => edit.commit(numberEdits(path, next))} onCancel={edit.cancel} />;
  };
  const toggle = (path: string, on: boolean, name: string) => <button type="button" className="color-qualifier-switch" role="switch" aria-checked={on}
    aria-label={name} title={name} disabled={disabled} onClick={() => edit.commit(wholeEdits(path, !on))} data-test={`qualifier-on-${path.split('.')[0]}`} />;

  const hueRow = () => {
    const hue = shown.hue;
    const bar = (change: BarChange, end: boolean) => set({ ...committed, hue: { ...committed.hue, ...change } }, end);
    return <div className="color-qualifier-row" data-component="hue">
      {toggle('hue.on', hue.on, labels.components.hue)}
      <span className="color-qualifier-name">{labels.components.hue}</span>
      <RangeBar kind="hue" name={labels.components.hue} disabled={disabled || !hue.on}
        values={{ center: hue.center as number, width: hue.width as number, soft: hue.soft as number, symmetry: hue.symmetry as number }}
        onPreview={change => bar(change, false)} onCommit={change => bar(change, true)} onCancel={edit.cancel} />
      <span className="color-qualifier-numbers">
        {number({ path: 'hue.center', name: labels.center, low: 0, high: 100, neutral: 50 })}
        {number({ path: 'hue.width', name: labels.width, low: 0, high: 100, neutral: 100 })}
        {number({ path: 'hue.soft', name: labels.soft, low: 0, high: 100, neutral: 0 })}
        {number({ path: 'hue.symmetry', name: labels.symmetry, low: 0, high: 100, neutral: 50 })}
      </span>
    </div>;
  };
  const linearRow = (name: Linear) => {
    const range = shown[name];
    const bar = (change: BarChange, end: boolean) => set({ ...committed, [name]: { ...committed[name], ...change } }, end);
    return <div key={name} className="color-qualifier-row" data-component={name}>
      {toggle(`${name}.on`, range.on, labels.components[name])}
      <span className="color-qualifier-name">{labels.components[name]}</span>
      <RangeBar kind={name} name={labels.components[name]} disabled={disabled || !range.on}
        values={{ low: range.low as number, high: range.high as number, low_soft: range.low_soft as number, high_soft: range.high_soft as number }}
        onPreview={change => bar(change, false)} onCommit={change => bar(change, true)} onCancel={edit.cancel} />
      <span className="color-qualifier-numbers">
        {number({ path: `${name}.low`, name: labels.low, low: 0, high: 100, neutral: 0 })}
        {number({ path: `${name}.high`, name: labels.high, low: 0, high: 100, neutral: 100 })}
        {number({ path: `${name}.low_soft`, name: labels.lowSoft, low: 0, high: 100, neutral: 0 })}
        {number({ path: `${name}.high_soft`, name: labels.highSoft, low: 0, high: 100, neutral: 0 })}
      </span>
    </div>;
  };
  const rows = shown.mode === 'rgb' ? (['red', 'green', 'blue'] as const).map(linearRow)
    : shown.mode === 'lum' ? [linearRow('lum')] : [hueRow(), linearRow('sat'), linearRow('lum')];
  const finesse = labels.finesse;
  const icon = (each: QualifierTool) => (each === 'pick' ? <Pipette size={13} />
    : each === 'add' || each === 'soften' ? <Plus size={13} /> : <Minus size={13} />);

  return <div className="color-qualifier" data-mode={shown.mode} data-test="qualifier">
    <div className="color-palette-head">
      <span className="color-qualifier-tools" role="radiogroup" aria-label={labels.selectionRange}>
        {TOOLS.map(each => <button key={each} type="button" role="radio" aria-checked={tool === each} className="icon-btn color-qualifier-tool"
          data-tool={each} data-soft={each === 'soften' || each === 'harden' || undefined} aria-label={labels.tools[each]} title={labels.tools[each]}
          disabled={disabled} onClick={() => setTool(each)}>{icon(each)}</button>)}
      </span>
      <button type="button" className="color-tool" aria-pressed={shown.invert} disabled={disabled}
        onClick={() => edit.commit(wholeEdits('invert', !shown.invert))} data-test="qualifier-invert">{labels.invert}</button>
      <span className="color-gap" />
      <span className="color-modes" role="radiogroup" aria-label={labels.modes}>
        {MODES.map(each => <button key={each} type="button" role="radio" aria-checked={shown.mode === each} className="color-mode" data-mode={each}
          disabled={disabled} onClick={() => { if (each !== shown.mode) edit.commit(wholeEdits('mode', each)); }}>{labels[each]}</button>)}
      </span>
      <button type="button" className="btn color-reset" disabled={disabled || !has} onClick={() => edit.commit([['qualifier', { whole: null }]])}
        data-test="reset-palette"><RotateCcw size={12} aria-hidden="true" />{labels.reset}</button>
    </div>
    {!has && !disabled && <p className="caption color-qualifier-hint" data-test="qualifier-empty">{words.hint.studio.colorPage.qualifierEmpty}</p>}
    <div className="color-qualifier-rows">{rows}</div>
    <div className="color-qualifier-finesse" role="group" aria-label={finesse.title}>
      <span className="overline">{finesse.title}</span>
      <div className="color-settings">
        {number({ path: 'finesse.pre_filter', name: finesse.pre_filter, low: 0, high: 100, neutral: 0, digits: 1 })}
        {number({ path: 'finesse.clean_black', name: finesse.clean_black, low: 0, high: 100, neutral: 0, digits: 1 })}
        {number({ path: 'finesse.black_clip', name: finesse.black_clip, low: 0, high: 100, neutral: 0, digits: 1 })}
        {number({ path: 'finesse.clean_white', name: finesse.clean_white, low: 0, high: 100, neutral: 0, digits: 1 })}
        {number({ path: 'finesse.white_clip', name: finesse.white_clip, low: 0, high: 100, neutral: 100, digits: 1 })}
        {number({ path: 'finesse.blur_radius', name: finesse.blur_radius, low: 0, high: 2000, neutral: 0, digits: 1, perPx: 1 })}
        {number({ path: 'finesse.in_out_ratio', name: finesse.in_out_ratio, low: -100, high: 100, neutral: 0, digits: 1, perPx: 0.5 })}
        <label className="color-number">
          <span className="color-number-name">{finesse.morph}</span>
          <select className="field color-qualifier-morph" value={shown.finesse.morph} disabled={disabled} aria-label={finesse.morph}
            onChange={(event) => { edit.commit(wholeEdits('finesse.morph', event.target.value as Morph)); event.currentTarget.blur(); }}
            data-test="qualifier-morph">
            {MORPHS.map(each => <option key={each} value={each}>{finesse.morphs[each]}</option>)}
          </select>
        </label>
        {number({ path: 'finesse.morph_radius', name: finesse.morph_radius, low: 0, high: 100, neutral: 0, digits: 1 })}
      </div>
    </div>
  </div>;
}
