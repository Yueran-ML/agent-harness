import { useEffect, useRef, useState } from 'react';
import { Sheet } from '../../app/Sheet';
import type { BusFormat, Pan } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { Knob } from './Knob';
import { defaultPan, outputsOf } from './routingModel';
import type { ProcessorProps } from './useProcessor';
import { useRoutingParameter, type RoutingParameter } from './useRoutingParameter';
import './routingControls.css';
const PAN_SIZE = 200, PAN_RADIUS = 75;

export function panCapabilities(formats: readonly BusFormat[]) {
  return { rear: formats.some(format => ['4.0', '5.0', '5.1', '7.0', '7.1'].includes(format)), lfe: formats.some(format => format === '5.1' || format === '7.1') };
}
function PanSurface({ control, prefix, rear, lfe, thumbnail = false, onOpen }: {
  control: RoutingParameter; prefix: string; rear: boolean; lfe: boolean; thumbnail?: boolean; onOpen?: () => void;
}) {
  const t = useFairlightText(), svg = useRef<SVGSVGElement>(null);
  const owner = useRef({});
  const drag = useRef<{ x: number; y: number; fields: string[]; started: boolean; width: number; height: number } | null>(null);
  const click = useRef<ReturnType<typeof setTimeout> | null>(null);
  const now = useRef(control); now.current = control;
  const stopClick = () => { if (click.current) clearTimeout(click.current); click.current = null; };
  useEffect(() => () => { stopClick(); if (drag.current?.started) now.current.cancel(drag.current.fields, owner.current); }, []);
  const field = (name: string) => prefix + name;
  const x = control.reading(field('x')), y = rear ? control.reading(field('y')) : 0;
  const spread = control.reading(field('spread')) / 99 * 30;
  const enabled = control.reading(field('on')) >= .5;
  return <svg ref={svg} viewBox="0 0 200 200" className={thumbnail ? 'pan-surface pan-thumbnail' : 'pan-surface'} role="slider" tabIndex={0}
    aria-label={`${t('Pan')} ${control.strip.name}`} aria-valuemin={-100} aria-valuemax={100} aria-valuenow={Math.round(x * 100)} aria-valuetext={`${Math.round(x * 100)}, ${Math.round(y * 100)}`} data-off={!enabled || undefined}
    onContextMenu={event => { event.preventDefault(); control.show(field('x')); }}
    onPointerDown={event => {
      if (event.button !== 0 || control.strip.locked) return;
      event.preventDefault(); event.stopPropagation(); event.currentTarget.focus(); stopClick();
      const fields = event.ctrlKey && control.strip.channels > 1 ? [field('spread')] : [field('x'), ...(rear ? [field('y')] : [])];
      const rect = event.currentTarget.getBoundingClientRect();
      drag.current = { x: event.clientX, y: event.clientY, fields, started: false, width: rect.width || 200, height: rect.height || 200 };
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }} onPointerMove={event => {
      const d = drag.current; if (!d) return;
      const dx = event.clientX - d.x, dy = event.clientY - d.y;
      if (!d.started) { if (Math.hypot(dx, dy) < 3) return; if (!control.start(d.fields, owner.current)) { drag.current = null; return; } d.started = true; }
      if (!control.active(d.fields, owner.current)) return;
      const initial = control.snapshot(), clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
      if (d.fields[0] === field('spread')) control.change({ [field('spread')]: clamp(initial[field('spread')] + dx / d.width * 98, 1, 99) }, owner.current);
      else control.change({ [field('x')]: event.shiftKey ? initial[field('x')] : clamp(initial[field('x')] + dx / d.width * PAN_SIZE / PAN_RADIUS, -1, 1),
        ...(rear ? { [field('y')]: clamp(initial[field('y')] - dy / d.height * PAN_SIZE / PAN_RADIUS, -1, 1) } : {}) }, owner.current);
    }} onPointerUp={event => {
      const d = drag.current; drag.current = null; event.currentTarget.releasePointerCapture?.(event.pointerId);
      if (!d) return; if (d.started) control.end(d.fields, owner.current);
      else if (thumbnail) { stopClick(); click.current = setTimeout(() => { click.current = null; now.current.commit({ [field('on')]: enabled ? 0 : 1 }); }, 500); }
    }} onPointerCancel={() => { if (drag.current?.started) control.cancel(drag.current.fields, owner.current); drag.current = null; }}
    onLostPointerCapture={() => { if (drag.current?.started) control.cancel(drag.current.fields, owner.current); drag.current = null; }}
    onDoubleClick={event => { event.preventDefault(); stopClick(); if (!event.altKey) onOpen?.(); }}
    onKeyDown={event => {
      if (event.nativeEvent.isComposing) return;
      if (event.key === 'Escape') { stopClick(); if (drag.current?.started) { event.preventDefault(); event.stopPropagation(); control.cancel(drag.current.fields, owner.current); drag.current = null; } }
      if (event.key === 'Enter') { event.preventDefault(); onOpen?.(); }
      if (event.key === ' ') { event.preventDefault(); control.commit({ [field('on')]: enabled ? 0 : 1 }); }
      if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
        event.preventDefault(); const vertical = event.key === 'ArrowUp' || event.key === 'ArrowDown'; if (vertical && !rear) return;
        const name = field(vertical ? 'y' : 'x'), sign = event.key === 'ArrowUp' || event.key === 'ArrowRight' ? 1 : -1;
        control.commit({ [name]: Math.max(-1, Math.min(1, control.reading(name) + sign * (event.shiftKey ? .001 : .01))) });
      }
    }}>
    <rect x="1" y="1" width="198" height="198" rx="8" /><path d="M100 10V190M10 100H190" className="pan-guide" />
    <text x="12" y={rear ? 20 : 105}>L</text><text x="178" y={rear ? 20 : 105}>R</text>{rear && <><text x="10" y="187">Ls</text><text x="175" y="187">Rs</text></>}
    <g transform={`translate(${100 + x * PAN_RADIUS} ${100 - y * PAN_RADIUS}) rotate(${control.reading(field('rotate'))})`}>
      <path d={`M${-control.reading(field('divergence')) / 2} 0H${control.reading(field('divergence')) / 2}`} className="pan-divergence" />
      {control.strip.channels > 1 && <><circle cx={-spread} r="4" /><circle cx={spread} r="4" /></>}<circle r="7" className="pan-position" />
    </g>
    {lfe && control.reading(field('lfe_on')) >= .5 && <path d={`M10 195H${10 + Math.max(0, (control.reading(field('lfe_db')) + 100) / 110) * 180}`} className="pan-boom" />}
  </svg>;
}

