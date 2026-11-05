import { useEffect, useRef, useState } from 'react';
import type { Dynamics, Equalizer } from '../engine';
import { CoveringMenu } from '../color/CoveringMenu';
import type { Menu } from '../color/ColorMenu';
import { useFairlightText } from '../fairlight/words';
import type { Modifiers } from './mixerModel';
import { frequencyPosition, targetKey, type ProcessorModule } from './processorModel';
import { useProcessorCurve } from './useProcessorCurve';
import type { ProcessorProps } from './useProcessor';
import { clamp } from './taper';
import { copyProcessor, pastedProcessor } from './ProcessorToolbar';

export function ProcessorThumbnail({ name, module, on, disabled, context, settings, onToggle, onOpen, onPaste, openOnClick = false, showCurve = true }: {
  name: string; module: ProcessorModule; on: boolean; disabled: boolean; context: ProcessorProps;
  settings: Equalizer | Dynamics; onPaste: (settings: Equalizer | Dynamics, modifiers: Modifiers) => void;
  onToggle: (modifiers: Modifiers) => void; onOpen: () => void;
  /** Edit opens its inspector; Fairlight uses the thumbnail as a bypass switch. */
  openOnClick?: boolean;
  showCurve?: boolean;
}) {
  const t = useFairlightText(), timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const session = context.session, timeline = context.timeline.id, owner = targetKey(context.target);
  const current = useRef({ session, timeline, owner, module, disabled, openOnClick, onToggle });
  current.current = { session, timeline, owner, module, disabled, openOnClick, onToggle };
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  }, [session, timeline, owner, module]);
  return <><button type="button" className="processor-thumbnail" aria-label={`${module === 'eq' ? 'EQ' : t('Dynamics')} ${name}`} aria-pressed={openOnClick ? undefined : on}
    onContextMenu={event => { event.preventDefault(); event.stopPropagation(); if (timer.current) clearTimeout(timer.current); timer.current = null;
      const saved = pastedProcessor(module, settings), modifiers = { altKey: event.altKey, ctrlKey: event.ctrlKey, shiftKey: event.shiftKey };
      setMenu({ x: event.clientX, y: event.clientY, label: `${module === 'eq' ? 'EQ' : t('Dynamics')} ${name}`, items: [
        { id: 'copy-processor', label: t('Copy'), run: () => copyProcessor(module, settings) },
        { id: 'paste-processor', label: t('Paste'), disabled: disabled || !saved, run: () => { if (saved) onPaste(saved, modifiers); } },
      ] });
    }}
    onClick={event => {
      if (timer.current) clearTimeout(timer.current);
      timer.current = null;
      if (openOnClick) { onOpen(); return; }
      if (disabled) return;
      if (event.detail > 1) { timer.current = null; return; }
      const modifiers = { altKey: event.altKey, ctrlKey: event.ctrlKey, shiftKey: event.shiftKey };
      if (event.detail === 0) onToggle(modifiers);
      // Leave the usual Windows double-click interval for the second click to cancel this write.
      else timer.current = setTimeout(() => {
        timer.current = null;
        const latest = current.current;
        // Revisions may advance while distinguishing a double click. Keep this action on its
        // original owner, but use that owner's current model, permissions and transaction callback.
        if (latest.session === session && latest.timeline === timeline && latest.owner === owner && latest.module === module
          && !latest.disabled && !latest.openOnClick) latest.onToggle(modifiers);
      }, 500);
    }} onDoubleClick={event => { event.preventDefault(); if (timer.current) clearTimeout(timer.current); timer.current = null; onOpen(); }}>
    {showCurve ? <ThumbnailCurve context={context} module={module} /> : <span>{module === 'eq' ? 'EQ' : 'DY'}</span>}
  </button>{menu && <CoveringMenu menu={menu} onClose={() => setMenu(null)} />}</>;
}
function ThumbnailCurve({ context, module }: { context: ProcessorProps; module: ProcessorModule }) {
  const { curve } = useProcessorCurve(context, module);
  return <svg viewBox="0 0 100 40" aria-hidden="true"><path d="M0 20H100" className="processor-grid" />
    {curve && <polyline fill="none" className="processor-response" points={curve.x.map((x, i) => module === 'eq'
      ? `${frequencyPosition(x) * 100},${20 - clamp(curve.y[i], -20, 20)}`
      : `${(x + 60) / 60 * 100},${-clamp(curve.y[i], -60, 0) / 60 * 40}`).join(' ')} />}
  </svg>;
}
