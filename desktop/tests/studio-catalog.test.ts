/**
 * The bounds that the palettes of the Color page keep for the settings of a corrector are those of
 * the engine's catalog of the settings that take keys (`engine/crates/model/src/catalog.rs`), which
 * holds the bounds that the engine checks a setting against. The catalog is written by the engine
 * (`engine/crates/model/tests/catalog.rs`).
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { BLUR_BOUNDS } from '../src/studio/color/blur';
import { BOUNDS, type Bounds } from '../src/studio/color/corrector';

interface Entry { path: string; default: number; low: number; high: number; unit: string; page: string; palette: string; clock: string }
const catalog: { corrector: Entry[] } = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/catalog.json', import.meta.url), 'utf8'));

const primaries: Record<string, Bounds> = BOUNDS;
const blur: Record<string, Bounds> = BLUR_BOUNDS;

/** The name under which the palettes keep the bounds of a setting, or none where they keep none of their own. */
function keptAs(path: string): { table: Record<string, Bounds>; name: string } | undefined {
  const [palette, name] = path.split('.');
  switch (palette) {
    case 'primaries': return { table: primaries, name };
    case 'log': return { table: primaries, name: name.endsWith('_range') ? name : 'log_wheel' };
    case 'curves': return { table: primaries, name: 'intensity' };
    case 'soft_clip': return { table: primaries, name: name.endsWith('_soft') ? 'clip_soft' : `clip_${name}` };
    case 'blur': return { table: blur, name };
    default: return undefined;
  }
}

describe('the bounds of the settings of a corrector', () => {
  it('are those of the catalog of the engine, for every setting that a palette keeps bounds of', () => {
    const used = new Set<Bounds>();
    for (const entry of catalog.corrector) {
      const kept = keptAs(entry.path);
      if (!kept) continue;
      const bounds = kept.table[kept.name];
      expect(bounds, entry.path).toEqual({ neutral: entry.default, low: entry.low, high: entry.high });
      used.add(bounds);
    }
    // And the palettes keep no bounds of a setting that the engine has not.
    for (const bounds of [...Object.values(primaries), ...Object.values(blur)]) expect(used.has(bounds)).toBe(true);
  });

  it('are listed for every palette, each setting once, with keys on the frames of the source', () => {
    const paths = catalog.corrector.map(entry => entry.path);
    expect(new Set(paths).size).toBe(paths.length);
    expect(new Set(catalog.corrector.map(entry => entry.palette))).toEqual(
      new Set(['primaries', 'log', 'curves', 'soft_clip', 'key', 'blur', 'shot_match', 'qualifier', 'window']),
    );
    expect(catalog.corrector.every(entry => entry.page === 'color' && entry.clock === 'source_frame')).toBe(true);
    // A place in a list is a star: the settings of every window are those of one.
    const pan = catalog.corrector.find(entry => entry.path === 'windows.*.transform.pan')!;
    expect([pan.default, pan.low, pan.high, pan.unit]).toEqual([50, -1000, 1000, 'number']);
    expect(catalog.corrector.find(entry => entry.path === 'windows.*.transform.rotate')!.unit).toBe('degrees');
  });
});
