import { useContext, useRef, type PointerEvent } from 'react';
import { Pipette, RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import {
  BAR_FEEL, BOUNDS, SETTING_FEEL, WHEEL_FEEL, chromaOf, fit, rgbOfChroma, ringChroma, withoutColor, type Bounds,
} from './corrector';
import { Bar, MasterWheel, NumberControl, useDrag } from './controls';
import { PickerContext, useArmedPicker } from './pickers';
import { blackPointEdits, whitePointEdits } from './pickMaths';
import type { Edits, NodeEdit } from './useNodeEdit';

/**
 * The Primaries palette (Reference Manual 20, pp. 3058–3076; the interaction reference, §3): its
 * three modes, Color Wheels, Color Bars and Log Wheels, which Alt+Z switches between Wheels and
 * Log; the four wheels or bars; the two rows of settings that the modes share; Reset at its upper
 * right; the A button (Auto Color) at its lower left, and the black and white point pickers at the
 * upper left of Lift and Gain.
 */
export type PrimariesMode = 'wheels' | 'bars' | 'log';
export const PRIMARIES_MODES: readonly PrimariesMode[] = ['wheels', 'bars', 'log'];

type Channel = 'y' | 'r' | 'g' | 'b';
type WheelName = 'lift' | 'gamma' | 'gain' | 'offset' | 'shadow' | 'midtone' | 'highlight';
interface WheelSpec {
  name: WheelName;
  /** Where its numbers are in the params. */
  path: string;
  /** Lift, Gamma and Gain have a Y; Offset and the Log wheels R, G and B only (p. 3067). */
  channels: readonly Channel[];
  bounds: Bounds;
  feel: { reach: number; master: number; field: number };
}
const YRGB: readonly Channel[] = ['y', 'r', 'g', 'b'];
const RGB: readonly Channel[] = ['r', 'g', 'b'];
const WHEELS: Record<WheelName, WheelSpec> = {
  lift: { name: 'lift', path: 'primaries.lift', channels: YRGB, bounds: BOUNDS.lift, feel: WHEEL_FEEL.lift },
  gamma: { name: 'gamma', path: 'primaries.gamma', channels: YRGB, bounds: BOUNDS.gamma, feel: WHEEL_FEEL.gamma },
  gain: { name: 'gain', path: 'primaries.gain', channels: YRGB, bounds: BOUNDS.gain, feel: WHEEL_FEEL.gain },
  offset: { name: 'offset', path: 'primaries.offset', channels: RGB, bounds: BOUNDS.offset, feel: WHEEL_FEEL.offset },
  shadow: { name: 'shadow', path: 'log.shadow', channels: RGB, bounds: BOUNDS.log_wheel, feel: WHEEL_FEEL.log },
  midtone: { name: 'midtone', path: 'log.midtone', channels: RGB, bounds: BOUNDS.log_wheel, feel: WHEEL_FEEL.log },
  highlight: { name: 'highlight', path: 'log.highlight', channels: RGB, bounds: BOUNDS.log_wheel, feel: WHEEL_FEEL.log },
};
const LETTER: Record<Channel, string> = { y: 'Y', r: 'R', g: 'G', b: 'B' };

/** The values of a wheel's numbers at the playhead, Y first where it has one. */
const valuesOf = (edit: NodeEdit, wheel: WheelSpec) => Object.fromEntries(wheel.channels.map(ch => [ch, edit.value(`${wheel.path}.${ch}`)])) as Record<Channel, number>;

/**
 * The edits of a master wheel turned by `dx` pixels (p. 3062): all of its numbers move together,
 * Y, R, G and B, "to reflect the simultaneous adjustment you're making to all channels". The
 * master of Lift, Gamma and Gain writes Y with R, G and B; Lum Mix weighs the Y in (p. 3064).
 */
export function masterEdits(wheel: Pick<WheelSpec, 'path' | 'channels' | 'bounds' | 'feel'>, start: Record<string, number>, dx: number): Edits {
  return wheel.channels.map(ch => [`${wheel.path}.${ch}`, fit(start[ch] + dx * wheel.feel.master, wheel.bounds)] as const);
}

/**
 * The edits of a ring's colour balance indicator moved by `cb` and `cr` (p. 3061): R, G and B move
 * apart, the luma kept; Y does not move.
 */
export function balanceEdits(wheel: Pick<WheelSpec, 'path' | 'bounds'>, start: Record<string, number>, cb: number, cr: number): Edits {
  const [dr, dg, db] = rgbOfChroma(cb, cr);
  return [[`${wheel.path}.r`, fit(start.r + dr, wheel.bounds)], [`${wheel.path}.g`, fit(start.g + dg, wheel.bounds)], [`${wheel.path}.b`, fit(start.b + db, wheel.bounds)]];
}

/** Where the indicator of a ring is, as a share of its radius from its centre, right and up. */
export function indicatorOf(wheel: Pick<WheelSpec, 'bounds' | 'feel'>, values: Record<string, number>): [number, number] {
  const n = wheel.bounds.neutral;
  const [cb, cr] = chromaOf(values.r - n, values.g - n, values.b - n);
  const edge = ringChroma(wheel.feel.reach);
  let [x, y] = [cb / edge, cr / edge];
  const out = Math.hypot(x, y);
  if (out > 1) { x /= out; y /= out; }
  return [x, y];
}

/**
 * Pick Black Point and Pick White Point (pp. 3044–3045): pressed, the next click on the viewer
 * takes the colour that goes into the node there and makes it black by Lift, or white by Gain, one
 * step to undo; then the picker is let go (the interaction reference, §3.5). Escape lets it go.
 */
function PointPicker({ which, edit, name }: { which: 'black' | 'white'; edit: NodeEdit; name: string }) {
  const words = useWords();
  const hub = useContext(PickerContext);
  const now = useRef(edit);
  now.current = edit;
  const picker = useArmedPicker((pointer) => {
    const sample = hub?.sample;
    if (!sample || pointer.x === null || pointer.y === null) return;
    void sample(pointer.x, pointer.y).then((input) => {
      const editing = now.current;
      if (!input || !editing.enabled) return;
      const of = (path: string) => Object.fromEntries((['y', 'r', 'g', 'b'] as const).map(ch => [ch, editing.value(`${path}.${ch}`)])) as Record<'y' | 'r' | 'g' | 'b', number>;
      editing.commit(which === 'black' ? blackPointEdits(input, of('primaries.lift'))
        : whitePointEdits(input, of('primaries.lift'), of('primaries.gamma'), of('primaries.gain')));
    }, failure => hub?.onFailure(failure));
  });
  const usable = picker.available && hub?.sample != null && edit.enabled;
  return <button type="button" className="icon-btn" disabled={!usable} aria-pressed={picker.armed} aria-label={name}
    title={`${name}: ${words.hint.studio.colorPage.picker}`} onClick={picker.toggle} data-test={`picker-${which}`}><Pipette size={12} /></button>;
}

/**
 * The head of a wheel or of a group of bars: its name, its reset button at its upper right, and at
 * the upper left of Lift and Gain the black and white point pickers (pp. 3044–3045).
 */
function WheelHead({ title, picker, disabled, onReset, edit }: { title: string; picker?: 'black' | 'white'; disabled: boolean; onReset: () => void; edit: NodeEdit }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.primariesPalette;
  const pickerName = picker === 'black' ? labels.blackPoint : picker === 'white' ? labels.whitePoint : null;
  return <div className="color-wheel-head">
    {pickerName !== null && picker
      ? <PointPicker which={picker} edit={edit} name={pickerName} />
      : <span className="color-wheel-spacer" />}
    <span className="color-wheel-title">{title}</span>
    <button type="button" className="icon-btn" aria-label={`${labels.resetWheel}: ${title}`} title={labels.resetWheel} disabled={disabled}
      onClick={onReset} data-test="reset-wheel"><RotateCcw size={12} /></button>
  </div>;
}

/**
 * A wheel: its colour ring with the colour balance indicator, its master wheel beneath, its
 * numbers and the labels beneath them (pp. 3061–3064):
 *
 * - dragged anywhere in the ring, the indicator moves by as much as the pointer, as a trackball
 *   does; with Shift it jumps to the pointer and follows it; with Ctrl the drag is the master wheel's;
 * - a double click in the ring takes the colour away and leaves the master;
 * - the button at its upper right resets the colour and the master, as does a double click on the
 *   labels beneath the numbers;
 * - the master wheel dragged left darkens, right lightens; a double click on it does nothing (the
 *   manual does not say), and neither does the mouse wheel.
 */
function Wheel({ wheel, edit, title, picker }: { wheel: WheelSpec; edit: NodeEdit; title: string; picker?: 'black' | 'white' }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.primariesPalette;
  const ring = useRef<HTMLDivElement>(null);
  const gesture = useRef<{ start: Record<Channel, number>; master: boolean } | null>(null);
  const lastEdits = useRef<Edits | null>(null);
  const drag = useDrag(() => { gesture.current = null; lastEdits.current = null; edit.cancel(); },
    () => edit.controlKey((gesture.current?.master ? wheel.channels : RGB).map(ch => `${wheel.path}.${ch}`)),
    () => { gesture.current = null; lastEdits.current = null; });
  const values = valuesOf(edit, wheel);
  const disabled = !edit.enabled;
  const neutral = wheel.bounds.neutral;
  const [x, y] = indicatorOf(wheel, values);
  const master = wheel.channels.includes('y') ? values.y : withoutColor(values.r, values.g, values.b);
  const set = (edits: Edits, end: boolean) => (end ? edit.commit(edits) : edit.preview(edits));
  const fromPointer = (event: PointerEvent<HTMLElement>): [number, number] => {
    const box = (ring.current ?? event.currentTarget).getBoundingClientRect();
    const radius = Math.max(1, Math.min(box.width, box.height) / 2);
    return [(event.clientX - (box.left + box.width / 2)) / radius, -(event.clientY - (box.top + box.height / 2)) / radius];
  };
  const jump = (event: PointerEvent<HTMLElement>, start: Record<Channel, number>): Edits => {
    // Shift: the indicator to the pointer (p. 3061).
    let [px, py] = fromPointer(event);
    const out = Math.hypot(px, py);
    if (out > 1) { px /= out; py /= out; }
    const edge = ringChroma(wheel.feel.reach);
    const [cb, cr] = chromaOf(start.r - neutral, start.g - neutral, start.b - neutral);
    return balanceEdits(wheel, start, px * edge - cb, py * edge - cr);
  };
  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    const start = valuesOf(edit, wheel);
    gesture.current = { start, master: event.ctrlKey };
    drag.down(event, 0);
    if (event.shiftKey && !event.ctrlKey) {
      const edits = jump(event, start);
      drag.last = 1;
      set(edits, false);
      lastEdits.current = edits;
    } else lastEdits.current = null;
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const moved = drag.move(event);
    const now = gesture.current;
    if (!moved || !now) return;
    let edits: Edits;
    if (now.master) edits = masterEdits(wheel, now.start, moved.dx);
    else if (event.shiftKey) edits = jump(event, now.start);
    else {
      const box = (ring.current ?? event.currentTarget).getBoundingClientRect();
      const radius = Math.max(1, Math.min(box.width, box.height) / 2);
      const edge = ringChroma(wheel.feel.reach);
      edits = balanceEdits(wheel, now.start, (moved.dx / radius) * WHEEL_FEEL.follow * edge, (-moved.dy / radius) * WHEEL_FEEL.follow * edge);
    }
    lastEdits.current = edits;
    set(edits, false);
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    const moved = drag.up(event);
    if (moved === null) return;
    const edits = lastEdits.current;
    gesture.current = null; lastEdits.current = null;
    // A Shift click without a drag still jumps, and is sent.
    if ((moved || event.shiftKey) && edits) set(edits, true);
  };
  const resetColor = () => {
    if (disabled) return;
    const now = valuesOf(edit, wheel);
    const grey = fit(withoutColor(now.r, now.g, now.b), wheel.bounds);
    edit.commit(RGB.map(ch => [`${wheel.path}.${ch}`, grey] as const));
  };
  const resetAll = () => { if (!disabled) edit.commit(wheel.channels.map(ch => [`${wheel.path}.${ch}`, neutral] as const)); };
  const masterStart = useRef<Record<Channel, number> | null>(null);
  const keyed = wheel.channels.some(ch => edit.keyed(`${wheel.path}.${ch}`));
  return <div className="color-wheel" data-wheel={wheel.name} data-changed={wheel.channels.some(ch => values[ch] !== neutral) || undefined}>
    <WheelHead title={title} picker={picker} disabled={disabled} onReset={resetAll} edit={edit} />
    <div ref={ring} className="color-ring" role="slider" aria-label={`${labels.balance}: ${title}`} aria-valuetext={`${x.toFixed(2)}, ${y.toFixed(2)}`}
      aria-disabled={disabled || undefined} data-test="ring"
      onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => drag.cancel()}
      onDoubleClick={(event) => { event.preventDefault(); resetColor(); }}>
      <span className="color-ring-indicator" data-test="indicator" style={{ left: `${50 + x * 50}%`, top: `${50 - y * 50}%` }} />
    </div>
    <MasterWheel name={`${labels.master}: ${title}`} turned={(master - neutral) / wheel.feel.master} disabled={disabled}
      identity={edit.controlKey(wheel.channels.map(ch => `${wheel.path}.${ch}`))}
      onDrag={(dx) => {
        masterStart.current ??= valuesOf(edit, wheel);
        edit.preview(masterEdits(wheel, masterStart.current, dx));
      }}
      onEnd={(dx) => {
        const start = masterStart.current ?? valuesOf(edit, wheel);
        masterStart.current = null;
        edit.commit(masterEdits(wheel, start, dx));
      }}
      onInvalidate={() => { masterStart.current = null; }}
      onCancel={() => { masterStart.current = null; edit.cancel(); }} />
    <div className="color-wheel-numbers">
      {wheel.channels.map(ch => <NumberControl key={edit.controlKey([`${wheel.path}.${ch}`])} bare name={`${title} ${LETTER[ch]}`} field={`${wheel.path}.${ch}`} value={values[ch]} neutral={neutral}
        low={wheel.bounds.low} high={wheel.bounds.high} digits={2} perPx={wheel.feel.field} keyed={edit.keyed(`${wheel.path}.${ch}`)}
        keyedTitle={words.hint.studio.colorPage.keyed} disabled={disabled}
        onPreview={value => edit.preview([[`${wheel.path}.${ch}`, value]])} onCommit={value => edit.commit([[`${wheel.path}.${ch}`, value]])}
        onCancel={edit.cancel} />)}
    </div>
    <div className="color-wheel-letters" data-keyed={keyed || undefined} onDoubleClick={(event) => { event.preventDefault(); resetAll(); }} data-test="wheel-letters">
      {wheel.channels.map(ch => <span key={ch} data-channel={ch}>{LETTER[ch]}</span>)}
    </div>
  </div>;
}

