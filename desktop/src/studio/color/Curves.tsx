import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from 'react';
import { Link2, Pipette, RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { CurvePoint } from '../engine';
import {
  BOUNDS, HSL_CURVES, SETTING_FEEL, VECTORS, addPoint, addVector, curveAt, defaultAnchors, isHueCurve, movePoint, outRange, removePoint,
  type HslCurve, type Vector,
} from './corrector';
import { NumberControl } from './controls';
import type { Edits, NodeEdit } from './useNodeEdit';

/**
 * The Curves palette (Reference Manual 20, pp. 3108–3126; the interaction reference, §4): the
 * custom curves of Y, R, G and B, ganged or not, with their intensities and Soft Clip; and the six
 * HSL curves, with the six vector patches of the curves of hue and the Input and Output of the
 * point chosen. Its mode is chosen at its upper right, beside its Reset.
 *
 * On every curve: a click adds a point where it is, and the curve goes through it; Shift+click adds
 * one on the curve as it is; a point dragged changes the curve between its neighbours; Alt while a
 * point of a custom curve is dragged puts it on the neutral diagonal; a right click takes a point
 * away (not the black or white point of a custom curve). A point added and dragged on is one step to
 * undo.
 */
export type CurveMode = 'custom' | HslCurve;
export const CURVE_MODES: readonly CurveMode[] = ['custom', ...HSL_CURVES];
type Channel = 'y' | 'r' | 'g' | 'b';
const CHANNELS: readonly Channel[] = ['y', 'r', 'g', 'b'];
const LETTER: Record<Channel, string> = { y: 'Y', r: 'R', g: 'G', b: 'B' };

/** A curve that the editor draws: the one being edited takes the pointer. */
interface Drawn { id: string; points: readonly CurvePoint[]; active: boolean }

/** How many pixels from a point a press takes it rather than adding another. */
const HIT_PX = 6;
/** How many steps a curve is drawn in. */
const STEPS = 128;

/**
 * The editor of a curve: a press where there is no point adds one (with Shift, on the curve), and
 * drags it on; a press on a point drags it; a right click on one takes it away.
 */
function CurveEditor({ drawn, kind, name, pointName, disabled, chosen, onChoose, onPreview, onCommit, onCancel }: {
  drawn: readonly Drawn[];
  kind: 'custom' | 'flat' | 'round';
  name: string; pointName: string; disabled: boolean;
  /** The point chosen on the curve being edited, whose Input and Output an HSL curve shows. */
  chosen: number | null;
  onChoose: (index: number | null) => void;
  onPreview: (points: CurvePoint[]) => void;
  onCommit: (points: CurvePoint[]) => void;
  onCancel: () => void;
}) {
  const area = useRef<HTMLDivElement>(null);
  const custom = kind === 'custom';
  const [low, high] = outRange(custom);
  const active = drawn.find(each => each.active) ?? drawn[0];
  const gesture = useRef<{ pointer: number; index: number; points: CurvePoint[]; added: boolean; moved: boolean; x: number; y: number } | null>(null);
  const [live, setLive] = useState<CurvePoint[] | null>(null);
  const [guide, setGuide] = useState(false);
  useEffect(() => {
    if (live === null) return undefined;
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopPropagation();
      gesture.current = null; setLive(null); setGuide(false); onCancel();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [live !== null]);
  const points = live ?? active.points;
  const where = (event: { clientX: number; clientY: number }): CurvePoint => {
    const box = area.current!.getBoundingClientRect();
    const nx = (event.clientX - box.left) / Math.max(1, box.width);
    const ny = 1 - (event.clientY - box.top) / Math.max(1, box.height);
    return [Math.min(1, Math.max(0, nx)), low + Math.min(1, Math.max(0, ny)) * (high - low)];
  };
  const near = (event: { clientX: number; clientY: number }): number => {
    const box = area.current!.getBoundingClientRect();
    let found = -1;
    let best = HIT_PX;
    points.forEach(([x, y], i) => {
      const px = box.left + x * box.width;
      const py = box.top + (1 - (y - low) / (high - low)) * box.height;
      const d = Math.hypot(event.clientX - px, event.clientY - py);
      if (d <= best) { best = d; found = i; }
    });
    return found;
  };
  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    const hit = near(event);
    let now: CurvePoint[] = [...active.points];
    let index = hit;
    let added = false;
    if (hit < 0) {
      const [x, y] = where(event);
      // Shift: on the curve as it is, which does not change it (p. 3108).
      const at: CurvePoint = event.shiftKey ? [x, curveAt(active.points, x, kind)] : [x, y];
      const made = addPoint(active.points, at, custom);
      if (made === null) return;
      now = made.points; index = made.index; added = true;
      setLive(now); onPreview(now);
    }
    gesture.current = { pointer: event.pointerId, index, points: now, added, moved: false, x: event.clientX, y: event.clientY };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    onChoose(index);
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const now = gesture.current;
    if (!now || now.pointer !== event.pointerId) return;
    if (!now.moved && Math.hypot(event.clientX - now.x, event.clientY - now.y) < 2) return;
    now.moved = true;
    let [x, y] = where(event);
    // Alt: onto the neutral diagonal, which is drawn while it holds (p. 3108; custom curves only).
    const snap = custom && event.altKey;
    if (snap) y = x;
    setGuide(snap);
    const next = movePoint(now.points, now.index, [x, y], custom);
    now.points = next;
    setLive(next); onPreview(next);
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    const now = gesture.current;
    if (!now || now.pointer !== event.pointerId) return;
    gesture.current = null; setGuide(false);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    setLive(null);
    if (now.added || now.moved) onCommit(now.points);
  };
  const menu = (event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (disabled) return;
    const hit = near(event);
    if (hit < 0) return;
    const left = removePoint(active.points, hit, custom);
    if (left === null) return;
    onChoose(null);
    onCommit(left);
  };
  const path = (list: readonly CurvePoint[]) => {
    const steps: string[] = [];
    for (let i = 0; i <= STEPS; i++) {
      const x = i / STEPS;
      const y = curveAt(list, x, kind);
      steps.push(`${i === 0 ? 'M' : 'L'}${x.toFixed(4)},${(1 - (y - low) / (high - low)).toFixed(4)}`);
    }
    return steps.join(' ');
  };
  return <div ref={area} className="color-curve-area" data-kind={kind} role="application" aria-label={name} aria-disabled={disabled || undefined}
    onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => { gesture.current = null; setLive(null); setGuide(false); onCancel(); }}
    onContextMenu={menu} data-test="curve-editor">
    <svg className="color-curve-svg" viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden="true">
      {custom ? <line className="color-curve-neutral" x1="0" y1="1" x2="1" y2="0" vectorEffect="non-scaling-stroke" />
        : <line className="color-curve-neutral" x1="0" y1="0.5" x2="1" y2="0.5" vectorEffect="non-scaling-stroke" />}
      {guide && <line className="color-curve-guide" x1="0" y1="1" x2="1" y2="0" vectorEffect="non-scaling-stroke" data-test="diagonal-guide" />}
      {drawn.filter(each => !each.active).map(each => <path key={each.id} className="color-curve-line" data-curve={each.id} d={path(each.points)} vectorEffect="non-scaling-stroke" />)}
      <path className="color-curve-line" data-curve={active.id} data-active="" d={path(points)} vectorEffect="non-scaling-stroke" />
    </svg>
    {points.map(([x, y], i) => <span key={i} className="color-curve-point" role="img" aria-label={`${pointName} ${i + 1}`} data-index={i}
      data-chosen={chosen === i || undefined} data-curve={active.id} data-test="curve-point"
      style={{ left: `${x * 100}%`, top: `${(1 - (y - low) / (high - low)) * 100}%` }} />)}
  </div>;
}

