import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';

/**
 * The number controls of the palettes, as DaVinci Resolve's on the Color page (Reference Manual
 * 20, pp. 62, 67–68, 3062; the interaction reference, "通用数值控件规则"):
 *
 * - The name or the number dragged left or right is a virtual slider, right to raise; the number's
 *   is finer than a slider beside it would be. There is no Shift or Ctrl to go finer, and no wheel.
 * - A double click on the number writes into it: Enter sets it, Escape puts it back; Up and Down
 *   change the digit at the left of the cursor, or the last digit where all is chosen.
 * - A double click on the name puts the default back.
 * - What is dragged is shown at once and sent once, when it is let go (one step to undo); Escape
 *   while dragging puts it back.
 * - A setting with keyframes shows the value of the key that an edit goes into (p. 3391), with a
 *   small diamond (ours: DaVinci Resolve marks keys in its Keyframe Editor only).
 */

/** How a number is shown: in other units than it is kept in (the levels of Soft Clip in 10 bits, say). */
export interface Shown { to: (value: number) => number; from: (shown: number) => number }
const SAME: Shown = { to: value => value, from: value => value };

export interface NumberProps {
  name: string;
  /** What the name shows, where it is not the name itself. */
  label?: ReactNode;
  /** For tests and styles. */
  field: string;
  value: number;
  neutral: number;
  low: number;
  high: number;
  /** The decimals shown (DaVinci keeps three, p. 3062). */
  digits: number;
  /** What a pixel of dragging adds, in the units the value is kept in. */
  perPx: number;
  shown?: Shown;
  keyed?: boolean;
  /** Where the hint of a key is, the words that say what a change of it does. */
  keyedTitle?: string;
  disabled?: boolean;
  /** No name beside the box: the numbers beneath a wheel, named by the labels beneath them. */
  bare?: boolean;
  onPreview: (value: number) => void;
  onCommit: (value: number) => void;
  onCancel: () => void;
}

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));
const keep = (value: number) => Math.round(value * 1000) / 1000;

/** The pointer dragging a control sideways: where it began and what it began at. */
interface Drag { x: number; y: number; start: number; pointer: number; moved: boolean; last: number; identity?: string }