/** The Bars mode's group of one wheel: an upright slider for each of its numbers, and the number beneath each (p. 3068). */
function Bars({ wheel, edit, title, picker }: { wheel: WheelSpec; edit: NodeEdit; title: string; picker?: 'black' | 'white' }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.primariesPalette;
  const values = valuesOf(edit, wheel);
  const neutral = wheel.bounds.neutral;
  const feel = BAR_FEEL[wheel.name as keyof typeof BAR_FEEL];
  const disabled = !edit.enabled;
  const masterStart = useRef<Record<Channel, number> | null>(null);
  const master = wheel.channels.includes('y') ? values.y : withoutColor(values.r, values.g, values.b);
  const resetAll = () => { if (!disabled) edit.commit(wheel.channels.map(ch => [`${wheel.path}.${ch}`, neutral] as const)); };
  return <div className="color-bars" data-wheel={wheel.name}>
    <WheelHead title={title} picker={picker} disabled={disabled} onReset={resetAll} edit={edit} />
    <div className="color-bars-row">
      {wheel.channels.map(ch => <div key={edit.controlKey([`${wheel.path}.${ch}`])} className="color-bar-column">
        <Bar name={`${title} ${LETTER[ch]}`} channel={ch} value={values[ch]} neutral={neutral} low={feel.low} high={feel.high} perPx={feel.perPx} disabled={disabled}
          onPreview={value => edit.preview([[`${wheel.path}.${ch}`, fit(value, wheel.bounds)]])}
          onCommit={value => edit.commit([[`${wheel.path}.${ch}`, fit(value, wheel.bounds)]])} onCancel={edit.cancel} />
        <NumberControl bare name={`${title} ${LETTER[ch]}`} field={`${wheel.path}.${ch}`} value={values[ch]} neutral={neutral}
          low={wheel.bounds.low} high={wheel.bounds.high} digits={2} perPx={wheel.feel.field} keyed={edit.keyed(`${wheel.path}.${ch}`)}
          keyedTitle={words.hint.studio.colorPage.keyed} disabled={disabled}
          onPreview={value => edit.preview([[`${wheel.path}.${ch}`, value]])} onCommit={value => edit.commit([[`${wheel.path}.${ch}`, value]])}
          onCancel={edit.cancel} />
        <span className="color-bar-letter" data-channel={ch} onDoubleClick={(event) => { event.preventDefault(); resetAll(); }}>{LETTER[ch]}</span>
      </div>)}
    </div>
    {wheel.name === 'offset' && <MasterWheel name={`${labels.master}: ${title}`} turned={(master - neutral) / wheel.feel.master} disabled={disabled}
      identity={edit.controlKey(wheel.channels.map(ch => `${wheel.path}.${ch}`))}
      onDrag={(dx) => { masterStart.current ??= valuesOf(edit, wheel); edit.preview(masterEdits(wheel, masterStart.current, dx)); }}
      onEnd={(dx) => { const start = masterStart.current ?? valuesOf(edit, wheel); masterStart.current = null; edit.commit(masterEdits(wheel, start, dx)); }}
      onInvalidate={() => { masterStart.current = null; }}
      onCancel={() => { masterStart.current = null; edit.cancel(); }} />}
  </div>;
}

