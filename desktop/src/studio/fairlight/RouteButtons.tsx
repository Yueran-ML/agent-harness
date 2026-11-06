import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useOverlay } from '../../app/overlay';
import type { ProcessorProps } from '../audio/useProcessor';
import { mixerStrips, operationTargets, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { targetOf } from '../audio/processorModel';
import { busChoices, outputsOf, routingChanges, type Routes } from '../audio/routingModel';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { useFairlightText } from './words';
import { useAutomationTouch } from './AutomationTouch';
import { laneValue } from './automation';
export function RouteButtons({ context, strip, field, onOpen }: {
  context: ProcessorProps;
  strip: MixerStrip;
  field: Routes;
  onOpen: () => void;
}) {
  const t = useFairlightText(), [open, setOpen] = useState(false), [problem, setProblem] = useState('');
  const automation = useAutomationTouch(), recording = Boolean(automation?.enabled && automation.recording);
  const [anchor, setAnchor] = useState({ x: 0, y: 0 });
  const trigger = useRef<HTMLButtonElement>(null), menu = useRef<HTMLDivElement>(null);
  useOverlay(open);
  useEffect(() => {
    if (!open)
      return;
    const outside = (event: PointerEvent) => {
      if (!menu.current?.contains(event.target as Node) && !trigger.current?.contains(event.target as Node))
        setOpen(false);
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  useEffect(() => setOpen(false), [context.session, context.timeline.id, strip.key]);
  const gesture = useAudioGesture({
    ...context, identity: `${context.timeline.id}/${strip.key}/${field}`, locked: strip.locked, onFailure: e => setProblem(String(e))
  });
  const choices = busChoices(context.timeline, targetOf(strip), field);
  const routes = field === 'outputs' ? outputsOf(context.timeline, strip) : strip.strip?.sends ?? [];
  const apply = (bus: string, action: 'add' | 'remove' | 'toggle', modifiers: Modifiers) => {
    const rows = operationTargets(mixerStrips(context.timeline), strip.key, context.selected ?? [], modifiers);
    if (recording) {
      if (action !== 'toggle')
        return;
      const path = `${field === 'sends' ? 'send' : 'output'}.${bus}.on`, value = read(path, Number(routes.find(row => row.bus === bus)?.on ?? true)) >= .5 ? 0 : 1;
      automation!.instantBatch(rows.filter(row => (field === 'outputs' ? outputsOf(context.timeline, row) : row.strip?.sends ?? []).some(route => route.bus === bus)).map(row => ({ target: { of: targetOf(row), path }, value })));
      return;
    }
    const owners = rows.map(targetOf);
    gesture.commit(routingChanges(context.timeline, owners, field, bus, action));
  };
  const read = (path: string, baseline: number) => automation?.reading(
    { of: targetOf(strip), path },
    laneValue(automation?.enabled !== false && strip.arm !== 'off' ? strip.lanes?.[path] : undefined, context.seconds ?? 0, baseline),
    { kind: path.endsWith('.on') ? 'switch' : 'continuous', scale: 'linear' }
  ) ?? laneValue(strip.arm !== 'off' ? strip.lanes?.[path] : undefined, context.seconds ?? 0, baseline);
  return <div className="strip-routes" onKeyDown={event => {
    if (event.key === 'Escape' && !event.nativeEvent.isComposing) {
      setOpen(false);
      event.stopPropagation();
    }
  }}>

    {routes.map(route => {
      const bus = context.timeline.buses?.find(b => b.id === route.bus);
      const path = `${field === 'sends' ? 'send' : 'output'}.${route.bus}.on`, on = read(path, Number(route.on)) >= .5;
      const level = 'level_db' in route ? read(`send.${route.bus}.level_db`, Number(route.level_db)) : null;
      return <span
        className="strip-route"
        key={route.bus}
        style={{ borderColor: bus?.color }}
        data-off={!on || undefined}
        title={bus?.name ?? route.bus}
      >

        <button
          type="button"
          className="route-switch"
          aria-label={`${t('Enable')} ${bus?.name}`}
          aria-pressed={on}
          disabled={strip.locked || !recording && automation?.enabled !== false && strip.arm !== 'off' && Boolean(strip.lanes?.[path])}
          onClick={event => apply(route.bus, 'toggle', event)}
        >●</button>

        <button
          type="button"
          className="route-name"
          onClick={field === 'sends' ? onOpen : undefined}
        >
          {bus?.name ?? route.bus}
          {level !== null ? ` ${level <= -100 ? 'OFF' : level.toFixed(1)}` : ''}
        </button>

        <button
          type="button"
          className="route-remove"
          aria-label={`${t('Remove')} ${bus?.name}`}
          disabled={strip.locked || recording}
          onClick={event => apply(route.bus, 'remove', event)}
        >×</button>
      </span>;
    })}

    <button
      ref={trigger}
      type="button"
      aria-label={`${t(field === 'outputs' ? 'Add output' : 'Add send')} ${strip.name}`}
      disabled={strip.locked || recording || routes.length >= 10 || field === 'sends' && (context.timeline.buses?.length ?? 0) < 2}
      title={field === 'sends' && (context.timeline.buses?.length ?? 0) < 2 ? t('Add another bus before creating a send.') : undefined}
      onClick={event => {
        const rect = event.currentTarget.getBoundingClientRect();
        setAnchor({ x: Math.max(4, Math.min(window.innerWidth - 180, rect.left)), y: Math.max(4, Math.min(window.innerHeight - 260, rect.bottom)) });
        setOpen(!open);
      }}
    >+</button>

    {open && createPortal(<div
      ref={menu}
      className="route-menu"
      role="menu"
      style={{ left: anchor.x, top: anchor.y }}
      aria-label={t(field === 'outputs' ? 'Outputs' : 'Sends')}
    >

      {choices.map(({ bus, assigned, reason }) => <button
        key={bus.id}
        type="button"
        role="menuitemcheckbox"
        aria-checked={assigned}
        disabled={Boolean(reason) || strip.locked || recording}
        title={reason === 'bus_cycle' ? t('Routing cycle') : reason === 'bus_depth' ? t('Maximum six routing levels') : reason ? t('Maximum ten routes') : undefined}
        onClick={event => {
          apply(bus.id, assigned ? 'remove' : 'add', event);
          if (!event.ctrlKey)
            setOpen(false);
        }}
      >
        {bus.name}
      </button>)}
    </div>, document.body)}

    {problem && <span role="alert">{problem}</span>}
  </div>;
}
