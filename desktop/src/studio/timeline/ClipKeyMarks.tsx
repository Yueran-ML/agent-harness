import { createContext, useContext, useMemo, type CSSProperties } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Media, Rate } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { settingNames } from '../inspector/clipKeys';
import { timecode } from '../timecode';
import { keyMarks } from './keyTrack';

/**
 * What a page that shows the keys of clips on its timeline gives it: the clips whose keyframe
 * tracks are open, which the page keeps (not the project), and what opens or closes one. `onToggle`
 * must stay the same call.
 */
export interface KeyTracksState { open: readonly string[]; onToggle: (clip: string) => void }

/** What the clips of a timeline that shows keys read: which keyframe tracks are open, and how to open one and move the playhead. */
export interface ClipKeyTracksContext {
  open: ReadonlySet<string>;
  onToggle: (clip: string) => void;
  onSeek: (frame: number, scrubbing: boolean) => void;
}
/**
 * Null on a timeline that shows no keys of clips. Only a clip that has keys reads it
 * (`ClipKeyMarks`), so a clip without keys is drawn again for nothing here.
 *
 * Left for later: the timelines of the Cut page show no keys yet; a page that gives its `Timeline`
 * the `keyTracks` of `useKeyTracks` gets the diamonds and the keyframe tracks as the Edit page has them.
 */
export const ClipKeyTracks = createContext<ClipKeyTracksContext | null>(null);

/**
 * The keys of a clip of picture drawn on it, as DaVinci Resolve marks a keyframed clip in the
 * timeline (Reference Manual 20, pp. 1108–1109): along its lower edge a diamond at each frame of
 * the timeline at which a setting of its transform or crop has a key (`keyMarks`, through the
 * clip's speed and direction), and at the right of its name the Keyframe button, which opens and
 * closes its keyframe track (`KeyTrack`). The button shows while the pointer is on the clip, while
 * the clip is chosen and while its track is open.
 *
 * A diamond says which settings have a key there and at what timecode, and a press on it moves the
 * playhead there. The press goes on to the clip, which is chosen as by any press on it. The
 * playhead moves at the press, not at a click: the timeline takes the pointer for a drag of the
 * clip as it is pressed, and no click comes to the diamond after that. With the blade the press
 * cuts, as everywhere on the clip, and the playhead stays.
 *
 * `ClipView` mounts this only for a clip that has keys, so one without keys has no part of it. The
 * diamonds are placed by the style sheet from their frames (`--at`, times the `--scale` of the
 * timeline), so a zoom draws nothing again.
 *
 * Left for later: the Curve button beside the Keyframe button (p. 1108; the Curve Editor opens from
 * the tool bar of the timeline for now), and the keys of a clip of sound (its volume has the line
 * of `VolumeLine`; DaVinci Resolve shows them in the same keyframe tracks).
 */
export function ClipKeyMarks({ clip, media, rate }: { clip: Clip; media: Media | undefined; rate: Rate }) {
  const tracks = useContext(ClipKeyTracks);
  const words = useWords();
  const t = useFairlightText();
  const shown = tracks !== null;
  const marks = useMemo(() => (shown ? keyMarks(clip, { rate }, media ? [media] : []) : []), [shown, clip, rate, media]);
  if (!tracks) return null;
  const names = settingNames(words.label.studio.inspector);
  const open = tracks.open.has(clip.id);
  const label = t('Keyframe Editor');
  // The press on the button stays with it: the timeline would take it for a drag of the clip, and no click would come.
  return <>
    <button type="button" className="clip-keys-toggle" aria-label={label} title={label} aria-pressed={open} data-test="clip-keys-toggle"
      onPointerDown={event => event.stopPropagation()} onDoubleClick={event => event.stopPropagation()}
      onClick={(event) => { event.stopPropagation(); tracks.onToggle(clip.id); }}>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M5,0.5 L9.5,5 L5,9.5 L0.5,5 Z" fill="currentColor" /></svg>
    </button>
    {marks.map(mark => <span key={mark.at} className="clip-key" data-clip-key={mark.at} style={{ '--at': mark.at - clip.start } as CSSProperties}
      title={`${mark.paths.map(path => names[path]).join(', ')} · ${timecode(mark.at, rate)}`}
      onPointerDown={(event) => {
        if (event.button !== 0 || event.currentTarget.closest('.timeline')?.getAttribute('data-tool') === 'blade') return;
        tracks.onSeek(mark.at, false);
      }} />)}
  </>;
}
