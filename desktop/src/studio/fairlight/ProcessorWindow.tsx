import { useRef, useState } from 'react';
import { useOverlay } from '../../app/overlay';
import { EqPanel } from '../audio/EqPanel';
import { DynamicsPanel } from '../audio/DynamicsPanel';
import type { ProcessorProps } from '../audio/useProcessor';
import type { ProcessorModule } from '../audio/processorModel';
import { useFairlightText } from './words';

export function ProcessorWindow({ context, module, name, compact, order, onClose, onFront }: {
  context: ProcessorProps; module: ProcessorModule; name: string; compact: boolean; order: number; onClose: () => void; onFront: () => void;
}) {
  useOverlay();
  const t = useFairlightText(), [position, setPosition] = useState({ x: 80 + order * 24, y: 100 + order * 24 });
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const title = `${module === 'eq' ? 'EQ' : t('Dynamics')} · ${name}`;
  return <aside role={compact ? 'complementary' : 'dialog'} aria-label={compact ? `${t('Track processor')} · ${name}` : title}
    className={compact ? 'edit-processor-inspector' : 'processor-window'} style={compact ? undefined : { left: position.x, top: position.y, zIndex: 70 + order }} onPointerDown={onFront}>
    <header onPointerDown={event => {
      if (compact || event.target instanceof HTMLButtonElement) return;
      drag.current = { x: event.clientX, y: event.clientY, left: position.x, top: position.y }; event.currentTarget.setPointerCapture?.(event.pointerId);
    }} onPointerMove={event => { const d = drag.current; if (d) setPosition({ x: Math.max(0, d.left + event.clientX - d.x), y: Math.max(0, d.top + event.clientY - d.y) }); }}
      onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; }}><strong>{title}</strong><button type="button" aria-label={t('Close processor')} onClick={onClose}>×</button></header>
    {module === 'eq' ? <EqPanel {...context} /> : <DynamicsPanel {...context} />}
  </aside>;
}
