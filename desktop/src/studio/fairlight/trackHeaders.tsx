import { useCallback, useRef, type PointerEvent as ReactPointerEvent } from 'react';
import type { Track } from '../engine';
import type { MenuItem } from '../color/ColorMenu';
import { Meter as AudioMeter } from '../audio/Meter';
import { sameChannels, stripChannels, useMeterReading, type MeterSource } from './audioReadings';
import { channelNames } from './ChannelLanes';
import { TRACK_COLORS } from './IndexTracks';
import { HEIGHT_PRESETS, type FairlightView, type FairlightViewUpdate, type HeightPreset } from './viewState';
import { useFairlightText } from './words';
import type { FairlightText } from '../../messages/fairlight';
import type { Change } from '../timeline/commands';

/** The default height of a Fairlight track, which a lock to Custom takes where none was set. */
export const FAIRLIGHT_TRACK_HEIGHT = 128;

const PRESET_NAMES: Record<HeightPreset, FairlightText> = {
  micro: 'Micro', mini: 'Mini', medium: 'Medium size', large: 'Large', extraLarge: 'Extra Large', custom: 'Custom',
};

/** The format of a track as its header shows it (Reference Manual 20, p. 3669). */
export function formatLabel(track: Track, t: (key: FairlightText) => string): string {
  if (track.kind !== 'audio') return '';
  if (track.format === 'mono') return t('Mono');
  if (track.format === '5.1') return track.order === 'film' ? '5.1 Film' : '5.1';
  if (track.format === 'adaptive') return `${t('Adaptive')} ${track.channels ?? 1}`;
  return t('Stereo');
}

/** A track's meter in its header: drawn again at its own readings, never with the row (p. 3669). */
function HeaderMeter({ source, track }: { source: MeterSource; track: Track }) {
  const t = useFairlightText();
  const channels = useMeterReading(source, meters => stripChannels(meters, { track: track.id }, false), sameChannels);
  return <span className="fairlight-header-meter">
    <AudioMeter label={`${t('Track meter')} ${track.name}`} values={channels ?? []} channels={channelNames(track).length} />
  </span>;
}

/** The number, format and meter of a track, beside its name in the header. */
export function HeaderDetail({ track, meters }: { track: Track; meters: MeterSource }) {
  const t = useFairlightText();
  return <span className="fairlight-header-detail" style={{ borderColor: track.color ?? TRACK_COLORS[8] }}>
    <span className="fairlight-track-number" data-track-number={track.id}>{track.id}</span>
    {track.kind === 'audio' && <span className="fairlight-track-format" data-track-format>{formatLabel(track, t)}</span>}
    {track.kind === 'audio' && <HeaderMeter source={meters} track={track} />}
  </span>;
}

/**
 * The Fairlight items of a header's right-click menu: Change Track Color and Lock Track Height To,
 * for the tracks chosen where the header is one of them, else for its track alone (p. 3670).
 */
export function headerMenuItems({ id, timeline, selected, view, update, onEdit, t }: {
  id: string; timeline: { id: string; tracks: readonly Track[] }; selected: readonly string[];
  view: FairlightView; update: FairlightViewUpdate; onEdit: (ops: Change[]) => void; t: (key: FairlightText) => string;
}): MenuItem[] {
  const track = timeline.tracks.find(each => each.id === id);
  if (!track) return [];
  const targets = selected.includes(id) ? timeline.tracks.filter(each => selected.includes(each.id)).map(each => each.id) : [id];
  const color = track.color ?? TRACK_COLORS[8];
  const lock = (preset: HeightPreset | null) => update(old => {
    const heights = { ...old.timelineViewport.heights };
    const heightLocks = { ...old.heightLocks };
    for (const target of targets) {
      if (preset === null) { delete heightLocks[target]; continue; }
      heightLocks[target] = preset;
      const fixed = HEIGHT_PRESETS.find(([name]) => name === preset)?.[1];
      heights[target] = fixed ?? heights[target] ?? FAIRLIGHT_TRACK_HEIGHT;
    }
    return { heightLocks, timelineViewport: { ...old.timelineViewport, heights } };
  });
  const locked = view.heightLocks[id] ?? null;
  return [
    {
      id: 'track-color', label: t('Change Track Color'),
      items: TRACK_COLORS.map((value, at) => ({
        id: `track-color-${at + 1}`, label: `${t('Color')} ${at + 1}`, checked: value === color,
        run: () => onEdit(targets.map(target => ({ op: 'set_track', timeline: timeline.id, id: target, color: value }))),
      })),
    },
    {
      id: 'lock-height', label: t('Lock Track Height To'),
      items: [
        ...[...HEIGHT_PRESETS.map(([name]) => name), 'custom' as const].map(preset => ({
          id: `lock-height-${preset}`, label: t(PRESET_NAMES[preset]), checked: locked === preset, run: () => lock(preset),
        })),
        { id: 'lock-height-none', label: t('None'), checked: locked === null, run: () => lock(null) },
      ],
    },
  ];
}

