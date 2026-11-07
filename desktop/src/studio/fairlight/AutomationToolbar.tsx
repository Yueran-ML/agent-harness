import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import type { Change, CopiedLane } from '../timeline/commands';
import {
  categories, defaultRecord, GLIDE_MAX, glideValue, NUDGE_UNITS, ownerKey, ownerOf, readGlide, readNudge, sampleTime, saveGlide,
  saveNudge, settingTarget, type NudgeUnit,
} from './automation';
import { useAutomationTouch } from './AutomationTouch';
import { levelPathsOf, pastedLanes } from './automationPaste';
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
  const [glide, setGlide] = useState(readGlide);
  // What is typed is a draft: the Glide time is set (and sent while recording) on Enter or on leaving the field.
  const [glideDraft, setGlideDraft] = useState<string | null>(null);
  const commitGlide = () => {
    const value = glideValue(glideDraft);
    setGlideDraft(null);
    if (value === null || value === glide) return;
    setGlide(value);
    saveGlide(value);
    if (recording) void run('automation.record', { on: true, glide_ms: value });
  };
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
  const touch = useAutomationTouch();
  const [nudge, setNudge] = useState<NudgeUnit>(readNudge);
  // The level that a level is pasted to (p. 3914): the same parameter, or a fader or send level.
  const [pasteLevel, setPasteLevel] = useState('');
  const levels = levelPathsOf(timeline, selected), levelOffered = levels.includes(pasteLevel);
  // A level no longer offered (the selection changed) is forgotten.
  useEffect(() => {
    if (pasteLevel && !levelOffered)
      setPasteLevel('');
  }, [pasteLevel, levelOffered]);
  const run = async <M extends 'automation.record' | 'automation.all_read' | 'automation.join' | 'automation.punch'
    | 'automation.punch_out' | 'automation.fill'>(
    method: M,
    args: Record<string, unknown> = {},
  ) => {
    setBusy(true);
    try {
      return await callEngine(bridge, method, { timeline: timeline.id, ...args }, { expectedSession: session });
    }
    catch (e) {
      onFailure(e);
      return null;
    }
    finally {
      setBusy(false);
    }
  };
  // Punch In (p. 3913): under Preview, what Preview holds is written; without it, the enabled
  // settings of the selected tracks go into Write until the next stop.
  const punchIn = async () => {
    if (record.preview) {
      await run('automation.punch');
      return;
    }
    if (!selected.length) {
      onFailure(new Error(t('Select the tracks to punch in first.')));
      return;
    }
    const of = selected.map(ownerOf);
    // Shown in Write only as the engine put settings of them into Write.
    const punched = await run('automation.punch', { of });
    if (punched?.settings.length)
      touch?.punchIn(punched.settings.map(settingTarget));
  };
  // Punch Out (p. 3913): under Preview, every setting in Write, Trim or Preview goes back to Read;
  // without it, those of the selected tracks (of all, where none are selected). Once the engine
  // has, what is held of them is let go of: the engine passes over that release. Where it
  // refuses, what is written stays as it was.
  const punchOut = async () => {
    const of = record.preview || !selected.length ? null : selected.map(ownerOf);
    if (await run('automation.punch_out', of ? { of } : {}))
      touch?.punchOut(of);
  };
  const patch = (next: Partial<typeof record>) => onEdit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { record: { ...record, ...next } } }]);
  const paste = () => {
    const sources = [...new Set(clipboard.map(l => ownerKey(l.of)))];
    if (selected.length && sources.length !== 1 && sources.length !== selected.length) {
      onFailure(new Error(t('Clipboard tracks must match the destination selection.')));
      return;
    }
    const lanes = pastedLanes(clipboard, selected, levelOffered ? pasteLevel : '');
    if (!lanes.length) {
      onFailure(new Error(t('The clipboard has no level to paste there.')));
      return;
    }
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
          max={GLIDE_MAX}
          value={glideDraft ?? glide}
          onChange={e => setGlideDraft(e.target.value)}
          onBlur={commitGlide}
          onKeyDown={e => {
            if (e.key === 'Enter') { e.preventDefault(); commitGlide(); }
            else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setGlideDraft(null); }
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

      <button onClick={() => void run('automation.all_read')}>{t('All Read')}</button>

      <button onClick={() => void punchIn()}>{t('Punch In')}</button>

      <button onClick={() => void punchOut()}>{t('Punch Out')}</button>

      <button onClick={() => void run('automation.join')}>{t('Join Mix')}</button>

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

      <label>
        {t('Paste level to')}
        <select value={levelOffered ? pasteLevel : ''} onChange={e => setPasteLevel(e.target.value)}>
          <option value="">{t('Same parameter')}</option>
          {levels.map(path => <option key={path} value={path}>
            {path === 'fader_db' ? t('fader') : `${t('Send Level')} ${path.split('.')[1]}`}
          </option>)}
        </select>
      </label>

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

      <label>
        <input
          type="checkbox"
          checked={touch?.writeOnPaste ?? false}
          disabled={!touch}
          onChange={e => touch?.setWriteOnPaste(e.target.checked)}
        />
        {t('Write on Preset or Clipboard Paste')}
      </label>

      <label>
        {t('Nudge automation by')}
        <select value={nudge} onChange={e => {
          const unit = e.target.value as NudgeUnit;
          saveNudge(unit);
          setNudge(unit);
        }}>
          {NUDGE_UNITS.map(unit => <option key={unit} value={unit}>{t(unit)}</option>)}
        </select>
      </label>
    </fieldset>}
  </section>;
}