export function PanFields({ control, prefix = '', formats }: { control: RoutingParameter; prefix?: string; formats: BusFormat[] }) {
  const t = useFairlightText(), { rear, lfe } = panCapabilities(formats);
  const owners = useRef(new Map<string, object>());
  const knobs = [
    ['x', 'Left/Right', -100, 100, 100, false], ['y', 'Front/Back', -100, 100, 100, !rear],
    ['rotate', 'Rotate', -180, 180, 1, false], ['spread', 'Spread', 1, 99, 1, control.strip.channels < 2],
    ['divergence', 'Divergence', 0, 100, 1, false], ['lfe_db', 'Boom Level', -100, 10, 1, !lfe],
  ] as const;
  return <div className="pan-editor" data-audio-owner={control.strip.key}>
    <PanSurface control={control} prefix={prefix} rear={rear} lfe={lfe} />
    <div className="pan-fields">{(['on', 'lfe_on', 'lfe_pre'] as const).map((name, index) => {
      const field = prefix + name;
      return <label key={name}><input type="checkbox" checked={control.reading(field) >= .5} disabled={(index > 0 && !lfe) || !control.writable(field)} onChange={event => control.commit({ [field]: Number(event.target.checked) })} />{t(index === 0 ? 'Pan Enable' : index === 1 ? 'Boom On' : 'Boom Pre')}</label>;
    })}{knobs.map(([name, label, min, max, scale, disabled]) => {
      const field = prefix + name, parameter = control.parameter(field);
      if (!owners.current.has(field)) owners.current.set(field, {});
      const owner = owners.current.get(field)!;
      return <Knob key={name} path={control.path(field)} label={t(label)} value={control.reading(field) * scale} min={Math.max(min, (parameter?.display_min ?? parameter?.min ?? min / scale) * scale)} max={Math.min(max, (parameter?.display_max ?? parameter?.max ?? max / scale) * scale)}
        fallback={Number(defaultPan()[name]) * scale} disabled={disabled || !control.writable(field)} unit={name === 'rotate' ? '°' : name === 'lfe_db' ? 'dB' : undefined}
        onStart={() => control.start([field], owner)} isActive={() => control.active([field], owner)} onChange={value => control.change({ [field]: value / scale }, owner)}
        onEnd={() => control.end([field], owner)} onCancel={() => control.cancel([field], owner)} onCommit={value => control.commit({ [field]: value / scale })} />;
    })}</div>
  </div>;
}
export function PanControl({ context, stripKey }: { context: ProcessorProps; stripKey: string }) {
  const control = useRoutingParameter(context, stripKey), t = useFairlightText();
  const [open, setOpen] = useState(false);
  const formats = outputsOf(context.timeline, control.strip).filter(output => output.on).flatMap(output => context.timeline.buses?.find(bus => bus.id === output.bus)?.format ?? []);
  if (!formats.length && control.strip.kind === 'bus') formats.push(context.timeline.buses!.find(bus => bus.id === control.strip.id)!.format);
  const capabilities = panCapabilities(formats);
  return <><PanSurface control={control} prefix="" {...capabilities} thumbnail onOpen={() => setOpen(true)} />
    {open && <Sheet title={`${t('Pan')} · ${control.strip.name}`} onClose={() => { control.cancel(); setOpen(false); }}><PanFields control={control} formats={formats} />{control.problem && <p role="alert">{control.problem}</p>}</Sheet>}
    {!open && control.problem && <span role="alert">{control.problem}</span>}</>;
}
