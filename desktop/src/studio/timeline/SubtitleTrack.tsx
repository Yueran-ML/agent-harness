import { memo, useEffect, useRef, useState } from 'react';
import { Plus } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { SubtitleTrack } from '../engine';
import { spanStyle, visible } from './geometry';

/**
 * The subtitles of a track of subtitles that lie in the window `from`–`to` (frames), each placed by
 * the style sheet from its frames, and the one chosen shown so. A subtitle is moved by its middle
 * and made longer or shorter by its edges (see `useEditing`); the timeline on screen changes only as
 * the engine tells of it.
 */
export const SubtitleCues = memo(function SubtitleCues({ track, from, to, chosen }: {
  track: SubtitleTrack; from: number; to: number;
  /** The subtitle chosen, where it is on this track. */
  chosen: string | null;
}) {
  return <>{visible(track.cues, from, to).map(cue => <div key={cue.id} className="timeline-cue" data-cue={cue.id} role="option"
    aria-selected={chosen === cue.id} data-selected={chosen === cue.id} style={spanStyle(cue.start, cue.length)} title={cue.text}>{cue.text}</div>)}</>;
});

/** The languages offered for a new track of subtitles, as short tags of BCP 47, as the engine keeps them. */
export const LANGUAGES = ['zh', 'en', 'ja', 'ko', 'fr', 'de', 'es', 'it', 'pt', 'ru'] as const;

/** A language by its name in the language of the page, or its tag where the system has no name for it. */
function languageName(tag: string, locale: string): string {
  try { return new Intl.DisplayNames([locale], { type: 'language' }).of(tag) ?? tag; } catch { return tag; }
}

/**
 * The "+" over the headers of the tracks: a track of subtitles is added in the language chosen,
 * as DaVinci Resolve's Add Subtitle Track. The language of the page is offered first.
 */
export function AddSubtitleTrack({ onAdd, disabled }: { onAdd: (language: string) => void; disabled: boolean }) {
  const words = useWords().label.studio.timeline;
  const [open, setOpen] = useState(false);
  const locale = document.documentElement.lang || 'en';
  const page = locale.split('-')[0].toLowerCase();
  const [language, setLanguage] = useState<string>((LANGUAGES as readonly string[]).includes(page) ? page : '');
  const box = useRef<HTMLDivElement>(null);
  // A press anywhere else puts it away, as a menu goes.
  useEffect(() => {
    if (!open) return undefined;
    const away = (event: PointerEvent) => { if (!(event.target instanceof Node) || !box.current?.contains(event.target)) setOpen(false); };
    window.addEventListener('pointerdown', away, true);
    return () => window.removeEventListener('pointerdown', away, true);
  }, [open]);
  return <div ref={box} className="timeline-add-track" onPointerDown={event => event.stopPropagation()}
    onKeyDown={event => { if (event.key === 'Escape' && open) { event.preventDefault(); event.stopPropagation(); setOpen(false); } }}>
    <button type="button" className="icon-btn" aria-label={words.addTrack} title={words.addTrack} aria-expanded={open} disabled={disabled}
      onClick={() => setOpen(value => !value)} data-test="add-track"><Plus size={14} /></button>
    {open && <div className="timeline-add-track-menu" role="dialog" aria-label={words.addSubtitleTrack} data-test="add-subtitle-track">
      <label>
        <span>{words.language}</span>
        <select className="field" value={language} onChange={event => setLanguage(event.target.value)} data-test="subtitle-language">
          <option value="">{words.noLanguage}</option>
          {LANGUAGES.map(tag => <option key={tag} value={tag}>{languageName(tag, locale)}</option>)}
        </select>
      </label>
      <button type="button" className="btn" onClick={() => { setOpen(false); onAdd(language); }} data-test="add-subtitle-track-confirm">{words.addSubtitleTrack}</button>
    </div>}
  </div>;
}
