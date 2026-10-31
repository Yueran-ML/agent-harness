import { useState, type FormEvent } from 'react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import { emptyGrade, type Media, type Rate, type Timeline, type Track } from '../engine';
import { rateText } from '../timecode';
import { RATES, rateKey } from '../inspector/TimelineSettings';

/** The settings of a timeline that has nothing to go by. */
const FALLBACK = { width: 1920, height: 1080, rate: { num: 25, den: 1 } } as const;
const SAMPLE_RATE = 48000;

/** A size of picture that the engine takes: even, from 16 to 16384 pixels. */
const even = (pixels: number) => Math.min(16384, Math.max(16, Math.floor(pixels / 2) * 2));

/**
 * The settings of a new timeline: the size and frame rate of the first video in the pool (as it is
 * shown, so turned where the file says so; stills have no frame rate of their own), else 1920 × 1080
 * at 25 fps. The sound is at 48 kHz.
 */
export function timelineDefaults(media: readonly Media[]): Pick<Timeline, 'width' | 'height' | 'rate' | 'sample_rate'> {
  const video = media.map(each => each.facts.video).find(facts => facts && !facts.still && facts.rate.num > 0 && facts.rate.den > 0 && facts.width > 0 && facts.height > 0);
  if (!video) return { ...FALLBACK, rate: { ...FALLBACK.rate }, sample_rate: SAMPLE_RATE };
  const turned = Math.abs(video.rotation) % 180 === 90;
  return {
    width: even(turned ? video.height : video.width),
    height: even(turned ? video.width : video.height),
    rate: { num: video.rate.num, den: video.rate.den },
    sample_rate: SAMPLE_RATE,
  };
}

/** A new timeline with one video track and one audio track, its id and theirs given by the engine. */
export function newTimeline(name: string, settings: Pick<Timeline, 'width' | 'height' | 'rate' | 'sample_rate'>): Timeline {
  const track = (kind: 'video' | 'audio', label: string): Track => ({ id: '', kind, name: label, enabled: true, locked: false, clips: [], transitions: [] });
  return { id: '$timeline', name, ...settings, tracks: [track('video', 'V1'), track('audio', 'A1')], grade: emptyGrade() };
}

const settingsText = (settings: { width: number; height: number; rate: Rate; sample_rate: number }, fps: (rate: string) => string) =>
  `${settings.width} × ${settings.height} · ${fps(rateText(settings.rate))} · ${settings.sample_rate / 1000} kHz`;

/** `Timeline N`, for the first N from `count + 1` that names no timeline that there is. */
export function freshName(title: string, names: readonly string[]): string {
  const taken = new Set(names.map(name => name.trim()));
  let number = names.length + 1;
  while (taken.has(`${title} ${number}`)) number++;
  return `${title} ${number}`;
}

/**
 * The dialog of a new timeline, as DaVinci Resolve asks for one: its name and its frame rate, which
 * a timeline can have of its own when it is made (Reference Manual 20, p. 698), with the other
 * settings that it will have. The rate is that of the first video in the pool until another is
 * chosen; once the timeline has content it is locked. It lies over the page, so the views of both
 * monitors are hidden while it is open.
 */
export function NewTimeline({ media, names, onCreate, onClose }: {
  media: readonly Media[];
  /** The names of the timelines that there are: the new one is named after none of them. */
  names: readonly string[];
  /** Create it; what failed is said in the dialog, which stays open. */
  onCreate: (timeline: Timeline) => Promise<string | null>;
  onClose: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio;
  const [name, setName] = useState(() => freshName(labels.timeline.title, names));
  const [pending, setPending] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const defaults = timelineDefaults(media);
  const rates = RATES.some(each => rateKey(each) === rateKey(defaults.rate)) ? RATES : [...RATES, defaults.rate];
  const [rate, setRate] = useState(() => rateKey(defaults.rate));
  const chosenRate = rates.find(each => rateKey(each) === rate) ?? defaults.rate;
  const settings = { ...defaults, rate: { num: chosenRate.num, den: chosenRate.den } };
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    const named = name.trim();
    if (!named || pending) return;
    setPending(true);
    setProblem(null);
    const failed = await onCreate(newTimeline(named, settings));
    setPending(false);
    if (failed !== null) setProblem(failed);
  };
  return <Sheet
    title={labels.editPage.newTimeline}
    onClose={onClose}
    foot={<>
      <button type="button" className="btn" onClick={onClose}>{words.label.cancel}</button>
      <button type="button" className="btn btn-main" disabled={!name.trim() || pending} onClick={() => void submit()} data-test="create-timeline">{labels.editPage.create}</button>
    </>}
  >
    <form className="new-timeline" onSubmit={event => void submit(event)}>
      <label className="setting-name" htmlFor="new-timeline-name">{labels.media.name}</label>
      <input id="new-timeline-name" className="field" value={name} data-first autoComplete="off" spellCheck={false}
        onChange={event => setName(event.target.value)} />
      <label className="setting-name" htmlFor="new-timeline-rate">{labels.inspector.frameRate}</label>
      <select id="new-timeline-rate" className="field" value={rate} onChange={event => setRate(event.target.value)} data-test="new-timeline-rate">
        {rates.map(each => <option key={rateKey(each)} value={rateKey(each)}>{labels.fps(rateText(each))}</option>)}
      </select>
      <p className="mono new-timeline-settings" data-test="timeline-settings">{settingsText(settings, labels.fps)}</p>
      <p className="caption">{words.hint.studio.newTimeline}</p>
      {problem && <p className="new-timeline-problem" role="alert">{words.hint.studio.newTimelineFailed}<span className="caption" lang="en">{problem}</span></p>}
    </form>
  </Sheet>;
}
