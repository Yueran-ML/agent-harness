import { useState, type FormEvent } from 'react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { RenderCodec, RenderOptions, RenderQuality, SubtitleTrack, Timeline } from '../engine';
import { outFrame, timecode } from '../timecode';
import { duration } from '../timeline/geometry';
import type { Marks } from '../timeline/snap';
import type { RenderRefusal, RenderStart } from './renders';

/** The codecs, by the names that every editor knows them by in every language. */
export const CODECS: readonly { id: RenderCodec; name: string }[] = [
  { id: 'h264', name: 'H.264' },
  { id: 'hevc', name: 'HEVC (H.265)' },
  { id: 'prores', name: 'ProRes' },
];
const QUALITIES = ['high', 'medium', 'small'] as const;

/** The subtitle tracks of a timeline, in its order: each can be saved as an SRT, shown or hidden. */
export function subtitleTracksOf(timeline: Timeline | null): SubtitleTrack[] {
  return timeline?.tracks.filter((track): track is SubtitleTrack => track.kind === 'subtitle') ?? [];
}
/** The subtitle tracks checked at first: the one shown in the viewer, as a timeline shows one at a time. */
export function shownSubtitleTracks(timeline: Timeline | null): string[] {
  return subtitleTracksOf(timeline).filter(track => track.enabled).map(track => track.id);
}

/**
 * The frames between the In and Out points of the timeline, the Out frame among them, as the engine
 * takes a range: half open (`to` is the frame after the Out, as the Out mark is kept), not empty,
 * and inside the timeline. An In alone goes to the end, an Out alone from the start.
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
 * quality, and which of its subtitle tracks are saved beside the film as SRT: a checkbox for each,
 * shown in the viewer or not, as the Deliver page of DaVinci Resolve has them (Reference Manual 20,
 * p. 1102); the one shown is checked at first. The file is chosen in Save As, which
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
  const [codec, setCodec] = useState<RenderCodec>('h264');
  const [quality, setQuality] = useState<RenderQuality>('high');
  // The subtitle tracks checked, for the timeline chosen: another timeline starts from its shown one.
  const [subtitles, setSubtitles] = useState<{ timeline: string; tracks: string[] }>(() => ({ timeline: timelineId, tracks: shownSubtitleTracks(timeline) }));
  const tracks = subtitleTracksOf(timeline);
  const checked = subtitles.timeline === timelineId ? subtitles.tracks : shownSubtitleTracks(timeline);
  const check = (id: string, on: boolean) => setSubtitles({ timeline: timelineId, tracks: on ? [...checked.filter(each => each !== id), id] : checked.filter(each => each !== id) });
  const [pending, setPending] = useState(false);
  const [refused, setRefused] = useState<{ reason: RenderRefusal; detail: string } | null>(null);
  const marked = part === 'marked' && range !== null;
  const empty = timeline === null || duration(timeline) === 0;

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (timeline === null || empty || running || pending) return;
    setPending(true);
    setRefused(null);
    const started = await onStart({
      // In the order of the tracks of the timeline, as the engine writes them.
      timeline: timeline.id, name: timeline.name, codec, quality, subtitle_tracks: tracks.filter(track => checked.includes(track.id)).map(track => track.id),
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
          {range !== null && timeline !== null && <span className="mono caption" data-test="range-marks">{timecode(range.from, timeline.rate)} – {timecode(outFrame(range.to), timeline.rate)}</span>}
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
      <select id="render-quality" className="field" value={quality} onChange={event => setQuality(event.target.value as RenderQuality)} data-test="render-quality">
        {QUALITIES.map(each => <option key={each} value={each}>{labels[each === 'small' ? 'low' : each]}</option>)}
      </select>

      {tracks.length > 0 && <fieldset className="render-choice" data-test="render-subtitles">
        <legend className="setting-name">{labels.subtitles}</legend>
        {tracks.map(track => <label key={track.id} className="render-check">
          <input type="checkbox" checked={checked.includes(track.id)} onChange={event => check(track.id, event.target.checked)} data-test={`render-subtitle-${track.id}`} />
          {track.name}{track.language && <span className="caption"> · {track.language}</span>}
        </label>)}
        <p className="caption">{hints.subtitles}</p>
      </fieldset>}

      {running && <p className="caption" role="status" data-test="render-running">{hints.running}</p>}
      {refused && <p className="render-problem" role="alert" data-test="render-refused">
        {hints.refused[refused.reason]}
        {refused.detail && <span className="caption" lang="en" title={refused.detail}>{refused.detail}</span>}
      </p>}
    </form>
  </Sheet>;
}