/** A drag of a control that tells of each move and of its end; Escape while it lasts puts back what was there. */
export function useDrag(onCancel: () => void, identity?: () => string, onInvalidate?: () => void) {
  const drag = useRef<Drag | null>(null);
  const [dragging, setDragging] = useState(false);
  const cancel = useRef(onCancel);
  cancel.current = onCancel;
  const owner = useRef(identity);
  owner.current = identity;
  const invalidated = useRef(onInvalidate);
  invalidated.current = onInvalidate;
  const drop = () => {
    drag.current = null; setDragging(false);
    // Forget the control's local starting values too, without cancelling an engine edit.
    invalidated.current?.();
  };
  const observedIdentity = identity?.();
  useLayoutEffect(() => {
    // Playback can cross a key and return without any pointer event. Once observed, that key
    // change ends this down permanently; coming back cannot revive its old starting value.
    if (drag.current && drag.current.identity !== observedIdentity) {
      drop();
    }
  }, [observedIdentity]);
  const read = () => {
    if (drag.current && drag.current.identity !== owner.current?.()) {
      // A down before the first preview still owns its original parameter/key. Dropping that
      // pointer must not cancel a gesture that now belongs to another key or control.
      drop();
    }
    return drag.current;
  };
  useEffect(() => {
    if (!dragging) return undefined;
    const key = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape' || !read()) return;
      event.preventDefault(); event.stopPropagation();
      drag.current = null; setDragging(false); cancel.current();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [dragging]);
  return {
    down(event: PointerEvent<Element>, start: number) {
      drag.current = { x: event.clientX, y: event.clientY, start, pointer: event.pointerId, moved: false, last: start, identity: owner.current?.() };
      (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    },
    /** How far the pointer is from where it began, once it has gone 3 pixels; null before, or for another pointer. */
    move(event: PointerEvent<Element>): { dx: number; dy: number; start: number } | null {
      const now = read();
      if (!now || now.pointer !== event.pointerId) return null;
      const dx = event.clientX - now.x;
      const dy = event.clientY - now.y;
      if (!now.moved && Math.abs(dx) < 3 && Math.abs(dy) < 3) return null;
      if (!now.moved) { now.moved = true; setDragging(true); }
      return { dx, dy, start: now.start };
    },
    /** Null for an invalid capture; otherwise whether this valid click/drag moved. */
    up(event: PointerEvent<Element>): boolean | null {
      const now = read();
      if (!now || now.pointer !== event.pointerId) return null;
      drag.current = null; setDragging(false);
      const element = event.currentTarget as Element;
      if (element.hasPointerCapture?.(event.pointerId)) element.releasePointerCapture(event.pointerId);
      return now.moved;
    },
    /** Remember the last value shown, to send at the end. */
    set last(value: number) { if (drag.current) drag.current.last = value; },
    get last() { return drag.current?.last ?? NaN; },
    cancel() { if (read()) { drag.current = null; setDragging(false); cancel.current(); } },
  };
}

/** The place of the digit that Up and Down change, from the cursor in the text of a number: 1 for units, 0.1 for tenths. */
export function stepAtCursor(text: string, cursor: number, chosenAll: boolean, digits: number): number {
  const least = 10 ** -digits;
  if (chosenAll) return least;
  const point = text.indexOf('.') < 0 ? text.length : text.indexOf('.');
  // The digit at the left of the cursor; the cursor at the start goes by the first digit.
  let at = Math.max(1, cursor) - 1;
  if (text[at] === '.' ) at -= 1;
  if (at < 0 || !/[0-9]/.test(text[at] ?? '')) return least;
  return at < point ? 10 ** (point - at - 1) : 10 ** -(at - point);
}

/** A number of a palette: a name and a box, both of which drag. */
export function NumberControl(props: NumberProps) {
  const id = useId();
  const shown = props.shown ?? SAME;
  const [draft, setDraft] = useState<string | null>(null);
  const [live, setLive] = useState<number | null>(null);
  const box = useRef<HTMLInputElement>(null);
  const drag = useDrag(() => { setLive(null); props.onCancel(); });
  const value = live ?? props.value;
  const digits = props.digits;
  const text = draft ?? shown.to(value).toFixed(digits);
  const toValue = (shownValue: number) => keep(clamp(shown.from(shownValue), props.low, props.high));

  const down = (event: PointerEvent<HTMLElement>) => {
    if (props.disabled || event.button !== 0 || draft !== null) return;
    event.preventDefault();
    drag.down(event, props.value);
  };
  const move = (event: PointerEvent<HTMLElement>) => {
    const moved = drag.move(event);
    if (!moved) return;
    const next = keep(clamp(moved.start + moved.dx * props.perPx, props.low, props.high));
    drag.last = next;
    setLive(next);
    props.onPreview(next);
  };
  const up = (event: PointerEvent<HTMLElement>) => {
    const last = drag.last;
    if (!drag.up(event)) return;
    setLive(null);
    if (Number.isFinite(last)) props.onCommit(last);
  };
  const type = () => {
    if (props.disabled) return;
    setDraft(shown.to(props.value).toFixed(digits));
    requestAnimationFrame(() => { box.current?.focus(); box.current?.select(); });
  };
  const send = () => {
    if (draft === null) return;
    const read = Number(draft.trim().replace(',', '.'));
    setDraft(null); setLive(null);
    if (draft.trim() === '' || !Number.isFinite(read)) { props.onCancel(); return; }
    // Out of range, a number written is kept to the range (the manual does not say).
    props.onCommit(toValue(read));
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (draft === null) {
      if (event.key === 'Enter') { event.preventDefault(); type(); }
      return;
    }
    if (event.key === 'Enter') { event.preventDefault(); send(); return; }
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setDraft(null); setLive(null); props.onCancel(); return; }
    if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault(); event.stopPropagation();
      const input = event.currentTarget;
      const start = input.selectionStart ?? draft.length;
      const all = start === 0 && (input.selectionEnd ?? 0) >= draft.length;
      const step = stepAtCursor(draft, start, all, digits);
      const read = Number(draft.replace(',', '.'));
      if (!Number.isFinite(read)) return;
      const next = toValue(read + (event.key === 'ArrowUp' ? step : -step));
      const written = shown.to(next).toFixed(digits);
      setDraft(written);
      setLive(next);
      props.onPreview(next);
      // The cursor stays at the same digit from the right.
      const fromRight = draft.length - start;
      requestAnimationFrame(() => { if (!all) input.setSelectionRange(written.length - fromRight, written.length - fromRight); });
    }
  };
  return <span className="color-number" data-field={props.field} data-keyed={props.keyed || undefined}
    data-changed={props.value !== props.neutral || undefined}>
    {!props.bare && <label htmlFor={id} className="color-number-name" data-disabled={props.disabled || undefined}
      onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => drag.cancel()}
      onDoubleClick={(event) => { event.preventDefault(); if (!props.disabled) props.onCommit(props.neutral); }}>{props.label ?? props.name}</label>}
    <input ref={box} id={id} className="color-number-box mono" type="text" inputMode="decimal" autoComplete="off" spellCheck={false}
      aria-label={props.name} value={text} readOnly={draft === null} disabled={props.disabled}
      onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => drag.cancel()}
      onDoubleClick={(event) => { event.preventDefault(); type(); }}
      onChange={event => setDraft(event.target.value)} onKeyDown={keys} onBlur={send} />
    {props.keyed && <span className="color-keyed" role="img" aria-label={props.keyedTitle} title={props.keyedTitle} data-test="keyed">◆</span>}
  </span>;
}

