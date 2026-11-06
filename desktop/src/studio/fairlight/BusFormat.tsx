import { useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { Sheet } from '../../app/Sheet';
import { callEngine, type Bus, type BusFormat as Format, type Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { BUS_FORMATS, busChannels, busUsers, nextBus } from '../audio/routingModel';
import { useFairlightText } from './words';

export interface RoutingContext { bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline }
/** A local draft: opening/typing/cancelling never touches the project's history. */
export function BusFormat({ bridge, session, revision, timeline, onClose }: RoutingContext & { onClose: () => void }) {
  const t = useFairlightText();
  const origin = useRef({ session, revision, timeline });
  const current = useRef({ session, timeline: timeline.id }); current.current = { session, timeline: timeline.id };
  const [buses, setBuses] = useState(() => structuredClone(timeline.buses ?? []));
  const [selected, setSelected] = useState(buses[0]?.id ?? ''), [removed, setRemoved] = useState<string[]>([]);
  const [problem, setProblem] = useState(''), [saving, setSaving] = useState(false);
  const stale = session !== origin.current.session || revision !== origin.current.revision || timeline.id !== origin.current.timeline.id;
  const disabled = stale || saving;
  const replace = (id: string, patch: Partial<Bus>) => setBuses(rows => rows.map(b => b.id === id ? { ...b, ...patch } : b));
  const add = (copy?: Bus) => {
    const bus = nextBus(timeline, [...buses, ...(origin.current.timeline.buses ?? []).filter(b => removed.includes(b.id))], copy);
    setBuses(rows => [...rows, bus]); setSelected(bus.id); setProblem('');
  };
  const save = async () => {
    if (disabled) return;
    const before = origin.current.timeline.buses ?? [];
    // Deletion was checked against the draft at each click. Keep that order and free its
    // slots before adding replacements, so a legal 64-bus draft never transiently has 65.
    const ops: Change[] = removed.filter(id => before.some(b => b.id === id)).map(id => ({ op: 'remove_bus', timeline: timeline.id, id }));
    ops.push(...buses.filter(b => !before.some(old => old.id === b.id)).map(bus => ({ op: 'add_bus' as const, timeline: timeline.id, bus })));
    for (const bus of buses) {
      const old = before.find(b => b.id === bus.id); if (!old) continue;
      const patch: Record<string, unknown> = {};
      for (const key of ['name', 'format', 'color'] as const) if (bus[key] !== old[key]) patch[key] = bus[key];
      if (Object.keys(patch).length) ops.push({ op: 'set_bus', timeline: timeline.id, id: bus.id, patch });
    }
    if (!ops.length) { onClose(); return; }
    setSaving(true); setProblem('');
    try { await callEngine(bridge, 'transaction.apply', { revision: origin.current.revision, ops }, { expectedSession: origin.current.session });
      if (current.current.session === origin.current.session && current.current.timeline === origin.current.timeline.id) onClose(); }
    catch (error) { setProblem(error instanceof Error ? error.message : String(error)); setSaving(false); }
  };
  return <Sheet title={t('Bus Format')} wide onClose={() => { if (!saving) onClose(); }} foot={<>
    <button type="button" data-first disabled={saving} onClick={onClose}>{t('Cancel')}</button><button type="button" disabled={disabled} onClick={() => void save()}>{t('OK')}</button>
  </>}><div className="bus-format">
    {stale && <p role="alert">{t('The project changed. Reopen this window before saving.')}</p>}
    {problem && <p role="alert">{problem}</p>}
    <fieldset disabled={disabled}><table><thead><tr><th /><th>{t('Name')}</th><th>{t('Format')}</th><th>{t('Channels')}</th><th>{t('Color')}</th></tr></thead>
      <tbody>{buses.map(bus => <tr key={bus.id} data-selected={selected === bus.id || undefined} onClick={() => setSelected(bus.id)}>
        <td><input type="radio" name="bus-selection" aria-label={bus.name} checked={selected === bus.id} onChange={() => setSelected(bus.id)} /></td>
        <td><input aria-label={`${t('Name')} ${bus.id}`} value={bus.name} onChange={event => replace(bus.id, { name: event.target.value })} /></td>
        <td><select aria-label={`${t('Format')} ${bus.id}`} value={bus.format} onChange={event => replace(bus.id, { format: event.target.value as Format })}>
          {BUS_FORMATS.map(format => <option key={format} value={format}>{format === 'mono' ? t('Mono') : format === 'stereo' ? t('Stereo') : format}</option>)}
        </select></td><td>{busChannels(bus.format)}</td>
        <td><input type="color" aria-label={`${t('Color')} ${bus.id}`} value={bus.color.slice(0, 7)} onChange={event => replace(bus.id, { color: `${event.target.value}${bus.color.slice(7) || 'ff'}` })} /></td>
      </tr>)}</tbody></table>
      <div className="routing-actions"><button type="button" disabled={buses.length >= 64} onClick={() => add()}>{t('Add Bus')}</button>
        <button type="button" disabled={buses.length >= 64 || !selected} onClick={() => add(buses.find(b => b.id === selected))}>{t('Duplicate')}</button>
        <button type="button" disabled={!selected} onClick={() => {
          const users = busUsers({ ...timeline, buses }, selected);
          if (users.length) { setProblem(`${t('In use')}: ${users.join(', ')}`); return; }
          setRemoved(ids => [...ids, selected]); setBuses(rows => rows.filter(b => b.id !== selected)); setSelected(''); setProblem('');
        }}>{t('Remove')}</button><span>{t('Used')}: {buses.length} / 64</span></div>
    </fieldset>
  </div></Sheet>;
}
