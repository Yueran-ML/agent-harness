import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { EngineBridge } from '../../bridge';
import type { Clip, Speed, Timeline } from '../engine';
import type { Edit } from '../timeline/commands';
import { clipLocations } from '../timeline/selection';
import { timecode } from '../timecode';
import { common } from './common';
import { useCommit } from './useCommit';
import { useFairlightText } from '../fairlight/words';
import { setClips } from './common';

/** The speeds that the engine takes, in percent: from 1/100 to 100 times. */
export const SLOWEST = 1;
export const FASTEST = 10_000;

const gcd = (a: number, b: number): number => (b === 0 ? a : gcd(b, a % b));
/** A speed in percent (to a hundredth) as the fraction that the engine keeps, in its lowest terms. */
export function speedOf(percent: number): Speed {
  const num = Math.round(percent * 100);
  const divisor = gcd(num, 10_000);
  return { num: num / divisor, den: 10_000 / divisor };
}
export const percentOf = (speed: Speed) => Number(((speed.num / speed.den) * 100).toFixed(2));

/**
 * The edits that change the speed of the clips chosen, in one transaction: a group of linked clips
 * all chosen changes as one (`linked`), a clip chosen without its partners alone. Titles and solid
 * colours have no speed, and are left out.
 */
export function speedEdits(timeline: Timeline, ids: readonly string[], speed: Speed, reverse: boolean | undefined, ripple: boolean): Edit[] {
  const locations = clipLocations(timeline);
  const chosen = new Set(ids);
  const groups = new Map<string, string[]>();
  for (const [id, { clip }] of locations) if (clip.link && clip.source.kind === 'media') groups.set(clip.link, [...groups.get(clip.link) ?? [], id]);
  const done = new Set<string>();
  const edits: Edit[] = [];
  for (const id of ids) {
    const clip = locations.get(id)?.clip;
    if (!clip || clip.source.kind !== 'media') continue;
    const whole = clip.link !== null && (groups.get(clip.link) ?? []).every(member => chosen.has(member));
    if (whole && done.has(clip.link!)) continue;
    if (whole) done.add(clip.link!);
    edits.push({ edit: 'set_speed', timeline: timeline.id, clip: id, speed, ...(reverse === undefined ? {} : { reverse }), ripple, linked: whole });
  }
  return edits;
}

/**
 * DaVinci Resolve's Change Clip Speed, opened with R or from the inspector: the speed in percent,
 * whether the clips play backward, and whether what follows moves with their ends (Ripple
 * Sequence). It lies over the page, so the views of both monitors are hidden while it is open.
 */
export function SpeedDialog({ clips: ids, timeline, bridge, session, revision, onClose }: {
  clips: readonly string[]; timeline: Timeline; bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; onClose: () => void;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.speed;
  const hints = words.hint.studio.inspector;
  const locations = clipLocations(timeline);
  const clips = ids.flatMap(id => { const found = locations.get(id); return found && !found.track.locked && found.clip.source.kind === 'media' ? [found.clip] : []; });
  const was = common(clips, clip => percentOf(clip.speed));
  const wasReverse = common(clips, clip => clip.reverse);
  const [percent, setPercent] = useState(was === null ? '' : String(was));
  const [reverse, setReverse] = useState<boolean | null>(wasReverse);
  const [ripple, setRipple] = useState(true);
  const [speedChanged, setSpeedChanged] = useState(false);
  const [reverseChanged, setReverseChanged] = useState(false);
  const [correction, setCorrection] = useState(common(clips, clip => clip.pitch_correction ?? false));
  const [correctionChanged, setCorrectionChanged] = useState(false);
  const correctionBox = useRef<HTMLInputElement>(null);
  const box = useRef<HTMLInputElement>(null);
  const { commit, refusal, pending } = useCommit({ bridge, revision, session, timeline });
  useEffect(() => { if (box.current) box.current.indeterminate = reverse === null; }, [reverse]);
  useEffect(() => { if (correctionBox.current) correctionBox.current.indeterminate = correction === null; }, [correction]);
  const value = Number(percent.trim().replace(',', '.'));
  const validPercent = percent.trim() !== '' && Number.isFinite(value) && value >= SLOWEST && value <= FASTEST;
  const valid = !speedChanged || validPercent;
  const first = clips[0];
  const length = validPercent && first ? Math.max(1, Math.round(first.length * (first.speed.num / first.speed.den) / (value / 100))) : null;
  const warning = clips.some(clip => (correctionChanged ? correction : clip.pitch_correction) && (() => {
    const percent = speedChanged && validPercent ? value : percentOf(clip.speed); return percent < 50 || percent > 200;
  })());
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!valid || clips.length === 0 || pending) return;
    const sent = await commit(now => {
      const open = clips.map(clip => clip.id).filter(id => !clipLocations(now).get(id)?.track.locked);
      const speedOps = speedChanged ? speedEdits(now, open, speedOf(value), reverseChanged ? reverse ?? undefined : undefined, ripple)
        : reverseChanged ? open.flatMap(id => { const clip = clipLocations(now).get(id)?.clip; return clip ? speedEdits(now, [id], clip.speed, reverse ?? undefined, ripple) : []; }) : [];
      const correctionOps = correctionChanged && correction !== null ? setClips(open, () => ({ pitch_correction: correction }))(now) ?? [] : [];
      return [...speedOps, ...correctionOps];
    });
    if (sent) onClose();
  };
  return <Sheet title={labels.title} onClose={onClose} foot={<>
    <button type="button" className="btn" onClick={onClose}>{words.label.cancel}</button>
    <button type="button" className="btn btn-main" disabled={!valid || clips.length === 0 || pending} onClick={() => void submit()} data-test="apply-speed">{labels.apply}</button>
  </>}>
    <form className="speed-dialog" onSubmit={event => void submit(event)}>
      {clips.length === 0 ? <p className="caption">{hints.speedMedia}</p> : <>
        <label className="setting-name" htmlFor="speed-percent">{labels.percent}</label>
        <span className="speed-percent">
          <input id="speed-percent" className="field" type="text" inputMode="decimal" autoComplete="off" spellCheck={false} data-first
            value={percent} placeholder={was === null ? '—' : undefined} aria-invalid={percent.trim() !== '' && !valid}
            onChange={event => { setPercent(event.target.value); setSpeedChanged(true); }} />
          <span aria-hidden="true">%</span>
        </span>
        {percent.trim() !== '' && !valid && <p className="caption" role="alert">{hints.speedRange}</p>}
        {length !== null && clips.length === 1 && <p className="mono caption" data-test="speed-duration">{labels.duration} {timecode(length, timeline.rate)}</p>}
        <label className="speed-check"><input ref={box} type="checkbox" checked={reverse === true} onChange={event => { setReverse(event.target.checked); setReverseChanged(true); }} />{labels.reverse}</label>
        <label className="speed-check"><input type="checkbox" checked={ripple} onChange={event => setRipple(event.target.checked)} />{labels.ripple}</label>
        <label className="speed-check"><input ref={correctionBox} type="checkbox" checked={correction === true} onChange={event => { setCorrection(event.target.checked); setCorrectionChanged(true); }} />{t('Pitch Correction')}</label>
        {warning && <p className="caption" role="status">{t('Pitch correction works best at speeds from 50% to 200%.')}</p>}
      </>}
      {refusal && <p className="speed-problem" role="alert">{refusal.text}</p>}
    </form>
  </Sheet>;
}