/** The Input and Output of a point of an HSL curve as they are shown (p. 3123): hues in degrees, a rotation in degrees, a gain as 1 for none. */
const inputShown = (curve: HslCurve) => (isHueCurve(curve) ? { to: (x: number) => x * 360, from: (d: number) => d / 360, digits: 2 } : { to: (x: number) => x, from: (d: number) => d, digits: 3 });
const outputShown = (curve: HslCurve) => (curve === 'hue_vs_hue'
  ? { to: (y: number) => y * 180, from: (d: number) => d / 180, digits: 2 }
  : { to: (y: number) => 1 + y, from: (d: number) => d - 1, digits: 2 });

/** Soft Clip beneath the custom curves (pp. 3117–3121): ganged at first, so R, G and B are set alike. */
function SoftClip({ edit }: { edit: NodeEdit }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.curvesPalette;
  const [ganged, setGanged] = useState(true);
  // High and Low are shown on the 10-bit scale of the scopes, 0 to 1023 (p. 3119); the engine keeps a share of 100.
  const tenBit = { to: (value: number) => value * 10.23, from: (shown: number) => shown / 10.23 };
  const fields = [
    { key: 'low', name: labels.low, bounds: BOUNDS.clip_low, feel: SETTING_FEEL.clip, shown: tenBit },
    { key: 'low_soft', name: labels.lowSoft, bounds: BOUNDS.clip_soft, feel: SETTING_FEEL.clip_soft, shown: undefined },
    { key: 'high', name: labels.high, bounds: BOUNDS.clip_high, feel: SETTING_FEEL.clip, shown: tenBit },
    { key: 'high_soft', name: labels.highSoft, bounds: BOUNDS.clip_soft, feel: SETTING_FEEL.clip_soft, shown: undefined },
  ] as const;
  const rows: readonly (readonly ('r' | 'g' | 'b')[])[] = ganged ? [['r', 'g', 'b']] : [['r'], ['g'], ['b']];
  return <div className="color-soft-clip" data-test="soft-clip">
    <div className="color-soft-clip-head">
      <span className="overline">{labels.softClip}</span>
      <button type="button" className="icon-btn" aria-pressed={ganged} aria-label={labels.gangSoftClip} title={labels.gangSoftClip}
        onClick={() => setGanged(on => !on)} data-test="gang-soft-clip"><Link2 size={12} /></button>
    </div>
    {rows.map(channels => <div key={channels.join('')} className="color-settings" data-channels={channels.join('')}>
      {!ganged && <span className="color-channel-letter" data-channel={channels[0]}>{channels[0].toUpperCase()}</span>}
      {fields.map(({ key, name, bounds, feel, shown }) => {
        const paths = channels.map(ch => `soft_clip.${key}.${ch}`);
        const set = (value: number): Edits => paths.map(path => [path, value] as const);
        return <NumberControl key={key} name={name} field={paths[0]} value={edit.value(paths[0])} neutral={bounds.neutral} low={bounds.low} high={bounds.high}
          digits={feel.digits} perPx={feel.perPx} shown={shown} keyed={paths.some(path => edit.keyed(path))} keyedTitle={words.hint.studio.colorPage.keyed}
          disabled={!edit.enabled} onPreview={value => edit.preview(set(value))} onCommit={value => edit.commit(set(value))} onCancel={edit.cancel} />;
      })}
    </div>)}
  </div>;
}

