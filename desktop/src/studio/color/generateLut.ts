import { useCallback, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { EngineError, type LutExported, type Media, type Timeline } from '../engine';
import type { MenuItem } from './ColorMenu';

/** The cubes that Generate LUT makes (Reference Manual 20, p. 3413); Panasonic VLUT is not done. */
export const LUT_SIZES = [17, 33, 65] as const;
export type LutSize = typeof LUT_SIZES[number];

/** What is said of a LUT generated, or of why none was. */
export type LutNotice = { kind: 'done'; path: string; ignored: number } | { kind: 'failed'; message: string };

/**
 * Generate LUT ▸ of a thumbnail's right-click menu (p. 3413): the clip's grade baked into a cube of
 * 17, 33 or 65 points, written where Save As says (the shell's dialog, `exportLut`), named after the
 * clip. The nodes with a qualifier, a window or a filter are left out, as DaVinci Resolve leaves
 * them; how many were is said with where the cube went. The keys of the grade are read at the
 * playhead where the clip is the current one, else at its first frame.
 */
export function useGenerateLut({ bridge, language, timeline, media, current, frame, words }: {
  bridge: Partial<Pick<EngineBridge, 'exportLut'>>;
  language: string;
  timeline: Timeline | null;
  media: ReadonlyMap<string, Media>;
  /** The current clip, and the playhead of the timeline. */
  current: string | null;
  frame: number | null;
  words: { generateLut: string; sizes: (size: number) => string };
}) {
  const [notice, setNotice] = useState<LutNotice | null>(null);
  const now = useRef({ bridge, language, timeline, media, current, frame, words });
  now.current = { bridge, language, timeline, media, current, frame, words };
  const generate = useCallback((clip: string, size: LutSize) => {
    const { bridge: shell, language: spoken, timeline: shown, media: pool, current: currentClip, frame: playhead } = now.current;
    if (!shell.exportLut || !shown) return;
    const found = shown.tracks.flatMap(track => ('clips' in track ? track.clips : [])).find(each => each.id === clip);
    if (!found) return;
    const name = found.source.kind === 'media' ? pool.get(found.source.media)?.name.replace(/\.[^.]+$/, '') ?? 'LUT'
      : found.source.kind === 'title' ? found.source.text : 'LUT';
    setNotice(null);
    void shell.exportLut({
      language: spoken, grade: { level: 'clip', timeline: shown.id, clip }, size, name,
      ...(clip === currentClip && playhead !== null ? { frame: playhead } : {}),
    }).then((answer) => {
      if (!answer.ok) { setNotice({ kind: 'failed', message: new EngineError(answer.error).message }); return; }
      const made: LutExported | null = answer.result;
      if (made) setNotice({ kind: 'done', path: made.path, ignored: made.ignored.length });
    }, (failure: unknown) => setNotice({ kind: 'failed', message: failure instanceof Error ? failure.message : String(failure) }));
  }, []);
  const items = useCallback((clip: string): MenuItem[] => (now.current.bridge.exportLut ? [{
    id: 'generate-lut', label: now.current.words.generateLut,
    items: LUT_SIZES.map(size => ({ id: `generate-lut-${size}`, label: now.current.words.sizes(size), run: () => generate(clip, size) })),
  }] : []), [generate]);
  return { items, notice, dismiss: () => setNotice(null) };
}