type Setting = 'temperature' | 'tint' | 'contrast' | 'pivot' | 'midtone_detail' | 'color_boost' | 'shadows' | 'highlights'
  | 'saturation' | 'hue' | 'lum_mix' | 'low_range' | 'high_range';
const TOP: readonly Setting[] = ['temperature', 'tint', 'contrast', 'pivot', 'midtone_detail'];
const TOP_LOG: readonly Setting[] = ['low_range', 'high_range', 'temperature', 'tint', 'contrast', 'pivot', 'midtone_detail'];
const BOTTOM: readonly Setting[] = ['color_boost', 'shadows', 'highlights', 'saturation', 'hue', 'lum_mix'];
const pathOf = (setting: Setting) => (setting === 'low_range' || setting === 'high_range' ? `log.${setting}` : `primaries.${setting}`);

/** A row of the settings that the three modes share (pp. 3062–3064): each a name and a number to drag. */
function Settings({ settings, edit, row }: { settings: readonly Setting[]; edit: NodeEdit; row: 'top' | 'bottom' }) {
  const words = useWords();
  const names = words.label.studio.colorPage.primariesPalette.settings;
  return <div className="color-settings" data-row={row}>
    {settings.map((setting) => {
      const path = pathOf(setting);
      return <NumberControl key={edit.controlKey([path])} name={names[setting]} field={path} value={edit.value(path)} neutral={BOUNDS[setting].neutral}
        low={BOUNDS[setting].low} high={BOUNDS[setting].high} digits={SETTING_FEEL[setting].digits} perPx={SETTING_FEEL[setting].perPx}
        keyed={edit.keyed(path)} keyedTitle={words.hint.studio.colorPage.keyed} disabled={!edit.enabled}
        onPreview={value => edit.preview([[path, value]])} onCommit={value => edit.commit([[path, value]])} onCancel={edit.cancel} />;
    })}
  </div>;
}