/**
 * A wheel to drag left or right (a Master Wheel, p. 3062): left darker, right lighter. It tells how
 * far it was dragged; up and down do nothing. Its ridges turn as it is dragged.
 */
export function MasterWheel({ name, turned, disabled, identity, onDrag, onEnd, onCancel, onInvalidate }: {
  name: string;
  /** How far it is turned, to draw its ridges: the change it has made, in pixels of dragging. */
  turned: number;
  disabled?: boolean;
  /** The parameters/keys this master adjusts, captured even before its first move. */
  identity?: string;
  onDrag: (dx: number) => void;
  onEnd: (dx: number) => void;
  onCancel: () => void;
  /** Clear local starting values when their parameter/key identity is lost. */
  onInvalidate?: () => void;
}) {
  const drag = useDrag(onCancel, () => identity ?? '', onInvalidate);
  return <div className="color-master" role="slider" aria-label={name} aria-valuenow={Math.round(turned)} aria-disabled={disabled || undefined}
    tabIndex={-1} data-test="master-wheel" style={{ backgroundPositionX: `${turned}px` }}
    onPointerDown={(event) => { if (disabled || event.button !== 0) return; event.preventDefault(); drag.down(event, 0); }}
    onPointerMove={(event) => { const moved = drag.move(event); if (moved) { drag.last = moved.dx; onDrag(moved.dx); } }}
    onPointerUp={(event) => { const last = drag.last; if (drag.up(event) && Number.isFinite(last)) onEnd(last); }}
    onPointerCancel={() => drag.cancel()} />;
}

/**
 * An upright slider of the Bars mode (p. 3068): dragged up to raise, down to lower, its fill
 * drawn from its neutral value. It shows `low` to `high`; the number beneath it goes further.
 */
export function Bar({ name, channel, value, neutral, low, high, perPx, disabled, onPreview, onCommit, onCancel }: {
  name: string; channel: 'y' | 'r' | 'g' | 'b'; value: number; neutral: number; low: number; high: number; perPx: number;
  disabled?: boolean;
  onPreview: (value: number) => void; onCommit: (value: number) => void; onCancel: () => void;
}) {
  const [live, setLive] = useState<number | null>(null);
  const drag = useDrag(() => { setLive(null); onCancel(); });
  const shown = live ?? value;
  const share = (each: number) => (clamp(each, low, high) - low) / (high - low);
  const from = share(neutral);
  const to = share(shown);
  return <div className="color-bar-slider" role="slider" aria-label={name} aria-valuenow={shown} aria-valuemin={low} aria-valuemax={high}
    aria-orientation="vertical" data-channel={channel} data-test={`bar-${channel}`}
    onPointerDown={(event) => { if (disabled || event.button !== 0) return; event.preventDefault(); drag.down(event, value); }}
    onPointerMove={(event) => {
      const moved = drag.move(event);
      if (!moved) return;
      const next = keep(moved.start - moved.dy * perPx);
      drag.last = next; setLive(next); onPreview(next);
    }}
    onPointerUp={(event) => { const last = drag.last; if (drag.up(event)) { setLive(null); if (Number.isFinite(last)) onCommit(last); } }}
    onPointerCancel={() => drag.cancel()}>
    <span className="color-bar-fill" style={{ bottom: `${Math.min(from, to) * 100}%`, height: `${Math.abs(to - from) * 100}%` }} />
    <span className="color-bar-mark" style={{ bottom: `${from * 100}%` }} />
  </div>;
}
