import { useState, type FormEvent } from 'react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { RenderOptions, Timeline } from '../engine';
import { timecode } from '../timecode';
import { duration } from '../timeline/geometry';
import type { Marks } from '../timeline/snap';
import type { RenderRefusal, RenderStart } from './renders';

/** The codecs, by the names that every editor knows them by in every language. */
export const CODECS: readonly { id: RenderOptions['codec']; name: string }[] = [
  { id: 'h264', name: 'H.264' },
  { id: 'hevc', name: 'HEVC (H.265)' },
  { id: 'prores', name: 'ProRes' },
];
const QUALITIES = ['high', 'medium', 'small'] as const;

/** Whether a timeline has a subtitle track that is shown: those are what an SRT is saved of. */
export function hasSubtitles(timeline: Timeline | null): boolean {
  return timeline?.tracks.some(track => track.kind === 'subtitle' && track.enabled) ?? false;
}

/**
 * The frames between the In and Out points of the timeline, as the engine takes a range: half open,
 * not empty, and inside the timeline. An In alone goes to the end, an Out alone from the start.
 * Null where there are no marks, or none that leave anything to export.
 */
export function markedRange(marks: Marks | null, frames: number): { from: number; to: number } | null {
  if (marks === null || (marks.in === null && marks.out === null)) return null;
  const from = Math.max(0, marks.in ?? 0);
  const to = Math.min(frames, marks.out ?? frames);
  return to > from ? { from, to } : null;
}

/**
 * The export of a timeline, as DaVinci Resolve's Quick Export with the settings of its render page
 * that phase 1 has: the timeline, the whole of it or between its In and Out points, the codec, the
 * quality, and whether its subtitles are saved beside the film. The file is chosen in Save As, which
 * the shell opens: the page names no path. It lies over the page, so the views of both monitors are
 * hidden while it is open.
 *
 * Args:
 *   timelines: The timelines of the project.
 *   chosen: The timeline that the page shows, chosen at first.
 *   marks: The In and Out points of that timeline, where the page has some.
 *   running: Whether an export of the window is going on: there is one at a time.
 *   onStart: Ask for the export; it answers what came of it.
 */
export function RenderDialog({ timelines, chosen, marks, running, onStart, onClose }: {
  timelines: readonly Timeline[];
  chosen: string | null;
  marks: Marks | null;
  running: boolean;
  onStart: (options: RenderOptions & { name: string }) => Promise<RenderStart>;
  onClose: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.render;
  const hints = words.hint.studio.render;
  const [timelineId, setTimelineId] = useState(() => chosen ?? timelines[0]?.id ?? '');
  const timeline = timelines.find(each => each.id === timelineId) ?? null;
  // The marks are those of the timeline that the page shows, and of no other.
  const range = timeline !== null && timeline.id === chosen ? markedRange(marks, duration(timeline)) : null;
  const [part, setPart] = useState<'whole' | 'marked'>(() => (range !== null ? 'marked' : 'whole'));
  const [codec, setCodec] = useState<RenderOptions['codec']>('h264');
  const [quality, setQuality] = useState<RenderOptions['quality']>('high');
  const [subtitles, setSubtitles] = useState(true);
  const [pending, setPending] = useState(false);
  const [refused, setRefused] = useState<{ reason: RenderRefusal; detail: string } | null>(null);
  const shownSubtitles = hasSubtitles(timeline);
  const marked = part === 'marked' && range !== null;
  const empty = timeline === null || duration(timeline) === 0;

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (timeline === null || empty || running || pending) return;
    setPending(true);
    setRefused(null);
    const started = await onStart({
      timeline: timeline.id, name: timeline.name, codec, quality, subtitles: shownSubtitles && subtitles,
      ...(marked ? { range } : {}),
    });
    setPending(false);
    if (started.kind === 'started') onClose();
    else if (started.kind === 'refused') setRefused({ reason: started.reason, detail: started.detail });
  };

  return <Sheet title={labels.title} onClose={onClose} foot={<>
    <button type="button" className="btn" onClick={onClose}>{words.label.cancel}</button>
    <button type="button" className="btn btn-main" disabled={empty || running || pending} onClick={() => void submit()} data-test="export-to">{labels.exportTo}</button>
  </>}>
    <form className="render-dialog" onSubmit={event => void submit(event)}>
      <label className="setting-name" htmlFor="render-timeline">{labels.timeline}</label>
      <select id="render-timeline" className="field" value={timelineId} data-first disabled={timelines.length === 0}
        onChange={event => { setTimelineId(event.target.value); setRefused(null); }} data-test="render-timeline">
        {timelines.map(each => <option key={each.id} value={each.id}>{each.name}</option>)}
      </select>
      {empty && <p className="caption" data-test="render-empty">{hints.noTimeline}</p>}

      <fieldset className="render-choice">
        <legend className="setting-name">{labels.range}</legend>
        <label><input type="radio" name="render-range" checked={!marked} onChange={() => setPart('whole')} data-test="range-whole" />{labels.whole}</label>
        <label><input type="radio" name="render-range" checked={marked} disabled={range === null} onChange={() => setPart('marked')} data-test="range-marked" />
          {labels.marked}
          {range !== null && timeline !== null && <span className="mono caption" data-test="range-marks">{timecode(range.from, timeline.rate)} – {timecode(range.to, timeline.rate)}</span>}
        </label>
        {range === null && <p className="caption" data-test="range-hint">{hints.marks}</p>}
      </fieldset>

      <fieldset className="render-choice">
        <legend className="setting-name">{labels.codec}</legend>
        {CODECS.map(each => <label key={each.id}>
          <input type="radio" name="render-codec" checked={codec === each.id} onChange={() => setCodec(each.id)} data-test={`codec-${each.id}`} />{each.name}
        </label>)}
        <p className="caption">{hints.codec}</p>
      </fieldset>

      <label className="setting-name" htmlFor="render-quality">{labels.quality}</label>
      <select id="render-quality" className="field" value={quality} onChange={event => setQuality(event.target.value as RenderOptions['quality'])} data-test="render-quality">
        {QUALITIES.map(each => <option key={each} value={each}>{labels[each === 'small' ? 'low' : each]}</option>)}
      </select>

      {shownSubtitles && <>
        <label className="render-check"><input type="checkbox" checked={subtitles} onChange={event => setSubtitles(event.target.checked)} data-test="render-subtitles" />{labels.subtitles}</label>
        <p className="caption">{hints.subtitles}</p>
      </>}

      {running && <p className="caption" role="status" data-test="render-running">{hints.running}</p>}
      {refused && <p className="render-problem" role="alert" data-test="render-refused">
        {hints.refused[refused.reason]}
        {refused.detail && <span className="caption" lang="en" title={refused.detail}>{refused.detail}</span>}
      </p>}
    </form>
  </Sheet>;
}
