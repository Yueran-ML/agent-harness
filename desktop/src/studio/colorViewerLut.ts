/**
 * The Color Viewer LUT on the Color page alone: "The Color Viewer Lookup Table options in the Color
 * Management panel of the Project Settings only affect the GUI Viewer in the Color page. They do not
 * affect the viewers in the Edit page" (Reference Manual 20, p. 672). The engine's timeline monitor
 * is shown on every page and does not know which, and shows the LUT until it is told not to: where
 * the project has a Color Viewer LUT, the engine is told whether its view shows it as the page
 * changes (`viewer.lut`, PROTOCOL.md "Live Preview and the Color Viewer LUT"); where it has none,
 * there is nothing to tell, and nothing is sent.
 */
import { useEffect, useRef } from 'react';
import type { EngineBridge } from '../bridge';
import { callEngine, type ColorSettings } from './engine';

/** Whether the colour settings of a project name a Color Viewer LUT. */
export function hasViewerLut(color: Partial<ColorSettings> | undefined): boolean {
  return Boolean(color?.viewer_lut?.['1d'] || color?.viewer_lut?.['3d']);
}

export function useColorViewerLut(bridge: Pick<EngineBridge, 'call'>, native: boolean, session: number | null, onColorPage: boolean, hasLut: boolean): void {
  // What the engine of each session was told last: on (as it is at first) or off.
  const told = useRef<{ session: number | null; on: boolean }>({ session: null, on: true });
  useEffect(() => {
    if (!native || session === null) return;
    if (told.current.session !== session) told.current = { session, on: true };
    if (!hasLut || told.current.on === onColorPage) return;
    const asked = { session, on: onColorPage };
    told.current = asked;
    void callEngine(bridge, 'viewer.lut', { on: onColorPage }).catch(() => {
      // Told again with the next change; an engine that has no monitor shows no LUT anyway.
      if (told.current === asked) told.current = { session, on: !onColorPage };
    });
  }, [bridge, native, session, onColorPage, hasLut]);
}
