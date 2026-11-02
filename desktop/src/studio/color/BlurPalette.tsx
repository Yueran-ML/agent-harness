import { useEffect, useRef } from 'react';
import { Link2, RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { BlurMode } from '../engine';
import {
  BLUR_BOUNDS, BLUR_CHANNELS, BLUR_COLUMNS, BLUR_FEEL, BLUR_MODES, BLUR_SINGLES, blurOf, blurUses, columnEdits, columnPaths,
  type BlurChannel, type BlurColumn, type BlurSingle,
} from './blur';
import { Bar, NumberControl } from './controls';
import type { Edits, NodeEdit } from './useNodeEdit';
import './blur.css';

/**
 * The Blur palette (Reference Manual 20, pp. 3454–3456): its tab, Blur, Sharpen or Mist, chosen at
 * its upper right; the three columns of Radius, H/V Ratio and Scaling, each an upright bar and a
 * number for red, green and blue, with the gang button left of its name and a reset at its right;
 * and Coring Softness, Level and Mix beneath. What a tab does not use is grey, and kept (the
 * engine keeps one set of settings for the three tabs).
 *
 * Ganged, as the columns are at first, a bar dragged moves the three by as much as it moves; ganging
 * them again does not make them alike. A notch of the wheel over a bar of Radius moves it by 0.01
 * (p. 3454); no other control of the palette takes the wheel. Each drag, turn of the wheel, typed
 * number or reset is one step to undo: the notches of a turn are one gesture, as a drag is, which
 * ends once the wheel has rested for `BLUR_FEEL.wheelRestMs`, or the pointer leaves or presses the
 * bar.
 */
const LETTER: Record<BlurChannel, string> = { r: 'R', g: 'G', b: 'B' };

/**
 * A notch of the wheel over `element`, to `onNotch` with +1 up and −1 down; it does not scroll what
 * is under it. `onRest` is told once the notches of a turn are over: the wheel has rested for
 * `BLUR_FEEL.wheelRestMs`, the pointer has left the element or pressed it (before what the press
 * does, so that a drag that follows is a gesture of its own), or the element is gone.
 */
function useWheel(onNotch: ((notch: 1 | -1) => void) | null, onRest: () => void) {
  const element = useRef<HTMLDivElement>(null);
  const handlers = useRef({ onNotch, onRest });
  handlers.current = { onNotch, onRest };
  useEffect(() => {
    const at = element.current;
    if (!at) return undefined;
    let resting: ReturnType<typeof setTimeout> | null = null;
    const rest = () => {
      if (resting === null) return;
      clearTimeout(resting);
      resting = null;
      handlers.current.onRest();
    };
    const wheel = (event: WheelEvent) => {
      const notch = handlers.current.onNotch;
      if (!notch || event.deltaY === 0) return;
      event.preventDefault();
      if (resting !== null) clearTimeout(resting);
      resting = setTimeout(rest, BLUR_FEEL.wheelRestMs);
      notch(event.deltaY < 0 ? 1 : -1);
    };
    at.addEventListener('wheel', wheel, { passive: false });
    at.addEventListener('pointerleave', rest);
    at.addEventListener('pointerdown', rest, true);
    return () => {
      at.removeEventListener('wheel', wheel);
      at.removeEventListener('pointerleave', rest);
      at.removeEventListener('pointerdown', rest, true);
      rest();
    };
  }, []);
  return element;
}

function Channel({ edit, column, ch, disabled }: { edit: NodeEdit; column: BlurColumn; ch: BlurChannel; disabled: boolean }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.blurPalette;
  const bounds = BLUR_BOUNDS[column];
  const path = `blur.${column}.${ch}`;
  const paths = columnPaths(edit.corrector, column, ch);
  const edits = (value: number): Edits => columnEdits(edit.base, edit.frame, column, ch, value);
  // Where the turn of the wheel has taken this channel: each notch goes on from it, as a drag does.
  const turned = useRef<number | null>(null);
  const wheel = useWheel(column === 'radius' && !disabled ? notch => {
    const next = columnEdits(edit.base, edit.frame, column, ch, (turned.current ?? edit.value(path)) + notch * BLUR_FEEL.wheelStep);
    turned.current = next.find(([each]) => each === path)?.[1] ?? null;
    edit.preview(next);
  } : null, () => {
    const at = turned.current;
    turned.current = null;
    if (at !== null) edit.commit(edits(at));
  });
  const name = `${labels[column]} ${LETTER[ch]}`;
  return <div ref={wheel} className="color-bar-column" data-test={`blur-${column}-${ch}`}>
    <Bar name={name} channel={ch} value={edit.value(path)} neutral={bounds.neutral} low={bounds.low} high={bounds.high}
      perPx={BLUR_FEEL.barPerPx} disabled={disabled}
      onPreview={value => edit.preview(edits(value))} onCommit={value => edit.commit(edits(value))} onCancel={edit.cancel} />
    <NumberControl bare name={name} field={path} value={edit.value(path)} neutral={bounds.neutral} low={bounds.low} high={bounds.high}
      digits={BLUR_FEEL.digits} perPx={BLUR_FEEL.fieldPerPx} keyed={paths.some(each => edit.keyed(each))}
      keyedTitle={words.hint.studio.colorPage.keyed} disabled={disabled}
      onPreview={value => edit.preview(edits(value))} onCommit={value => edit.commit(edits(value))} onCancel={edit.cancel} />
    <span className="color-bar-letter" data-channel={ch}>{LETTER[ch]}</span>
  </div>;
}

function Column({ edit, column, mode }: { edit: NodeEdit; column: BlurColumn; mode: BlurMode }) {
  const labels = useWords().label.studio.colorPage.blurPalette;
  const linked = blurOf(edit.corrector).linked[column];
  const used = blurUses(mode, column);
  const disabled = !edit.enabled || !used;
  const reset = () => edit.commit(BLUR_CHANNELS.map(ch => [`blur.${column}.${ch}`, BLUR_BOUNDS[column].neutral] as const));
  return <div className="color-blur-column" data-column={column} data-unused={!used || undefined}>
    <div className="color-blur-head">
      <button type="button" className="icon-btn" aria-pressed={linked} aria-label={`${labels.gang}: ${labels[column]}`} title={labels.gang}
        disabled={!edit.enabled} onClick={() => edit.commit([[`blur.linked.${column}`, { whole: !linked }]])} data-test={`blur-gang-${column}`}>
        <Link2 size={12} /></button>
      <span className="overline">{labels[column]}</span>
      <button type="button" className="icon-btn" aria-label={`${labels.resetColumn}: ${labels[column]}`} title={labels.resetColumn}
        disabled={disabled} onClick={reset} data-test={`blur-reset-${column}`}><RotateCcw size={11} /></button>
    </div>
    <div className="color-bars-row">
      {BLUR_CHANNELS.map(ch => <Channel key={`${ch}:${edit.controlKey(columnPaths(edit.corrector, column, ch))}`} edit={edit} column={column} ch={ch} disabled={disabled} />)}
    </div>
  </div>;
}

function Single({ edit, single, mode }: { edit: NodeEdit; single: BlurSingle; mode: BlurMode }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.blurPalette;
  const bounds = BLUR_BOUNDS[single];
  const path = `blur.${single}`;
  return <NumberControl key={edit.controlKey([path])} name={labels[single]} field={path} value={edit.value(path)}
    neutral={bounds.neutral} low={bounds.low} high={bounds.high} digits={BLUR_FEEL.digits}
    perPx={single === 'mix' ? BLUR_FEEL.mixPerPx : BLUR_FEEL.fieldPerPx} keyed={edit.keyed(path)}
    keyedTitle={words.hint.studio.colorPage.keyed} disabled={!edit.enabled || !blurUses(mode, single)}
    onPreview={value => edit.preview([[path, value]])} onCommit={value => edit.commit([[path, value]])} onCancel={edit.cancel} />;
}

export function BlurPalette({ edit }: { edit: NodeEdit }) {
  const labels = useWords().label.studio.colorPage.blurPalette;
  const disabled = !edit.enabled;
  const mode = blurOf(edit.corrector).mode;
  const reset = () => edit.commit([
    ...BLUR_COLUMNS.flatMap(column => BLUR_CHANNELS.map(ch => [`blur.${column}.${ch}`, BLUR_BOUNDS[column].neutral] as const)),
    ...BLUR_SINGLES.map(single => [`blur.${single}`, BLUR_BOUNDS[single].neutral] as const),
  ]);
  return <div className="color-blur" data-mode={mode} data-test="blur-palette">
    <div className="color-palette-head">
      <span className="color-gap" />
      <span className="color-modes" role="radiogroup" aria-label={labels.modes}>
        {BLUR_MODES.map(each => <button key={each} type="button" role="radio" aria-checked={mode === each} className="color-mode"
          disabled={disabled} onClick={() => { if (each !== mode) edit.commit([['blur.mode', { whole: each }]]); }} data-blur-mode={each}>
          {labels[each]}</button>)}
      </span>
      <button type="button" className="btn color-reset" disabled={disabled} onClick={reset} data-test="blur-reset">
        <RotateCcw size={12} aria-hidden="true" />{labels.reset}</button>
    </div>
    <div className="color-blur-columns">
      {BLUR_COLUMNS.map(column => <Column key={column} edit={edit} column={column} mode={mode} />)}
    </div>
    <div className="color-settings color-blur-singles">
      {BLUR_SINGLES.map(single => <Single key={single} edit={edit} single={single} mode={mode} />)}
    </div>
  </div>;
}
