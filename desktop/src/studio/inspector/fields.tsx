import { useEffect, useId, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';

/**
 * The fields of the inspector, as DaVinci Resolve's: what is typed, dragged or slid is shown at
 * once, and sent once, as one step to take back, when it is let go or Enter is pressed. Escape puts
 * back what was there. A number box dragged sideways is a virtual slider, as DaVinci Resolve's
 * (Reference Manual 20, p. 67), and so is its label; pressed without dragging, the box is written
 * in. A label double-clicked puts the default back (p. 68). A field of several clips that differ
 * shows a dash, and sets them all alike.
 */

/** What a field shows where the clips it sets differ. */
const MIXED = '—';
/** How long a value that was sent is shown, at most, until the engine tells of it: it does in a moment, or refused it. */
export const HOLD_MS = 1500;

/**
 * What was sent, shown until the value of the clips changes (the engine told of the change) or a
 * while has passed, so that a field does not jump back to the value before for a moment.
 */
function useHeld<T extends string | number | null>(value: T) {
  const [held, setHeld] = useState<{ value: T } | null>(null);
  useEffect(() => { setHeld(null); }, [value]);
  useEffect(() => {
    if (held === null) return undefined;
    const timer = setTimeout(() => setHeld(null), HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  return [held, (sent: T) => setHeld({ value: sent })] as const;
}

export interface NumberShape {
  min: number;
  max: number;
  /** The decimals that it keeps. */
  digits: number;
  /** A last shaping of a value in range: to an even number, say. */
  fit?: (value: number) => number;
}

/** A value as the field keeps it: rounded to its decimals, within its range, and shaped. */
export function shape(value: number, field: NumberShape): number {
  const rounded = Number(value.toFixed(field.digits));
  const within = Math.min(field.max, Math.max(field.min, rounded));
  return field.fit ? field.fit(within) : within;
}

function parse(text: string): number | null {
  const trimmed = text.trim().replace(',', '.');
  if (trimmed === '') return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

export interface NumberFieldProps extends NumberShape {
  /** What it is called, for its label and for tests (`data-field`). */
  label: string;
  name: string;
  /** The value of the clips, or null where they differ. */
  value: number | null;
  /** What a double click on the label puts back. */
  fallback: number;
  /** How much a pixel of dragging the label changes it (ten times as much with Shift). */
  scrub: number;
  /** A slider beneath it, over a range that may be narrower than the field's. */
  slider?: { min: number; max: number; step: number };
  unit?: string;
  disabled?: boolean;
  title?: string;
  onCommit: (value: number) => void;
}

/** A number with a label to drag, and a slider where it has one. */
export function NumberField(props: NumberFieldProps) {
  const id = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [live, setLive] = useState<number | null>(null);
  const [scrubbing, setScrubbing] = useState(false);
  const [held, hold] = useHeld(props.value);
  const liveRef = useRef<number | null>(null);
  const scrub = useRef<{ x: number; start: number; pointer: number; moved: boolean } | null>(null);
  const sliding = useRef<'idle' | 'cancelled'>('idle');
  const slider = useRef<HTMLInputElement>(null);
  const now = useRef(props);
  now.current = props;
  const holdRef = useRef(hold);
  holdRef.current = hold;
  const show = (value: number | null) => { liveRef.current = value; setLive(value); };
  const send = (value: number | null) => {
    show(null); setDraft(null);
    if (value === null) return;
    const field = now.current;
    const next = shape(value, field);
    if (field.value !== null && next === field.value) return;
    holdRef.current(next);
    field.onCommit(next);
  };
  const revert = () => { scrub.current = null; setScrubbing(false); show(null); setDraft(null); };

  // Escape while the label is being dragged puts the value back.
  useEffect(() => {
    if (!scrubbing) return undefined;
    const key = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopPropagation();
      revert();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [scrubbing]);
  // A slider tells that it moves with `input`, and that it was let go (or stepped by a key) with `change`.
  const hasSlider = Boolean(props.slider);
  useEffect(() => {
    const element = slider.current;
    if (!element) return undefined;
    const changed = () => {
      if (sliding.current === 'cancelled') { sliding.current = 'idle'; show(null); return; }
      send(Number(element.value));
    };
    element.addEventListener('change', changed);
    return () => element.removeEventListener('change', changed);
  }, [hasSlider]);

  const shown = live ?? held?.value ?? props.value;
  const text = draft ?? (shown === null ? '' : shown.toFixed(props.digits));
  const fieldKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      if (draft !== null) send(parse(draft));
    } else if (event.key === 'Escape' && (draft !== null || live !== null)) {
      event.preventDefault(); event.stopPropagation();
      revert();
    }
  };
  const labelDown = (event: PointerEvent<HTMLElement>) => {
    if (props.disabled || event.button !== 0) return;
    scrub.current = { x: event.clientX, start: props.value ?? props.fallback, pointer: event.pointerId, moved: false };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };
  // The number box is a virtual slider until it is written in: a press that does not drag goes in to write.
  const boxDown = (event: PointerEvent<HTMLInputElement>) => {
    if (props.disabled || event.button !== 0 || document.activeElement === event.currentTarget) return;
    event.preventDefault();
    labelDown(event);
  };
  const boxUp = (event: PointerEvent<HTMLInputElement>) => {
    const drag = scrub.current;
    if (!drag || drag.pointer !== event.pointerId) return;
    const element = event.currentTarget;
    labelUp(event);
    if (!drag.moved) { element.focus(); element.select(); }
  };
  const labelMove = (event: PointerEvent<HTMLElement>) => {
    const drag = scrub.current;
    if (!drag || drag.pointer !== event.pointerId) return;
    const by = event.clientX - drag.x;
    if (!drag.moved && Math.abs(by) < 3) return;
    if (!drag.moved) { drag.moved = true; setScrubbing(true); }
    show(shape(drag.start + by * props.scrub * (event.shiftKey ? 10 : 1), props));
  };
  const labelUp = (event: PointerEvent<HTMLElement>) => {
    const drag = scrub.current;
    if (!drag || drag.pointer !== event.pointerId) return;
    scrub.current = null; setScrubbing(false);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (drag.moved) { event.preventDefault(); send(liveRef.current); }
  };
  return <div className="inspector-row" data-field={props.name} data-mixed={props.value === null || undefined} title={props.title}>
    <label htmlFor={id} className="inspector-label" data-disabled={props.disabled || undefined}
      onPointerDown={labelDown} onPointerMove={labelMove} onPointerUp={labelUp} onPointerCancel={revert}
      onDoubleClick={event => { event.preventDefault(); if (!props.disabled) send(props.fallback); }}>{props.label}</label>
    <span className="inspector-value">
      <input id={id} className="field inspector-number" type="text" inputMode="decimal" autoComplete="off" spellCheck={false}
        value={text} placeholder={shown === null ? MIXED : undefined} disabled={props.disabled}
        onPointerDown={boxDown} onPointerMove={labelMove} onPointerUp={boxUp} onPointerCancel={revert}
        onChange={event => setDraft(event.target.value)} onKeyDown={fieldKey}
        onBlur={() => { if (draft !== null) send(parse(draft)); }} />
      {props.unit && <span className="inspector-unit" aria-hidden="true">{props.unit}</span>}
    </span>
    {props.slider && <input ref={slider} className="inspector-slider" type="range" aria-label={props.label} disabled={props.disabled}
      min={props.slider.min} max={props.slider.max} step={props.slider.step} value={shown ?? props.fallback}
      onPointerDown={() => { sliding.current = 'idle'; }}
      onChange={event => {
        if (event.nativeEvent.type === 'change' || sliding.current === 'cancelled') return;
        show(Number(event.target.value));
      }}
      onKeyDown={event => {
        if (event.key === 'Escape' && live !== null) {
          event.preventDefault(); event.stopPropagation();
          sliding.current = 'cancelled'; revert();
        } else if (event.key !== 'Escape') sliding.current = 'idle';
      }} />}
  </div>;
}

/** A line of text, or lines of it: sent when Enter is pressed (Shift+Enter starts a new line) or the field is left. */
export function TextField({ label, name, value, multiline = false, allowEmpty = false, disabled, onCommit }: {
  label: string; name: string; value: string | null; multiline?: boolean; allowEmpty?: boolean; disabled?: boolean; onCommit: (value: string) => void;
}) {
  const id = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [held, hold] = useHeld(value);
  const send = () => {
    if (draft === null) return;
    setDraft(null);
    if ((!allowEmpty && draft.trim() === '') || draft === value) return;
    hold(draft);
    onCommit(draft);
  };
  const key = (event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); send(); }
    else if (event.key === 'Escape' && draft !== null) { event.preventDefault(); event.stopPropagation(); setDraft(null); }
  };
  const common = {
    id, className: 'field inspector-text', value: draft ?? held?.value ?? value ?? '', placeholder: value === null ? MIXED : undefined, disabled,
    spellCheck: false, onKeyDown: key, onBlur: send,
  };
  return <div className="inspector-row inspector-row-wide" data-field={name} data-mixed={value === null || undefined}>
    <label htmlFor={id} className="inspector-label">{label}</label>
    {multiline ? <textarea {...common} rows={3} onChange={event => setDraft(event.target.value)} />
      : <input {...common} type="text" autoComplete="off" onChange={event => setDraft(event.target.value)} />}
  </div>;
}

/** A colour of `#rrggbbaa`: its red, green and blue are chosen, its alpha kept. Sent when the picker is closed. */
export function ColorField({ label, name, value, onCommit }: { label: string; name: string; value: string | null; onCommit: (value: string) => void }) {
  const id = useId();
  const [live, setLive] = useState<string | null>(null);
  const [held, hold] = useHeld(value);
  const input = useRef<HTMLInputElement>(null);
  const now = useRef({ value, onCommit, hold });
  now.current = { value, onCommit, hold };
  useEffect(() => {
    const element = input.current;
    if (!element) return undefined;
    const changed = () => {
      setLive(null);
      const { value: was, onCommit: commit, hold: keep } = now.current;
      const next = `${element.value.toLowerCase()}${was?.slice(7, 9) || 'ff'}`;
      if (next !== was) { keep(next); commit(next); }
    };
    element.addEventListener('change', changed);
    return () => element.removeEventListener('change', changed);
  }, []);
  const color = held?.value ?? value;
  return <div className="inspector-row" data-field={name} data-mixed={color === null || undefined}>
    <label htmlFor={id} className="inspector-label">{label}</label>
    <span className="inspector-value">
      <input ref={input} id={id} className="inspector-color" type="color" value={live ?? color?.slice(0, 7) ?? '#000000'}
        onChange={event => { if (event.nativeEvent.type !== 'change') setLive(event.target.value); }}
        onKeyDown={event => { if (event.key === 'Escape' && live !== null) { event.stopPropagation(); setLive(null); } }} />
      <span className="inspector-unit mono">{(live ?? color?.slice(0, 7) ?? MIXED).toUpperCase()}</span>
    </span>
  </div>;
}

/** One of a few choices, sent as it is chosen. */
export function ChoiceField<T extends string>({ label, name, value, options, disabled, describedBy, onCommit }: {
  label: string; name: string; value: T | null; options: readonly { value: T; label: string }[]; disabled?: boolean;
  /** The id of words that say more of it: why it cannot be chosen, say. */
  describedBy?: string;
  onCommit: (value: T) => void;
}) {
  const id = useId();
  return <div className="inspector-row" data-field={name} data-mixed={value === null || undefined}>
    <label htmlFor={id} className="inspector-label">{label}</label>
    <select id={id} className="field inspector-select" value={value ?? ''} disabled={disabled} aria-describedby={describedBy}
      onChange={event => { if (event.target.value) onCommit(event.target.value as T); }}>
      {value === null && <option value="">{MIXED}</option>}
      {options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </select>
  </div>;
}

/** A row of buttons, one pressed: as DaVinci Resolve shows the alignment of a title or a transition. */
export function SegmentField<T extends string>({ label, name, value, options, onCommit }: {
  label: string; name: string; value: T | null; options: readonly { value: T; label: string; icon?: ReactNode }[]; onCommit: (value: T) => void;
}) {
  return <div className="inspector-row" data-field={name} data-mixed={value === null || undefined}>
    <span className="inspector-label">{label}</span>
    <span className="inspector-segments" role="group" aria-label={label}>
      {options.map(option => <button key={option.value} type="button" className="icon-btn" aria-pressed={value === option.value}
        aria-label={option.label} title={option.label} onClick={() => { if (value !== option.value) onCommit(option.value); }}>{option.icon ?? option.label}</button>)}
    </span>
  </div>;
}