export function CurvesPalette({ edit, mode, onMode }: { edit: NodeEdit; mode: CurveMode; onMode: (mode: CurveMode) => void }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.curvesPalette;
  const disabled = !edit.enabled;
  const curves = edit.corrector.curves;
  const ganged = curves.ganged;
  const [channel, setChannel] = useState<Channel>('y');
  const [chosen, setChosen] = useState<number | null>(null);
  useEffect(() => { setChosen(null); }, [mode]);
  const editing: Channel = ganged ? 'y' : channel;

  // Ganged, the four curves are edited as one (pp. 3115–3116): each gets the same points.
  const customEdits = (points: CurvePoint[]): Edits => (ganged ? CHANNELS : [editing]).map(ch => [`curves.${ch}.points`, { whole: points }] as const);
  const hslEdits = (curve: HslCurve, points: CurvePoint[]): Edits => [[`hsl.${curve}`, { whole: points }]];
  const reset = () => {
    if (mode === 'custom') {
      // Reset Custom Curve (p. 3109): every channel's curve and intensity, as new.
      edit.commit(CHANNELS.flatMap(ch => [[`curves.${ch}.points`, { whole: [[0, 0], [1, 1]] }], [`curves.${ch}.intensity`, BOUNDS.intensity.neutral]] as const));
    } else edit.commit(hslEdits(mode, []));
    setChosen(null);
  };
  const resetChannel = (ch: Channel) => {
    // The reset beside an intensity (p. 3109): that channel's curve, neutral; ganged, Y is the four.
    const channels = ganged && ch === 'y' ? CHANNELS : [ch];
    edit.commit(channels.flatMap(each => [[`curves.${each}.points`, { whole: [[0, 0], [1, 1]] }], [`curves.${each}.intensity`, BOUNDS.intensity.neutral]] as const));
  };
  const option = (value: string) => {
    const points = curves[editing].points;
    if (value === 'anchors') edit.commit(customEdits(defaultAnchors(points)));
    else if (value === 'r' || value === 'g' || value === 'b') edit.commit([[`curves.${value}.points`, { whole: points }]]);
  };

  const head = <div className="color-palette-head">
    <button type="button" className="icon-btn" disabled aria-label={labels.picker} title={`${labels.picker}: ${words.hint.studio.colorPage.picker}`}
      data-test="curve-picker"><Pipette size={13} /></button>
    {mode === 'custom' && <select className="field color-curve-options" aria-label={labels.options} value="" disabled={disabled}
      onChange={(event) => { option(event.target.value); event.currentTarget.value = ''; }} data-test="curve-options">
      <option value="">{labels.options}</option>
      <option value="anchors">{labels.defaultAnchors}</option>
      <option value="r">{labels.copyToRed}</option>
      <option value="g">{labels.copyToGreen}</option>
      <option value="b">{labels.copyToBlue}</option>
    </select>}
    <span className="color-gap" />
    <span className="color-modes" role="radiogroup" aria-label={labels.modes}>
      {CURVE_MODES.map(each => <button key={each} type="button" role="radio" aria-checked={mode === each} className="color-mode"
        onClick={() => onMode(each)} data-mode={each}>{labels[each]}</button>)}
    </span>
    <button type="button" className="btn color-reset" disabled={disabled} onClick={reset} data-test="reset-palette">
      <RotateCcw size={12} aria-hidden="true" />{mode === 'custom' ? labels.resetCustom : labels.reset}</button>
  </div>;

  if (mode === 'custom') {
    const drawn: Drawn[] = ganged ? [{ id: 'y', points: curves.y.points, active: true }]
      : CHANNELS.map(ch => ({ id: ch, points: curves[ch].points, active: ch === editing }));
    return <div className="color-curves" data-mode="custom" data-test="curves">
      {head}
      <div className="color-curves-body">
        <CurveEditor drawn={drawn} kind="custom" name={labels.editor} pointName={labels.point} disabled={disabled} chosen={chosen} onChoose={setChosen}
          onPreview={points => edit.preview(customEdits(points))} onCommit={points => edit.commit(customEdits(points))} onCancel={edit.cancel} />
        <div className="color-curves-side">
          <div className="color-curve-channels" role="group" aria-label={labels.editor}>
            {/* The Gang button, at the left of the Curve Edit buttons (p. 3116). */}
            <button type="button" className="icon-btn" aria-pressed={ganged} aria-label={labels.gang} title={labels.gang} disabled={disabled}
              onClick={() => { if (!ganged) edit.commit([['curves.ganged', { whole: true }]]); }} data-test="gang"><Link2 size={12} /></button>
            {/* A Curve Edit button chooses its curve, and ungangs them (p. 3115). */}
            {CHANNELS.map(ch => <button key={ch} type="button" className="color-channel" data-channel={ch} aria-pressed={!ganged && ch === editing}
              aria-label={labels.channels[ch]} title={labels.channels[ch]} disabled={disabled}
              onClick={() => { setChannel(ch); if (ganged) edit.commit([['curves.ganged', { whole: false }]]); }}>{LETTER[ch]}</button>)}
          </div>
          <div className="color-intensities">
            <span className="overline">{labels.intensity}</span>
            {CHANNELS.map((ch) => {
              const path = `curves.${ch}.intensity`;
              return <div key={ch} className="color-intensity" data-channel={ch}>
                <NumberControl name={`${labels.intensity} ${LETTER[ch]}`} label={LETTER[ch]} field={path} value={edit.value(path)}
                  neutral={BOUNDS.intensity.neutral} low={BOUNDS.intensity.low} high={BOUNDS.intensity.high}
                  digits={SETTING_FEEL.intensity.digits} perPx={SETTING_FEEL.intensity.perPx} keyed={edit.keyed(path)}
                  keyedTitle={words.hint.studio.colorPage.keyed} disabled={disabled}
                  onPreview={value => edit.preview([[path, value]])} onCommit={value => edit.commit([[path, value]])} onCancel={edit.cancel} />
                <button type="button" className="icon-btn" aria-label={`${labels.resetChannel}: ${labels.channels[ch]}`} title={labels.resetChannel}
                  disabled={disabled} onClick={() => resetChannel(ch)} data-test={`reset-${ch}`}><RotateCcw size={11} /></button>
              </div>;
            })}
          </div>
          <SoftClip edit={edit} />
        </div>
      </div>
    </div>;
  }

  const curve = mode;
  const points = edit.corrector.hsl[curve];
  const hue = isHueCurve(curve);
  const point = chosen !== null && chosen < points.length ? points[chosen] : null;
  const input = inputShown(curve);
  const output = outputShown(curve);
  const outName = curve === 'hue_vs_hue' ? labels.hueRotate : curve.endsWith('_sat') ? labels.saturation : labels.lum;
  const setPoint = (index: number, next: CurvePoint): Edits => hslEdits(curve, movePoint(points, index, next, false));
  return <div className="color-curves" data-mode={curve} data-test="curves">
    {head}
    <div className="color-curves-body" data-hue={hue || undefined}>
      <div className="color-curve-column">
        <CurveEditor drawn={[{ id: curve, points, active: true }]} kind={hue ? 'round' : 'flat'} name={labels.editor} pointName={labels.point}
          disabled={disabled} chosen={chosen} onChoose={setChosen}
          onPreview={next => edit.preview(hslEdits(curve, next))} onCommit={next => edit.commit(hslEdits(curve, next))} onCancel={edit.cancel} />
        {hue && <div className="color-hue-strip" aria-hidden="true" />}
      </div>
      <div className="color-curves-side">
        {hue && <div className="color-vectors" role="group" aria-label={labels.modes}>
          {(Object.keys(VECTORS) as Vector[]).map(each => <button key={each} type="button" className="color-vector" data-vector={each}
            aria-label={labels.vectors[each]} title={labels.vectors[each]} disabled={disabled}
            onClick={() => edit.commit(hslEdits(curve, addVector(points, VECTORS[each])))} />)}
        </div>}
        <div className="color-settings">
          <NumberControl name={labels.input} field={`hsl.${curve}.input`} value={point?.[0] ?? 0} neutral={point?.[0] ?? 0} low={0} high={1}
            digits={input.digits} perPx={0.002} shown={input} disabled={disabled || point === null}
            onPreview={value => { if (chosen !== null && point) edit.preview(setPoint(chosen, [value, point[1]])); }}
            onCommit={value => { if (chosen !== null && point) edit.commit(setPoint(chosen, [value, point[1]])); }} onCancel={edit.cancel} />
          <NumberControl name={outName} field={`hsl.${curve}.output`} value={point?.[1] ?? 0} neutral={0} low={-1} high={1}
            digits={output.digits} perPx={0.004} shown={output} disabled={disabled || point === null}
            onPreview={value => { if (chosen !== null && point) edit.preview(setPoint(chosen, [point[0], value])); }}
            onCommit={value => { if (chosen !== null && point) edit.commit(setPoint(chosen, [point[0], value])); }} onCancel={edit.cancel} />
        </div>
      </div>
    </div>
  </div>;
}
