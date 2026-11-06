import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { ownerOf } from '../fairlight/automation';
import { useEffect, useRef, useState } from 'react';
import { clamp } from './taper';
import { useFairlightText } from '../fairlight/words';

export interface KnobProps {
  /** Existing model automation path; the panel supplies the owner separately. */
  path?: string;
  label: string; value: number; min: number; max: number; step?: number; fallback: number; unit?: string; disabled?: boolean; logarithmic?: boolean;
  onStart: () => boolean; onChange: (value: number) => void; onEnd: () => void; onCancel: () => void; onCommit: (value: number) => void; isActive?: () => boolean;
}
/** Numeric Alt/Shift is precision, never a batch-target modifier. One wheel burst is one gesture. */
export function Knob(props: KnobProps) {
  const t = useFairlightText(), automation=useAutomationTouch(), [menu,setMenu]=useState(false);
  const { label, value, min, max, fallback, disabled, onStart, onChange, onEnd, onCancel, onCommit, step = .1 } = props;
  const now = useRef(props); now.current = props;
  const cap = useRef<HTMLSpanElement>(null), drag = useRef<{ y: number; value: number } | null>(null);
  const scrub = useRef<{ x: number; value: number } | null>(null);
  const wheel = useRef<{ value: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const [text, setText] = useState(String(value)), [editing, setEditing] = useState(false);
  const owner=cap.current?.closest<HTMLElement>('[data-audio-owner]')?.dataset.audioOwner;
  const status=owner&&props.path&&!owner.startsWith('clip:')?automation?.status({of:ownerOf(owner),path:props.path}):null;
  useEffect(() => { if (!editing) setText(String(Number(value.toFixed(3)))); }, [value, editing]);
  const cancel = () => { drag.current = null; scrub.current = null; if (wheel.current) clearTimeout(wheel.current.timer); wheel.current = null; now.current.onCancel(); setEditing(false); setText(String(now.current.value)); };
  useEffect(() => { window.addEventListener('blur', cancel); return () => { window.removeEventListener('blur', cancel); cancel(); }; }, []);
  useEffect(() => {
    const element = cap.current; if (!element) return;
    const turn = (event: WheelEvent) => {
      const p = now.current; if (p.disabled || drag.current || !event.deltaY) return;
      event.preventDefault(); event.stopPropagation();
      if (wheel.current && p.isActive?.() === false) { clearTimeout(wheel.current.timer); wheel.current = null; }
      if (!wheel.current && !p.onStart()) return;
      element.focus({ preventScroll: true });
      const previous = wheel.current?.value ?? p.value;
      if (wheel.current) clearTimeout(wheel.current.timer);
      const next = clamp(previous + (event.deltaY < 0 ? 1 : -1) * (p.step ?? .1) * (event.altKey || event.shiftKey ? .1 : 1), p.min, p.max);
      p.onChange(next); wheel.current = { value: next, timer: setTimeout(() => { wheel.current = null; now.current.onEnd(); }, 500) };
    };
    element.addEventListener('wheel', turn, { passive: false }); return () => element.removeEventListener('wheel', turn);
  }, []);
  return <label className="audio-knob" data-audio-path={props.path} onContextMenu={event=>{const key=cap.current?.closest<HTMLElement>('[data-audio-owner]')?.dataset.audioOwner;if(automation&&props.path&&key&&!key.startsWith('clip:')){event.preventDefault();event.stopPropagation();setMenu(true);}}} onKeyDown={event => { event.stopPropagation(); if (event.key === 'Escape') { event.preventDefault(); cancel(); } }}>
    {menu&&<span role="menu"><button type="button" onClick={()=>{const key=cap.current?.closest<HTMLElement>('[data-audio-owner]')?.dataset.audioOwner;if(key&&props.path)automation?.show({of:ownerOf(key),path:props.path});setMenu(false);}}>{t('Show automation in timeline')}</button><button type="button" onClick={()=>setMenu(false)}>{t('Cancel')}</button></span>}
    <span onDoubleClick={event => { event.preventDefault(); if (!disabled) onCommit(fallback); }}>{label}</span>
    <span ref={cap} role="slider" aria-label={label} aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} aria-disabled={disabled || undefined} tabIndex={disabled ? -1 : 0}
      className="audio-knob-cap" data-automation={status??undefined} onPointerDown={event => {
        if (event.button !== 0 || disabled) return; event.preventDefault(); event.stopPropagation();
        const burst = wheel.current, continuing = burst !== null && props.isActive?.() !== false;
        if (burst) clearTimeout(burst.timer); wheel.current = null;
        if (!continuing && !onStart()) return;
        event.currentTarget.focus({ preventScroll: true });
        drag.current = { y: event.clientY, value: continuing ? burst.value : value }; event.currentTarget.setPointerCapture?.(event.pointerId);
      }} onPointerMove={event => { const d = drag.current; if (!d || props.isActive?.() === false) return;
        const delta = (d.y - event.clientY) * (event.altKey || event.shiftKey ? .1 : 1);
        onChange(clamp(props.logarithmic ? d.value * Math.pow(1.02, delta) : d.value + delta * step, min, max));
      }} onPointerUp={event => { if (drag.current) { drag.current = null; onEnd(); } event.currentTarget.releasePointerCapture?.(event.pointerId); }}
      onPointerCancel={cancel} onLostPointerCapture={() => { if (drag.current) cancel(); }} onDoubleClick={() => { if (!disabled) onCommit(fallback); }}
      onKeyDown={event => { if (disabled || !['ArrowUp', 'ArrowDown'].includes(event.key)) return; event.preventDefault(); onCommit(clamp(value + (event.key === 'ArrowUp' ? 1 : -1) * step * (event.altKey || event.shiftKey ? .1 : 1), min, max)); }}>◉</span>
    <input type="number" aria-label={`${label} ${t('value')}`} min={min} max={max} step="any" value={text} disabled={disabled}
      onPointerDown={event => {
        if (event.button !== 0 || disabled) return;
        const burst = wheel.current, continuing = burst !== null && props.isActive?.() !== false;
        if (burst) clearTimeout(burst.timer); wheel.current = null;
        if (!continuing && !onStart()) return;
        event.currentTarget.focus({ preventScroll: true });
        scrub.current = { x: event.clientX, value: continuing ? burst.value : value }; event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={event => { const d = scrub.current; if (!d || props.isActive?.() === false || Math.abs(event.clientX - d.x) < 3) return;
        event.preventDefault(); event.stopPropagation(); event.currentTarget.setPointerCapture?.(event.pointerId);
        const delta = (event.clientX - d.x) * (event.altKey || event.shiftKey ? .1 : 1);
        const next = clamp(props.logarithmic ? d.value * Math.pow(1.02, delta) : d.value + delta * step, min, max);
        setText(String(Number(next.toFixed(3)))); onChange(next);
      }} onPointerUp={event => { if (scrub.current) { scrub.current = null; onEnd(); setEditing(false); } event.currentTarget.releasePointerCapture?.(event.pointerId); }}
      onPointerCancel={cancel} onLostPointerCapture={() => { if (scrub.current) cancel(); }}
      onFocus={() => setEditing(true)} onChange={event => { setEditing(true); setText(event.target.value); }} onBlur={() => { if (scrub.current) cancel(); setEditing(false); setText(String(value)); }}
      onKeyDown={event => {
        if (['ArrowUp', 'ArrowDown'].includes(event.key) && !event.nativeEvent.isComposing) { event.preventDefault();
          const base = text.trim() && Number.isFinite(Number(text)) ? Number(text) : value;
          const next = clamp(base + (event.key === 'ArrowUp' ? 1 : -1) * step * (event.altKey || event.shiftKey ? .1 : 1), min, max);
          setText(String(next)); onCommit(next); return;
        }
        if (event.key === 'Enter' && !event.nativeEvent.isComposing && text.trim() && Number.isFinite(Number(text))) {
        event.preventDefault(); onCommit(clamp(Number(text), min, max)); setEditing(false);
      } }} />{props.unit && <small>{props.unit}</small>}
  </label>;
}
