import { useFairlightText } from './words';
import { useEffect, useRef, useState } from 'react';
import type { Op, Timeline, Track } from '../engine';
import { NameField } from '../media/NameField';

export const TRACK_COLORS = ['#7f7f7fff', '#c65c5cff', '#dc8a54ff', '#d5b758ff', '#a4bd5bff', '#66ad74ff', '#56a69aff', '#57b7c7ff',
  '#5aa0e0ff', '#687cceff', '#8f70c4ff', '#b275c0ff', '#c36d9bff', '#a98268ff', '#b8b8b8ff', '#e5e5e5ff'];
/** Final indexes address the full timeline, including video/subtitle tracks. */
export function moveTracks(timeline: Timeline, ids: string[], before: string): Op[] {
  const moving = timeline.tracks.filter(track => ids.includes(track.id));
  if (!moving.length || ids.includes(before)) return [];
  const wanted = timeline.tracks.filter(track => !ids.includes(track.id));
  const at = wanted.findIndex(track => track.id === before);
  if (at < 0) return [];
  wanted.splice(at, 0, ...moving);
  const order = timeline.tracks.map(track => track.id); const ops: Op[] = [];
  wanted.forEach((track, index) => {
    const from = order.indexOf(track.id);
    if (from !== index) { ops.push({ op: 'move_track', timeline: timeline.id, id: track.id, index }); order.splice(index, 0, order.splice(from, 1)[0]); }
  });
  return ops;
}
export function IndexTracks({ timeline, hidden, onHidden, onSelect, onEdit, selection, onSelection, automation = false }: {
  timeline: Timeline; hidden: readonly string[]; onHidden: (ids: string[]) => void; onSelect: (id: string) => void; onEdit: (ops: Op[]) => void;
  automation?: boolean;
  selection?: readonly string[]; onSelection?: (ids: string[]) => void;
}) {
  const t = useFairlightText();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [localSelection, setLocalSelection] = useState<string[]>([]);
  const selected = selection ?? localSelection;
  const setSelected = (ids: string[]) => { setLocalSelection(ids); onSelection?.(ids); };
  const dragged = useRef<string[]>([]);
  const sweep = useRef<{ field: 'mute' | 'solo' | 'locked'; value: boolean; ids: Set<string> } | null>(null);
  const submit = useRef(onEdit); submit.current = onEdit;
  useEffect(() => {
    const cancel = () => { sweep.current = null; };
    const finish = () => {
      const active = sweep.current; cancel();
      if (active) submit.current([...active.ids].map(id => active.field === 'locked'
        ? { op: 'set_track', timeline: timeline.id, id, locked: active.value }
        : { op: 'set_track_audio', timeline: timeline.id, track: id, patch: { [active.field]: active.value } }));
    };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') cancel(); };
    // A sweep can finish outside the starting button; publish only the complete gesture.
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', cancel);
    window.addEventListener('blur', cancel);
    window.addEventListener('keydown', escape);
    return () => {
      cancel(); window.removeEventListener('pointerup', finish); window.removeEventListener('pointercancel', cancel);
      window.removeEventListener('blur', cancel); window.removeEventListener('keydown', escape);
    };
  }, [timeline.id]);
  const patch = (track: Track, fields: Record<string, unknown>) => onEdit([{ op: 'set_track_audio', timeline: timeline.id, track: track.id, patch: fields }]);
  const toggle = (track: Track, field: 'mute' | 'solo' | 'locked' | 'visible', value: boolean) => {
    if (field === 'visible') onHidden(value ? hidden.filter(id => id !== track.id) : [...hidden, track.id]);
    else if (field === 'locked') onEdit([{ op: 'set_track', timeline: timeline.id, id: track.id, locked: value }]);
    else patch(track, { [field]: value });
  };
  return <section className="fairlight-index" aria-label={t('Index Tracks')}><h2>{t('Tracks')}</h2><table><thead><tr>
    {(['Color', 'Visible', '#', 'Name', 'Controls', 'Format', 'ADC', 'Order'] as const).map(label => <th key={label}>{label === '#' || label === 'ADC' ? label : t(label)}</th>)}<th>VCA</th>
  </tr></thead><tbody>{timeline.tracks.map((track, index) => <tr key={track.id} data-index-track={track.id} aria-selected={selected.includes(track.id)} draggable
    onDragStart={event => { dragged.current = selected.includes(track.id) ? [...selected] : [track.id]; event.dataTransfer.setData('application/x-fairlight-tracks', dragged.current.join(',')); }}
    onDragOver={event => { if (dragged.current.length) event.preventDefault(); }}
    onDrop={event => { event.preventDefault(); onEdit(moveTracks(timeline, dragged.current, track.id)); dragged.current = []; }} onDragEnd={() => { dragged.current = []; }}>
    <td><select aria-label={`${t('Color')} ${track.name}`} value={track.color ?? TRACK_COLORS[8]} style={{ color: track.color ?? TRACK_COLORS[8] }}
      onChange={event => onEdit((selected.includes(track.id) ? selected : [track.id]).map(id => ({ op: 'set_track', timeline: timeline.id, id, color: event.target.value })))}>
      {TRACK_COLORS.map((color, at) => <option key={color} value={color}>{at + 1}</option>)}</select></td>
    <td><input type="checkbox" aria-label={`${t('Visible')} ${track.name}`} checked={!hidden.includes(track.id)} onChange={event => toggle(track, 'visible', event.target.checked)} /></td>
    <td><button type="button" onClick={event => { onSelect(track.id); setSelected(event.ctrlKey ? selected.includes(track.id) ? selected.filter(id => id !== track.id) : [...selected, track.id] : [track.id]); }}>{track.id}</button></td>
    <td>{renaming === track.id ? <NameField name={track.name} onCommit={async name => { onEdit([{ op: 'set_track', timeline: timeline.id, id: track.id, name }]); setRenaming(null); return true; }} onCancel={() => setRenaming(null)} />
      : <button type="button" onClick={() => { onSelect(track.id); setSelected([track.id]); setRenaming(track.id); }}>{track.name}</button>}</td>
    <td>{(['locked', ...(track.kind === 'audio' ? ['solo', 'mute'] : [])] as const).map(field => {
      const checked = field === 'locked' ? track.locked : track.kind === 'audio' && Boolean(track[field as 'solo' | 'mute']);
      return <button type="button" key={field} aria-label={`${t(field as 'locked' | 'solo' | 'mute')} ${track.name}`} aria-pressed={checked}
        onPointerDown={event => { if (event.button !== 0) return; event.preventDefault(); sweep.current = { field: field as 'locked' | 'solo' | 'mute', value: !checked, ids: new Set([track.id]) }; }}
        onPointerEnter={event => { const active = sweep.current; if (event.buttons === 1 && active?.field === field) active.ids.add(track.id); }}
        onClick={event => { if (event.detail === 0) toggle(track, field as 'locked' | 'solo' | 'mute', !checked); }}>{field === 'locked' ? 'L' : field === 'solo' ? 'S' : 'M'}</button>;
    })}</td>
    <td>{track.kind === 'audio' ? <><select aria-label={`${t('Format')} ${track.name}`} value={track.format === '5.1' && track.order === 'film' ? '5.1-film' : track.format ?? 'stereo'} onChange={event => {
      const format = event.target.value; patch(track, { format: format === '5.1-film' ? '5.1' : format, channels: format === 'mono' ? 1 : format.startsWith('5.1') ? 6 : 2, order: format === '5.1-film' ? 'film' : 'smpte' });
    }}><option value="mono">{t('Mono')}</option><option value="stereo">{t('Stereo')}</option><option value="5.1">5.1</option><option value="5.1-film">5.1 Film</option><option value="adaptive">{t('Adaptive')}</option></select>
      {track.format === 'adaptive' && <select aria-label={`${t('Channels')} ${track.name}`} value={track.channels} onChange={event => patch(track, { channels: Number(event.target.value) })}>
        {Array.from({ length: 24 }, (_, at) => <option key={at + 1}>{at + 1}</option>)}</select>}</> : t(track.kind === 'video' ? 'Video' : 'Subtitles')}</td>
    <td>{track.kind === 'audio' && <input type="checkbox" aria-label={`ADC ${track.name}`} checked={track.adc ?? true} onChange={event => patch(track, { adc: event.target.checked })} />}</td>
    <td><button type="button" aria-label={t('Move track up').replace('{name}', track.name)} disabled={index === 0} onClick={() => onEdit([{ op: 'move_track', timeline: timeline.id, id: track.id, index: index - 1 }])}>↑</button>
      <button type="button" aria-label={t('Move track down').replace('{name}', track.name)} disabled={index === timeline.tracks.length - 1} onClick={() => onEdit([{ op: 'move_track', timeline: timeline.id, id: track.id, index: index + 1 }])}>↓</button></td>
    <td>{track.kind==='audio' ? (track.strip?.vcas??[]).join(', ') : ''}</td>
  </tr>)}</tbody></table>{automation&&<fieldset><legend>{t('Buses')}</legend>{timeline.buses?.map(bus=><label key={bus.id}><input type="checkbox" checked={!hidden.includes(`bus:${bus.id}`)} onChange={e=>onHidden(e.target.checked?hidden.filter(id=>id!==`bus:${bus.id}`):[...hidden,`bus:${bus.id}`])}/>{bus.name}</label>)}</fieldset>}</section>;
}
