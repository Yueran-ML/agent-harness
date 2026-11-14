import { useRef } from 'react';
/** Shared layout divider; these sizes are view state, never project edits. */
export function Separator({ label, vertical = false, direction = 1, value, min, max, onChange }: {
  label: string; vertical?: boolean; direction?: 1 | -1; value: number; min: number; max: number; onChange: (value: number) => void;
}) {
  const drag = useRef<{ at: number; value: number } | null>(null);
  const set = (next: number) => onChange(Math.max(min, Math.min(max, next)));
  return <div className="fairlight-separator" role="separator" tabIndex={0} aria-label={label} aria-orientation={vertical ? 'vertical' : 'horizontal'} aria-valuenow={value} aria-valuemin={min} aria-valuemax={max}
    onPointerDown={event => { if (event.button) return; event.preventDefault(); event.stopPropagation(); event.currentTarget.setPointerCapture?.(event.pointerId); drag.current = { at: vertical ? event.clientX : event.clientY, value }; }}
    onPointerMove={event => { if (drag.current) set(drag.current.value + direction * ((vertical ? event.clientX : event.clientY) - drag.current.at)); }}
    onPointerUp={event => { drag.current = null; event.currentTarget.releasePointerCapture?.(event.pointerId); }} onPointerCancel={() => { drag.current = null; }}
    onKeyDown={event => { if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) { event.preventDefault(); event.stopPropagation(); set(value + direction * (['ArrowLeft', 'ArrowUp'].includes(event.key) ? -16 : 16)); } }} />;
}