/** Every setting of the palette at its neutral value: its Reset (p. 3064), in every mode. */
export function resetPrimaries(): Edits {
  const edits: [string, number][] = [];
  for (const wheel of Object.values(WHEELS)) for (const ch of wheel.channels) edits.push([`${wheel.path}.${ch}`, wheel.bounds.neutral]);
  for (const setting of [...TOP_LOG, ...BOTTOM]) edits.push([pathOf(setting), BOUNDS[setting].neutral]);
  return edits;
}

/** The pickers at the upper left of Lift and Gain, in the Wheels and Bars modes; not in Log (the manual does not say; we take not). */
const pickerOf = (name: WheelName): 'black' | 'white' | undefined => (name === 'lift' ? 'black' : name === 'gain' ? 'white' : undefined);

export function PrimariesPalette({ edit, mode, onMode, onAuto, status }: {
  edit: NodeEdit; mode: PrimariesMode; onMode: (mode: PrimariesMode) => void;
  /** Auto Color, the A button (pp. 3045–3046). */
  onAuto: () => void;
  /** What Auto Color has to say, where it could not do anything. */
  status: string | null;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage.primariesPalette;
  const names = labels.wheel;
  const wheels: readonly WheelName[] = mode === 'log' ? ['shadow', 'midtone', 'highlight', 'offset'] : ['lift', 'gamma', 'gain', 'offset'];
  return <div className="color-primaries" data-mode={mode} data-test="primaries">
    <div className="color-palette-head">
      <span className="color-gap" />
      <span className="color-modes" role="radiogroup" aria-label={labels.modes}>
        {PRIMARIES_MODES.map(each => <button key={each} type="button" role="radio" aria-checked={mode === each} className="color-mode"
          onClick={() => onMode(each)} data-mode={each}>{labels[each]}</button>)}
      </span>
      <button type="button" className="btn color-reset" disabled={!edit.enabled} onClick={() => edit.commit(resetPrimaries())} data-test="reset-palette">
        <RotateCcw size={12} aria-hidden="true" />{labels.reset}</button>
    </div>
    <Settings settings={mode === 'log' ? TOP_LOG : TOP} edit={edit} row="top" />
    <div className="color-wheels" data-test="wheels">
      {wheels.map(name => (mode === 'bars'
        ? <Bars key={name} wheel={WHEELS[name]} edit={edit} title={names[name]} picker={pickerOf(name)} />
        : <Wheel key={name} wheel={WHEELS[name]} edit={edit} title={names[name]} picker={mode === 'wheels' ? pickerOf(name) : undefined} />))}
    </div>
    <div className="color-palette-foot">
      <button type="button" className="btn color-auto" disabled={!edit.enabled || edit.pending()} aria-label={labels.auto} title={`${labels.auto} (Alt+Shift+C)`}
        onClick={onAuto} data-test="auto-color">A</button>
      <Settings settings={BOTTOM} edit={edit} row="bottom" />
    </div>
    {status !== null && <p className="caption color-status" role="status">{status}</p>}
  </div>;
}
