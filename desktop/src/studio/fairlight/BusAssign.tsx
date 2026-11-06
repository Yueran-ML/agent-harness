import { useEffect, useRef, useState } from 'react';
import { useOverlay } from '../../app/overlay';
import { keyOf } from '../../app/keys';
import { callEngine } from '../engine';
import { editKey } from '../timeline/editKeys';
import { mixerStrips } from '../audio/mixerModel';
import { targetOf } from '../audio/processorModel';
import { busChoices, routingChanges, type Routes } from '../audio/routingModel';
import { useAudioGesture } from '../timeline/useAudioGesture';
import type { RoutingContext } from './BusFormat';
import { useFairlightText } from './words';

export function BusAssign({ onClose, ...context }: RoutingContext & { onClose: () => void }) {
  useOverlay(); const t = useFairlightText();
  const panel = useRef<HTMLElement>(null);
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panel.current?.focus({ preventScroll: true });
    return () => { if (before?.isConnected) before.focus(); };
  }, []);
  const [bus, setBus] = useState(context.timeline.buses?.[0]?.id ?? ''), [field, setField] = useState<Routes>('outputs');
  const [icons, setIcons] = useState(false), [problem, setProblem] = useState('');
  const [history, setHistory] = useState<('history.undo' | 'history.redo')[]>([]), [historyBusy, setHistoryBusy] = useState(false);
  const historyPending = useRef(false);
  const strips = mixerStrips(context.timeline);
  const gesture = useAudioGesture({ ...context, identity: `${context.timeline.id}/bus-assign`, locked: historyBusy, onFailure: e => setProblem(String(e)) });
  const sweep = useRef<{ keys: Set<string>; add: boolean; revision: number; session: number; timeline: string; bus: string; field: Routes } | null>(null);
  const latest = useRef({ context, strips, gesture }); latest.current = { context, strips, gesture };
  const paint = (keys: string[], add: boolean) => gesture.commit(routingChanges(context.timeline, strips.filter(s => keys.includes(s.key)).map(targetOf), field, bus, add ? 'add' : 'remove'));
  // useOverlay hides the native viewer and blocks the underlying editor's shortcut handler.
  // Use the same engine history here, after an assignment in flight has finished.
  useEffect(() => {
    if (!history.length || gesture.busy() || historyPending.current) return;
    historyPending.current = true; setHistoryBusy(true);
    void callEngine(context.bridge, history[0], {}, { expectedSession: context.session })
      .catch(error => setProblem(String(error)))
      .finally(() => { historyPending.current = false; setHistoryBusy(false); setHistory(rows => rows.slice(1)); });
  });
  useEffect(() => {
    const cancel = () => { sweep.current = null; };
    const finish = () => {
      const pending = sweep.current; cancel(); const { context: current, strips: rows, gesture: writer } = latest.current;
      if (!pending || pending.revision !== current.revision || pending.session !== current.session || pending.timeline !== current.timeline.id) return;
      writer.commit(routingChanges(current.timeline, rows.filter(s => pending.keys.has(s.key)).map(targetOf), pending.field, pending.bus, pending.add ? 'add' : 'remove'));
    };
    window.addEventListener('pointerup', finish); window.addEventListener('pointercancel', cancel); window.addEventListener('blur', cancel);
    return () => { cancel(); window.removeEventListener('pointerup', finish); window.removeEventListener('pointercancel', cancel); window.removeEventListener('blur', cancel); };
  }, []);
  return <section ref={panel} tabIndex={-1} role="dialog" aria-label={t('Bus Assign')} className="routing-window" onKeyDown={event => {
    const action = editKey(keyOf(event));
    if (action?.kind === 'history') { event.preventDefault(); event.stopPropagation(); sweep.current = null; setHistory(rows => [...rows, action.method]); return; }
    if (event.key === 'Escape' && !event.nativeEvent.isComposing) { sweep.current = null; event.stopPropagation(); onClose(); }
  }}>
    <header><h2>{t('Bus Assign')}</h2><button type="button" onClick={() => setIcons(!icons)}>{t(icons ? 'List view' : 'Icon view')}</button><button type="button" onClick={onClose}>{t('Close')}</button></header>
    {problem && <p role="alert">{problem}</p>}
    <div className="routing-actions">{context.timeline.buses?.map(b => <div key={b.id}>{b.name}{(['sends', 'outputs'] as const).map(f => <button key={f} type="button" aria-label={`${t(f === 'outputs' ? 'Out' : 'Send')} ${b.name}`}
      aria-pressed={bus === b.id && field === f} onClick={() => { sweep.current = null; setBus(b.id); setField(f); }}>{t(f === 'outputs' ? 'Out' : 'Send')}</button>)}</div>)}</div>
    <div className="assignment-targets" data-icons={icons || undefined} role="group" aria-label={t('Assignment targets')}>{strips.map(strip => {
      const choice = busChoices(context.timeline, targetOf(strip), field).find(c => c.bus.id === bus);
      const disabled = historyBusy || history.length > 0 || strip.locked || !choice || Boolean(choice.reason);
      return <button key={strip.key} type="button" aria-label={strip.name} aria-pressed={choice?.assigned ?? false} disabled={disabled}
        title={choice?.reason === 'bus_cycle' ? t('Routing cycle') : choice?.reason === 'bus_depth' ? t('Maximum six routing levels') : undefined}
        onPointerDown={event => {
          if (event.button !== 0) return; event.preventDefault(); event.currentTarget.focus();
          sweep.current = { keys: new Set([strip.key]), add: !choice?.assigned, revision: context.revision, session: context.session, timeline: context.timeline.id, bus, field };
        }} onPointerEnter={event => { if (event.buttons === 1 && sweep.current && !disabled) sweep.current.keys.add(strip.key); }}
        onClick={event => { if (!event.detail) paint([strip.key], !choice?.assigned); }}>{strip.name}{icons && choice?.assigned ? ` · ${bus}${field === 'outputs' ? 'o' : 's'}` : ''}</button>;
    })}</div>
    <footer><button type="button" disabled={!bus || historyBusy || history.length > 0} onClick={() => paint(strips.map(s => s.key), true)}>{t('Assign All')}</button>
      <button type="button" disabled={!bus || historyBusy || history.length > 0} onClick={() => paint(strips.map(s => s.key), false)}>{t('Unassign All')}</button></footer>
  </section>;
}
