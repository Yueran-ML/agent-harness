import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { clamp, dbToPosition, positionToDb, FADER_GAIN } from './taper';
import { formatDb, type Modifiers } from './mixerModel';

interface Props {
  label: string; value: number; disabled?: boolean; readOnly?: boolean; hasLane?: boolean; valueLabel?: string;
  /** VCA contribution affects the cap/readout, while all changes still address this fader's base. */
  displayOffset?: number;
  onStart: (modifiers: Modifiers) => boolean; onChange: (value: number) => void; onEnd: () => void; onCancel: () => void;
  onCommit: (value: number, modifiers?: Modifiers) => void;
  /** The owner can reject an old burst after an external revision or session change. */
  isActive?: () => boolean;
}
/** RM20 pp.3688,3886: only the cap drags, reset at unity, fine drag and wheel (C-F1–4). */
export function Fader(props: Props) {
  const { label, value, disabled, readOnly, hasLane, onStart, onChange, onEnd, onCancel, onCommit } = props;
  const contribution = props.displayOffset ?? 0;
  const offset = Number.isFinite(contribution) ? contribution : 0;
  const displayed = value <= -100 || contribution === -Infinity ? -100 : value + offset;
  const now = useRef(props); now.current = props;
  const well = useRef<HTMLDivElement>(null);
  const cap = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; db: number; offset: number; height: number } | null>(null);
  const wheel = useRef<{ value: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const [editing, setEditing] = useState(false), [text, setText] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const valueButton = useRef<HTMLButtonElement>(null), editorOpener = useRef<HTMLElement | null>(null), wasEditing = useRef(false);
  const inputModifiers = useRef<Modifiers | undefined>(undefined);
  const cancel = () => { drag.current = null; if (wheel.current) clearTimeout(wheel.current.timer); wheel.current = null; now.current.onCancel(); };
  useEffect(() => { window.addEventListener('blur', cancel); return () => { window.removeEventListener('blur', cancel); cancel(); }; }, []);
  useEffect(() => {
    const element = cap.current;
    if (!element) return;
    const turn = (event: WheelEvent) => {
      const current = now.current;
      if (current.disabled || current.readOnly || drag.current || !event.deltaY) return;
      // React 19 delegates wheel with passive:true. Only this editable cap consumes scrolling;
      // native passive:false is required to stop its overflow:auto parent moving beneath it.
      event.preventDefault(); event.stopPropagation();
      if (wheel.current && current.isActive?.() === false) { clearTimeout(wheel.current.timer); wheel.current = null; }
      if (!wheel.current && !current.onStart(event)) return;
      const previous = wheel.current?.value ?? current.value;
      if (wheel.current) clearTimeout(wheel.current.timer);
      const next = previous + (event.deltaY < 0 ? 1 : -1) * (event.shiftKey ? .1 : .5);
      current.onChange(next);
      wheel.current = { value: next, timer: setTimeout(() => { wheel.current = null; now.current.onEnd(); }, 500) };
    };
    element.addEventListener('wheel', turn, { passive: false });
    return () => element.removeEventListener('wheel', turn);
  }, []);
  useEffect(() => {
    if (editing) { input.current?.focus(); input.current?.select(); }
    else if (wasEditing.current) (editorOpener.current?.isConnected ? editorOpener.current : valueButton.current)?.focus();
    wasEditing.current = editing;
  }, [editing]);
  const editValue = (opener: HTMLElement, modifiers: Modifiers) => {
    if (disabled || readOnly) return;
    editorOpener.current = opener;
    inputModifiers.current = modifiers.altKey || modifiers.ctrlKey || modifiers.shiftKey ? { altKey: modifiers.altKey, ctrlKey: modifiers.ctrlKey, shiftKey: modifiers.shiftKey } : undefined;
    setText(formatDb(displayed)); setEditing(true);
  };
  const finish = () => { if (drag.current) { drag.current = null; onEnd(); } };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const d = drag.current; if (!d) return;
    const position = dbToPosition(d.db + offset) + (d.y - event.clientY) / d.height * (event.shiftKey ? .1 : 1);
    // Continue the shared taper's endpoint slopes for the requested delta. Each strip clamps
    // independently, so the grabbed cap reaching its stop does not stop the other selected caps.
    const next = position < 0 ? -100 + position * (positionToDb(.001) + 100) / .001
      : position > 1 ? 10 + (position - 1) * (10 - positionToDb(.999)) / .001 : positionToDb(position);
    const requested = next - offset + d.offset;
    onChange(event.shiftKey ? Math.round(requested * 10) / 10 : requested);
  };
  const save = () => {
    const parsed = ['-inf', '-∞', '−∞'].includes(text.trim().toLowerCase()) ? -100 : Number(text);
    if (text.trim() && Number.isFinite(parsed)) {
      const base = parsed <= -100 ? -100 : parsed - offset;
      if (inputModifiers.current) onCommit(clamp(base, ...FADER_GAIN), inputModifiers.current); else onCommit(clamp(base, ...FADER_GAIN));
      setEditing(false);
    }
  };
  return <div className="audio-fader" onKeyDown={event => {
    if (event.target instanceof HTMLInputElement || event.nativeEvent.isComposing || event.keyCode === 229) event.stopPropagation();
    if (!event.nativeEvent.isComposing && event.keyCode !== 229 && event.key === 'Escape') { event.stopPropagation(); event.preventDefault(); cancel(); setEditing(false); }
  }}>
    {editing ? <input ref={input} aria-label={props.valueLabel ?? `${label} value`} value={text} onChange={event => setText(event.target.value)}
      onBlur={() => setEditing(false)} onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); save(); } }} />
      : <button ref={valueButton} className="audio-fader-value" type="button" aria-label={props.valueLabel ?? `${label} value`} disabled={disabled || readOnly}
        onDoubleClick={event => editValue(event.currentTarget, event)}
        onKeyDown={event => { if (!event.nativeEvent.isComposing && ['Enter', 'F2'].includes(event.key)) { event.preventDefault(); event.stopPropagation(); editValue(event.currentTarget, event); } }}>{formatDb(displayed)}</button>}
    <div ref={well} className="audio-fader-well">
      {[10, 0, -10, -20, -40, -60].map(db => <span aria-hidden="true" key={db} className="audio-fader-tick" style={{ bottom: `${dbToPosition(db) * 100}%` }}>{db}</span>)}
      <div ref={cap} role="slider" aria-label={label} aria-orientation="vertical" aria-valuemin={Math.min(-100,-100+offset)} aria-valuemax={Math.max(10,10+offset)} aria-valuenow={displayed} aria-valuetext={`${formatDb(displayed)} dB`}
        aria-disabled={disabled || undefined} aria-readonly={readOnly || undefined} tabIndex={disabled ? -1 : 0} className="audio-fader-cap"
        style={{ bottom: `${dbToPosition(displayed) * 100}%` }}
        onKeyDown={event => {
          if (event.nativeEvent.isComposing || event.keyCode === 229) return;
          if (['Enter', 'F2'].includes(event.key)) { event.preventDefault(); event.stopPropagation(); editValue(event.currentTarget, event); return; }
          if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault(); event.stopPropagation(); if (disabled || readOnly || drag.current) return;
          if (wheel.current && now.current.isActive?.() === false) { clearTimeout(wheel.current.timer); wheel.current = null; }
          if (!wheel.current && !onStart(event)) return;
          const previous = wheel.current?.value ?? value;
          if (wheel.current) clearTimeout(wheel.current.timer);
          const next = event.key === 'Home' ? -100 : event.key === 'End' ? 10 : previous + (event.key === 'ArrowUp' ? 1 : -1) * (event.shiftKey ? .1 : .5);
          onChange(next); wheel.current = { value: next, timer: setTimeout(() => { wheel.current = null; now.current.onEnd(); }, 500) };
        }} onKeyUp={event => {
          if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key) && wheel.current) { event.preventDefault(); event.stopPropagation(); clearTimeout(wheel.current.timer); wheel.current = null; onEnd(); }
        }}
        onPointerDown={event => {
          if (event.button !== 0 || disabled || readOnly) return;
          event.preventDefault(); event.stopPropagation(); event.currentTarget.focus();
          const burst = wheel.current, continuing = burst !== null && now.current.isActive?.() !== false;
          if (burst) clearTimeout(burst.timer);
          wheel.current = null;
          // A wheel burst and immediate cap drag are one continuous gesture, with the same
          // original group snapshot/undo. Do not end the async owner then try to restart it.
          if (!continuing && !onStart(event)) return;
          const db = continuing ? clamp(burst.value, ...FADER_GAIN) : value;
          drag.current = { y: event.clientY, db, offset: (continuing ? burst.value - db : 0) + (db + offset - clamp(db + offset, ...FADER_GAIN)), height: well.current?.getBoundingClientRect().height || 160 };
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }} onPointerMove={move} onPointerUp={event => { finish(); event.currentTarget.releasePointerCapture?.(event.pointerId); }} onPointerCancel={cancel}
        onLostPointerCapture={() => { if (drag.current) cancel(); }}
        onDoubleClick={event => { event.stopPropagation(); if (!disabled && !readOnly && !hasLane) onCommit(0, event); }} />
    </div>
  </div>;
}
