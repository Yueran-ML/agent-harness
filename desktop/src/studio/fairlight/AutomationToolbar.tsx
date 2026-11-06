import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import type { Change, CopiedLane } from '../timeline/commands';
import { categories, defaultRecord, ownerKey, ownerOf, sampleTime } from './automation';
import type { FairlightView } from './viewState';
import { useFairlightText } from './words';
export function AutomationToolbar({ bridge, session, timeline, view, update, recording, selected, frame, onEdit, onFailure, onToggle, marks }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  timeline: Timeline;
  view: FairlightView;
  update: (patch: Partial<FairlightView>) => void;
  recording: boolean;
  selected: readonly string[];
  frame: number;
  onEdit: (ops: Change[]) => void;
  onFailure: (error: unknown) => void;
  onToggle: () => void;
  marks: () => {
    in: number | null;
    out: number | null;
  };
}) {
  const t = useFairlightText(), record = timeline.audio?.record ?? defaultRecord();
  const [glide, setGlide] = useState(() => Number(localStorage.getItem('fairlight.glide_ms') ?? 250));
  const [clipboard, setClipboard] = useState<CopiedLane[]>([]), [busy, setBusy] = useState(false);
  const [activeIn, setActiveIn] = useState(record.active_range?.start ?? null), [activeOut, setActiveOut] = useState(record.active_range?.end ?? null);
  // Keep incomplete marks locally, but let committed edits and undo/redo replace them.
  const active = record.active_range;
  useEffect(() => {
    setActiveIn(active?.start ?? null);
    setActiveOut(active?.end ?? null);
  }, [active?.start.ticks, active?.start.scale, active?.end.ticks, active?.end.scale]);
  const owners = selected.length ? selected.map(ownerOf) : [...timeline.tracks.filter(x => x.kind === 'audio').map(x => ({ track: x.id })), ...(timeline.buses ?? []).map(x => ({ bus: x.id }))];
  const range = () => {
    const m = marks();
    return m.in !== null && m.out !== null && m.out > m.in ? { from: sampleTime(m.in * timeline.rate.den / timeline.rate.num * timeline.sample_rate, timeline.sample_rate), to: sampleTime(m.out * timeline.rate.den / timeline.rate.num * timeline.sample_rate, timeline.sample_rate) } : null;
  };
  const at = () => sampleTime(frame * timeline.rate.den / timeline.rate.num * timeline.sample_rate, timeline.sample_rate);
  const run = async (method: 'automation.record' | 'automation.all_read' | 'automation.join' | 'automation.punch' | 'automation.fill', args: Record<string, unknown> = {}) => {
    setBusy(true);
    try {
      await callEngine(bridge, method, { timeline: timeline.id, ...args }, { expectedSession: session });
    }
    catch (e) {
      onFailure(e);
    }
    finally {
      setBusy(false);
    }
  };
  const patch = (next: Partial<typeof record>) => onEdit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { record: { ...record, ...next } } }]);
  const paste = () => {
    const sources = [...new Set(clipboard.map(l => ownerKey(l.of)))];
    if (selected.length && sources.length !== 1 && sources.length !== selected.length) {
      onFailure(new Error(t('Clipboard tracks must match the destination selection.')));
      return;
    }
    const lanes = selected.length ? selected.flatMap((key, index) => clipboard.filter(l => ownerKey(l.of) === sources[sources.length === 1 ? 0 : index]).map(l => ({ ...l, of: ownerOf(key) }))) : clipboard;
    const r = range();
    onEdit([{
      edit: 'automation_paste', timeline: timeline.id, at: r?.from ?? at(), ...(r ? { to: r.to } : {}), lanes
    }]);
  };
  return <section className="automation-toolbar" aria-label={t('Automation Controls')}>

    <button aria-pressed={view.automation} onClick={onToggle}>{t('Toggle Automation')}</button>

    <button aria-pressed={view.automationControls} onClick={() => update({ automationControls: !view.automationControls })}>
      {t('Automation Controls')}
    </button>

    {view.automation && <>
      <button aria-pressed={view.pencil} onClick={() => update({ pencil: !view.pencil })}>{t('Pencil')}</button>

      <label>
        <input
          type="checkbox"
          checked={timeline.audio?.follows_edit !== false}
          onChange={e => onEdit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { follows_edit: e.target.checked } }])}
        />
        {t('Automation Follows Edit')}
      </label>
    </>}

    {view.automationControls && <fieldset disabled={!view.automation || busy}>

      <button
        aria-pressed={recording}
        data-recording={recording}
        onClick={() => void run('automation.record', { on: !recording, glide_ms: glide })}
      >
        {t('Record automation')}
      </button>

      <label>
        {t('Mode')}
        <select value={record.mode} onChange={e => patch({ mode: e.target.value as typeof record.mode })}>
          {(['write', 'trim'] as const).map(x => <option key={x} value={x}>{t(x)}</option>)}
        </select>
      </label>

      <label>
        {t('Touch')}
        <select value={record.touch} onChange={e => patch({ touch: e.target.value as typeof record.touch })}>
          {(['off', 'latch', 'snap', 'snap_latch'] as const).map(x => <option key={x} value={x}>{t(x)}</option>)}
        </select>
      </label>

      <label>
        {t('On Stop')}
        <select value={record.on_stop} onChange={e => patch({ on_stop: e.target.value as typeof record.on_stop })}>
          {(['event', 'hold', 'return'] as const).map(x => <option key={x} value={x}>{t(x)}</option>)}
        </select>
      </label>

      <label>
        {t('Glide Time')}
        <input
          type="number"
          min={0}
          max={10000}
          value={glide}
          onChange={e => {
            const value = Math.max(0, Math.min(10000, Number(e.target.value)));
            setGlide(value);
            localStorage.setItem('fairlight.glide_ms', String(value));
            if (recording)
              void run('automation.record', { on: true, glide_ms: value });
          }}
        />ms</label>

      {categories.map(x => <label key={x}>
        <input
          type="checkbox"
          checked={record.categories[x]}
          onChange={e => patch({ categories: { ...record.categories, [x]: e.target.checked } })}
        />
        {t(x)}
      </label>)}

      <label>
        <input
          type="checkbox"
          checked={record.preview}
          onChange={e => patch({ preview: e.target.checked })}
        />
        {t('Preview')}
      </label>

      <button onClick={() => {
        const start = at();
        setActiveIn(start);
        if (activeOut && activeOut.ticks / activeOut.scale > start.ticks / start.scale)
          patch({ active_range: { start, end: activeOut, on: record.active_range?.on ?? false } });
      }}>
        {t('Mark Active In')}
      </button>

      <button onClick={() => {
        const end = at();
        setActiveOut(end);
        if (activeIn && end.ticks / end.scale > activeIn.ticks / activeIn.scale)
          patch({ active_range: { start: activeIn, end, on: record.active_range?.on ?? false } });
      }}>
        {t('Mark Active Out')}
      </button>

      <label>
        <input
          type="checkbox"
          checked={record.active_range?.on ?? false}
          onChange={e => {
            const saved = activeIn && activeOut ? { from: activeIn, to: activeOut } : range();
            const r = saved && saved.to.ticks / saved.to.scale > saved.from.ticks / saved.from.scale ? saved : null;
            if (e.target.checked && !r) {
              onFailure(new Error(t('Set timeline In and Out points first.')));
              return;
            }
            patch({ active_range: r ? { start: r.from, end: r.to, on: e.target.checked } : null });
          }}
        />
        {t('Active Range')}
      </label>

      {([['automation.all_read', 'All Read'], ['automation.punch', 'Punch In'], ['automation.join', 'Join Mix']] as const).map(([method, label]) => <button key={method} onClick={() => void run(method)}>{t(label)}</button>)}

      {([false, true] as const).map(glide => <button key={String(glide)} onClick={() => {
        const r = range();
        if (r)
          void run('automation.fill', { ...r, glide });
        else
          onFailure(new Error(t('Set timeline In and Out points first.')));
      }}>
        {t(glide ? 'Glide Range' : 'Fill Range')}
      </button>)}

      <button onClick={() => {
        const r = range();
        void callEngine(bridge, 'automation.copy', { timeline: timeline.id, of: owners, ...(r ?? { at: at() }) }, { expectedSession: session }).then(r => setClipboard(r.lanes)).catch(onFailure);
      }}>
        {t('Copy automation')}
      </button>

      <button disabled={!clipboard.length} onClick={paste}>{t('Paste automation')}</button>

      <button onClick={() => {
        const r = range();
        if (r)
          onEdit([{
            edit: 'automation_erase', timeline: timeline.id, of: owners, ...r
          }]);
        else
          onFailure(new Error(t('Set timeline In and Out points first.')));
      }}>
        {t('Erase automation')}
      </button>
    </fieldset>}
  </section>;
}