/** Whether a press lands on a control of the header (a switch, the name being written, the edge). */
const onControl = (target: EventTarget) => target instanceof Element
  && target.closest('.timeline-track-switches, .timeline-track-resize, input, select, textarea, [data-clip]') !== null;

/**
 * Choosing tracks as Fairlight does (Reference Manual 20, p. 3747): a click on a header or a
 * track's background chooses it; Ctrl+click adds or takes away one; Shift+click chooses those
 * between; a drag down the headers chooses the tracks it passes. The order is the order shown.
 */
export function useTrackChoice({ selected, onTracks, onTrack }: {
  selected: readonly string[]; onTracks?: (ids: string[]) => void; onTrack?: (id: string) => void;
}) {
  const now = useRef({ selected, onTracks, onTrack });
  now.current = { selected, onTracks, onTrack };
  const choose = (ids: string[]) => { now.current.onTracks?.(ids); };
  // The track a Shift+click goes from, a sweep in progress, and a press the timeline should not take again.
  const anchor = useRef<string | null>(null);
  const sweep = useRef<string | null>(null);
  const taken = useRef(false);
  const order = (from: Element) => [...(from.closest('.fairlight-timeline')?.querySelectorAll<HTMLElement>('.timeline-track[data-track]') ?? [])]
    .map(row => row.dataset.track!);
  const between = (from: Element, a: string, b: string) => {
    const ids = order(from);
    const [low, high] = [ids.indexOf(a), ids.indexOf(b)].sort((x, y) => x - y);
    return low < 0 ? [b] : ids.slice(low, high + 1);
  };
  const pointerDown = useCallback((event: ReactPointerEvent) => {
    if (event.button !== 0 || !(event.target instanceof Element) || onControl(event.target)) return;
    const row = event.target.closest<HTMLElement>('.timeline-track[data-track]');
    if (!row || !now.current.onTracks) return;
    const id = row.dataset.track!;
    const { selected: chosen } = now.current;
    if (event.ctrlKey || event.metaKey) {
      taken.current = true;
      anchor.current = id;
      choose(chosen.includes(id) ? chosen.filter(each => each !== id) : [...chosen, id]);
    } else if (event.shiftKey && anchor.current) {
      taken.current = true;
      choose(between(event.target, anchor.current, id));
    } else {
      anchor.current = id;
      if (event.target.closest('.timeline-track-header')) sweep.current = id;
    }
  }, []);
  const pointerOver = useCallback((event: ReactPointerEvent) => {
    const start = sweep.current;
    if (start === null || !(event.target instanceof Element)) return;
    if ((event.buttons & 1) === 0) { sweep.current = null; return; }
    const id = event.target.closest('.timeline-track-header')?.closest<HTMLElement>('.timeline-track[data-track]')?.dataset.track;
    if (id) choose(between(event.target, start, id));
  }, []);
  // A press taken here that the timeline did not see (a lane stopped it) is forgotten as it ends.
  const pointerUp = useCallback(() => { sweep.current = null; taken.current = false; }, []);
  /** What the timeline is told of a press: passed on unless the press was taken here. */
  const trackSelect = useCallback((id: string) => {
    if (taken.current) { taken.current = false; return; }
    now.current.onTrack?.(id);
  }, []);
  /** Ctrl+Alt+Up and Down: the choice moves up or down a track; with none chosen, nothing (p. 3748). */
  const move = useCallback((from: Element, direction: -1 | 1) => {
    const { selected: chosen, onTracks } = now.current;
    if (!chosen.length || !onTracks) return false;
    const ids = order(from);
    const places = chosen.map(id => ids.indexOf(id)).filter(at => at >= 0);
    if (!places.length) return false;
    const edge = direction < 0 ? Math.min(...places) : Math.max(...places);
    if (ids[edge + direction] === undefined) return true;
    choose(places.map(at => ids[at + direction]).filter((id): id is string => id !== undefined));
    return true;
  }, []);
  return { pointerDown, pointerOver, pointerUp, trackSelect, move };
}
