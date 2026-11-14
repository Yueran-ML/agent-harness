import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { useWords } from '../../app/locale';
import { callEngine, type Clip, type ColorSettings, type Op, type Timeline } from '../engine';
import { refusalText } from '../inspector/refusal';
import { fullCorrector, neutralCorrector } from './corrector';
import type { MenuItem } from './ColorMenu';
import { matchShots, ShotMatchError } from './shotMatching';

// Filling the complete defaults also normalizes object field order before the comparison;
// Rust's serialized order and omitted fields in older projects do not count as adjustments.
const adjusted = (clip: Clip) => clip.grade.nodes.some(node => node.enabled && (node.kind !== 'corrector'
  || JSON.stringify(fullCorrector(node.params)) !== JSON.stringify(neutralCorrector())));

/** The thumbnail command keeps the selection; the right-clicked clip is the reference, not a new selection. */
export function useShotMatch({ bridge, session, revision, timeline, clips, chosen, current, nodes, color }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  revision: number;
  timeline: Timeline | null;
  clips: readonly Clip[];
  chosen: ReadonlySet<string>;
  current: string | null;
  nodes: Readonly<Record<string, string>>;
  color?: ColorSettings;
}) {
  const words = useWords();
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const active = useRef<AbortController | null>(null);
  const now = useRef({ bridge, session, revision, timeline, clips, chosen, current, nodes, color, words });
  now.current = { bridge, session, revision, timeline, clips, chosen, current, nodes, color, words };
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; active.current?.abort(); }; }, []);
  useEffect(() => { active.current?.abort(); setNotice(null); }, [session, timeline?.id]);

  const start = useCallback(async (referenceId: string, selectedIds: readonly string[], resetting: boolean) => {
    if (active.current) return;
    const asked = now.current;
    if (!asked.timeline) return;
    // A menu can stay open while the agent edits a grade. Pair this revision with these fresh
    // clips, never with the grade objects captured when the menu was opened.
    const reference = asked.clips.find(clip => clip.id === referenceId);
    const selected = asked.clips.filter(clip => selectedIds.includes(clip.id));
    if (!reference || selected.length !== selectedIds.length) { setNotice(asked.words.hint.studio.colorPage.shotMatch.changed); return; }
    if (selected.length === 0) return;
    const abort = new AbortController();
    active.current = abort;
    setBusy(true);
    const text = asked.words.hint.studio.colorPage.shotMatch;
    setNotice(resetting ? null : asked.words.label.studio.colorPage.matching);
    try {
      if (resetting) {
        const ops: Op[] = selected.flatMap(clip => clip.grade.nodes.flatMap(node => node.kind === 'corrector' && node.params.shot_match != null
          ? [{ op: 'set_node' as const, grade: { level: 'clip' as const, timeline: asked.timeline!.id, clip: clip.id }, id: node.id, params: { shot_match: null } }] : []));
        if (ops.length) await callEngine(asked.bridge, 'transaction.apply', { revision: asked.revision, ops });
      } else {
        await matchShots({ bridge: asked.bridge, session: asked.session, revision: asked.revision, timeline: asked.timeline.id,
          reference, clips: selected, nodes: asked.nodes, signal: abort.signal });
      }
      if (!live.current || abort.signal.aborted) return;
      const matched = selected.filter(clip => clip.id !== reference.id);
      const notes = resetting ? [] : [
        text.done(matched.length),
        ...(matched.some(adjusted) ? [text.graded] : []),
        ...(asked.color && asked.color.science !== 'yrgb' ? [text.space] : []),
      ];
      setNotice(notes.join(' ') || null);
    } catch (error) {
      if (!live.current || asked.session !== now.current.session || asked.timeline.id !== now.current.timeline?.id) return;
      if (error instanceof ShotMatchError) {
        if (error.reason !== 'cancelled') setNotice(text[error.reason]);
      } else setNotice(refusalText(error, now.current.words.hint.studio));
    } finally {
      active.current = null;
      if (live.current) setBusy(false);
    }
  }, []);

  const items = useCallback((id: string): MenuItem[] => {
    const state = now.current;
    const reference = state.clips.find(clip => clip.id === id);
    if (!reference) return [];
    const selected = state.clips.filter(clip => state.chosen.size ? state.chosen.has(clip.id) : clip.id === state.current);
    const reset = state.chosen.has(id) ? selected : [reference];
    const labels = state.words.label.studio.colorPage.nodeMenu;
    return [
      { id: 'shot-match', label: labels.shotMatch, disabled: active.current !== null || !selected.some(clip => clip.id !== id), run: () => { void start(id, selected.map(clip => clip.id), false); } },
      { id: 'reset-clip-shot-match', label: labels.resetShotMatch, disabled: active.current !== null || !reset.some(clip => clip.grade.nodes.some(node => node.kind === 'corrector' && node.params.shot_match != null)),
        run: () => { void start(id, reset.map(clip => clip.id), true); } },
    ];
  }, [start]);
  return { items, notice, busy };
}
