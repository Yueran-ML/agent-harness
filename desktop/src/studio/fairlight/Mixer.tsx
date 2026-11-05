import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Meters, type Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { faderChanges, faderPlan, groupMembers, mixerStrips, operationTargets, stripChange, type FaderPlan, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { ChannelStrip } from './ChannelStrip';
import { useFairlightText } from './words';
import './mixer.css';
import { keyOf } from '../../app/keys';
import { writing } from '../keys';
import { editKey } from '../timeline/editKeys';
import { processorChanges, targetOf, type ProcessorModule } from '../audio/processorModel';
import { ProcessorWindow } from './ProcessorWindow';
import '../audio/processors.css';

/** Keep text/local edits here; ordinary transport and global history keep their existing route. */
export function isolateMixerKeys(event: KeyboardEvent<HTMLElement>) {
  const key = keyOf(event), action = editKey(key);
  if (key.composing || writing(event.target) || (action && action.kind !== 'history') || ['[', ']'].includes(key.key)
    || ((key.ctrl || key.meta) && ['c', 'x', 'v'].includes(key.key.toLowerCase()))) event.stopPropagation();
}

export interface MixerProps {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; frame: number; meters: Meters | null;
  selected: readonly string[]; onSelect: (keys: string[]) => void; compact?: boolean;
}
export function Mixer({ bridge, session, revision, timeline, frame, meters, selected, onSelect, compact = false }: MixerProps) {
  const t = useFairlightText(); const [problem, setProblem] = useState<string | null>(null);
  const [processors, setProcessors] = useState<{ key: string; module: ProcessorModule }[]>([]);
  const strips = mixerStrips(timeline), plan = useRef<FaderPlan | null>(null);
  const [preview, setPreview] = useState<Record<string, number>>({});
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${timeline.id}/mixer`, locked: false, onFailure: error => setProblem(error instanceof Error ? error.message : String(error)) });
  const latest = useRef({ timeline, strips, gesture, selected, revision, session }); latest.current = { timeline, strips, gesture, selected, revision, session };
  const anchor = useRef<string | null>(selected[0] ?? null);
  const localSelection = useRef<readonly string[] | null>(null);
  useEffect(() => {
    const local = localSelection.current;
    if (!local || local.length !== selected.length || local.some((key, i) => key !== selected[i])) anchor.current = selected.at(-1) ?? null;
    localSelection.current = null;
  }, [selected]);
  const sweep = useRef<{ field: 'mute' | 'solo' | 'solo_safe'; value: boolean; keys: Set<string>; modifiers: Modifiers } | null>(null);
  const cancel = () => { sweep.current = null; plan.current = null; gesture.cancel(); setPreview({}); };
  const switchOps = (keys: string[], field: 'mute' | 'solo' | 'solo_safe', value: boolean, modifiers: Modifiers): Change[] => {
    const state = latest.current;
    const seeds = keys.flatMap(key => state.strips.filter(s => s.key === key && !s.locked));
    if (field === 'solo') {
      // A solo seed and its directly linked group form one exclusive-solo edit. Expanding its
      // followers into separate edits would make each clear the previous member. Preserve the
      // actual sweep order, so the last requested solo/group wins under Exclusive Solo.
      return seeds.map(s => stripChange(state.timeline.id, s, { solo: value }, !(modifiers.ctrlKey && modifiers.shiftKey)));
    }
    const members = field === 'solo_safe' ? seeds : groupMembers(state.timeline, state.strips, seeds, field, Boolean(modifiers.ctrlKey && modifiers.shiftKey));
    // Each member once. Exclusive Solo remains the server's set_* edit policy.
    return members.filter(s => !s.locked).map(s => stripChange(state.timeline.id, s, { [field]: value }));
  };
  useEffect(() => {
    const abandon = () => { sweep.current = null; };
    const finish = () => { const s = sweep.current; abandon(); if (s) latest.current.gesture.commit(switchOps([...s.keys], s.field, s.value, s.modifiers)); };
    window.addEventListener('pointerup', finish); window.addEventListener('pointercancel', abandon); window.addEventListener('blur', abandon);
    return () => { abandon(); window.removeEventListener('pointerup', finish); window.removeEventListener('pointercancel', abandon); window.removeEventListener('blur', abandon); };
  }, [session, timeline.id]);
  useEffect(() => { sweep.current = null; plan.current = null; setPreview({}); setProcessors([]); }, [session, timeline.id]);
  const start = (strip: MixerStrip, modifiers: Modifiers) => {
    const captured = faderPlan(timeline, strip.key, selected, modifiers);
    if (!captured.members.length || !gesture.start(captured.restore)) return false;
    plan.current = captured; return true;
  };
  const change = (value: number) => {
    const active = plan.current; if (!active || !gesture.active()) return;
    const delta = value - Math.max(-100, active.source.db);
    const changes = faderChanges(timeline.id, active, delta);
    setPreview(Object.fromEntries(active.members.map(s => [s.key, Math.max(-100, Math.min(10, Math.max(-100, s.db) + delta))])));
    gesture.preview(changes);
  };
  const select = (strip: MixerStrip, modifiers: Modifiers) => {
    let keys: string[];
    if (modifiers.shiftKey && anchor.current) {
      const from = strips.findIndex(s => s.key === anchor.current), to = strips.indexOf(strip);
      keys = from >= 0 ? strips.slice(Math.min(from, to), Math.max(from, to) + 1).map(s => s.key) : [strip.key];
    } else {
      keys = modifiers.ctrlKey ? selected.includes(strip.key) ? selected.filter(k => k !== strip.key) : [...selected, strip.key] : [strip.key];
      if (!modifiers.ctrlKey) anchor.current = strip.key;
    }
    localSelection.current = keys;
    onSelect(keys);
  };
  const single = (strip: MixerStrip, field: 'mute' | 'solo' | 'solo_safe', modifiers: Modifiers) => {
    const keys = operationTargets(strips, strip.key, selected, modifiers).map(s => s.key);
    const value = !(field === 'solo_safe' ? strip.safe : strip[field]);
    if (!modifiers.altKey && field !== 'solo_safe') gesture.commit([stripChange(timeline.id, strip, { [field]: value }, !(modifiers.ctrlKey && modifiers.shiftKey))]);
    else gesture.commit(switchOps(keys, field, value, modifiers));
  };
  const draw = (strip: MixerStrip) => {
    const reading = meters?.strips.find(m => 'track' in m.owner ? strip.kind === 'track' && m.owner.track === strip.id : strip.kind === 'bus' && m.owner.bus === strip.id);
    return <ChannelStrip key={strip.key} strip={strip} compact={compact} seconds={frame * timeline.rate.den / timeline.rate.num} selected={selected.includes(strip.key)}
      processorContext={{ bridge, session, revision, timeline, target: targetOf(strip), selected, seconds: frame * timeline.rate.den / timeline.rate.num }}
      onProcessor={module => setProcessors(old => [...(compact ? [] : old.filter(p => p.key !== strip.key || p.module !== module)), { key: strip.key, module }])}
      onStripSetting={(path, value, modifiers) => gesture.commit(processorChanges(timeline.id, operationTargets(strips, strip.key, selected, modifiers), path, value))}
      value={gesture.active() ? preview[strip.key] : undefined} meters={timeline.audio?.prefader_metering ? reading?.pre : reading?.post}
      shadow={Boolean(timeline.audio?.prefader_metering && strips.some(s => s.solo) && !strip.solo && !strip.safe)}
      isActive={gesture.active} onSelect={modifiers => select(strip, modifiers)} onRename={async name => {
        try { await callEngine(bridge, 'transaction.apply', { revision, ops: [strip.kind === 'track' ? { op: 'set_track', timeline: timeline.id, id: strip.id, name }
          : { edit: 'set_bus', timeline: timeline.id, id: strip.id, patch: { name } }] }, { expectedSession: session }); return true; }
        catch (error) { setProblem(error instanceof Error ? error.message : String(error)); return false; }
      }} onStart={modifiers => start(strip, modifiers)} onChange={change} onEnd={() => { plan.current = null; gesture.end(); setPreview({}); }} onCancel={cancel}
      onCommit={(value, modifiers = {}) => { const p = faderPlan(timeline, strip.key, selected, modifiers); gesture.commit(faderChanges(timeline.id, p, value - Math.max(-100, strip.db))); }}
      onToggle={(field, modifiers) => single(strip, field, modifiers)}
      onSweep={(field, event) => {
        if (event.button !== 0 || strip.locked) return;
        event.preventDefault(); event.stopPropagation(); event.currentTarget.focus();
        sweep.current = { field, value: !(field === 'solo_safe' ? strip.safe : strip[field]), keys: new Set(operationTargets(strips, strip.key, selected, event).map(s => s.key)),
          modifiers: { altKey: event.altKey, ctrlKey: event.ctrlKey, shiftKey: event.shiftKey } };
      }} onEnter={(field, event) => { if (event.buttons === 1 && sweep.current?.field === field && !strip.locked) sweep.current.keys.add(strip.key); }} />;
  };
  return <section className="audio-mixer" data-compact={compact || undefined} aria-label={t('Mixer channels')} onKeyDown={event => {
    isolateMixerKeys(event); if (!keyOf(event).composing && event.key === 'Escape') { event.preventDefault(); cancel(); }
  }}>
    <header className="mixer-toolbar"><label><input type="checkbox" checked={timeline.audio?.exclusive_solo ?? false}
      onChange={event => gesture.commit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { exclusive_solo: event.target.checked } }])} />{t('Exclusive Solo')}</label></header>
    {problem && <p role="alert">{problem}<button type="button" onClick={() => setProblem(null)}>{t('Dismiss')}</button></p>}
    <div className="mixer-regions"><div className="mixer-region mixer-tracks" aria-label={t('Tracks')}>{strips.filter(s => s.kind === 'track').map(draw)}</div>
      <div className="mixer-region mixer-buses" aria-label={t('Buses')}>{strips.filter(s => s.kind === 'bus').map(draw)}</div></div>
    {processors.map((p, order) => {
      const owner = strips.find(s => s.key === p.key); if (!owner) return null;
      return <ProcessorWindow key={`${p.key}/${p.module}`} context={{ bridge, session, revision, timeline, target: targetOf(owner), selected, seconds: frame * timeline.rate.den / timeline.rate.num }} module={p.module} name={owner.name} compact={compact} order={order}
        onClose={() => setProcessors(old => old.filter(each => each.key !== p.key || each.module !== p.module))}
        onFront={() => setProcessors(old => old.at(-1) === p ? old : [...old.filter(each => each !== p), p])} />;
    })}
  </section>;
}
