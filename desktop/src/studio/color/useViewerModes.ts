import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Target, type TimelineTransportState } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { useBypassColor } from '../BypassColor';

/** The Highlight modes of the viewer (pp. 2986, 3170–3171): off, Highlight, and Highlight BW; Difference is not done. */
export type HighlightMode = 'off' | 'highlight' | 'bw';

/** `value` written with the keys of its objects in order, so that the same target is the same words whoever wrote it. */
function keyed(value: unknown): string {
  return JSON.stringify(value, (_, each: unknown) => (each !== null && typeof each === 'object' && !Array.isArray(each)
    ? Object.fromEntries(Object.entries(each as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
    : each));
}

/**
 * The Highlight modes and Bypass Color of the Color page's viewer (Reference Manual 20, pp. 2986–
 * 2987, 3170–3171; the interaction reference, §1.2):
 *
 * - **Shift+H** turns Highlight on and off, **Alt+Shift+H** Highlight BW: the key of the current
 *   node, its picture where the key is and grey where it is not, or the key in black and white.
 *   The engine is told the grade and the node (`viewer.highlight`), again whenever another is
 *   current; a node of no key of its own is shown alone (p. 3171), which the engine does.
 *   Highlight is of the Color page alone: the page shows the mode that the monitor tells, and
 *   turns it off as it is left, so that the Cut and Edit pages, which share the timeline
 *   monitor, never show a key.
 * - **Shift+D** turns Bypass Color on and off (`viewer.bypass`, `useBypassColor`): the picture
 *   without its grades. It is the monitor's and is kept from page to page, as DaVinci Resolve's
 *   is (pp. 504, 550, 672); the Edit and Cut pages show it and turn it too.
 *
 * The modes are of the monitor: the state of the transport tells them, and they are shown as it
 * tells them.
 */
export function useViewerModes({ bridge, session, loaded, state, grade, node, onFailure }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  /** Whether a timeline is loaded in the monitor. */
  loaded: boolean;
  state: TimelineTransportState | null;
  /** The grade shown and its current node, whose key a Highlight mode shows. */
  grade: Target | null;
  node: string | null;
  onFailure: (failure: unknown) => void;
}) {
  const [highlight, setHighlight] = useState<HighlightMode>('off');
  const { bypass, toggleBypass } = useBypassColor({ bridge, session, loaded, state, onFailure });
  const now = useRef({ bridge, session, loaded, onFailure, highlight });
  now.current = { bridge, session, loaded, onFailure, highlight };
  // A mode taken from the monitor that this render does not have yet: nothing is sent until it does.
  const taking = useRef<HighlightMode | null>(null);
  // What the monitor was last told, or last told us it shows: `off`, the key of a mode, or null where nothing is known.
  const sentKey = useRef<string | null>(null);
  const shown = state?.highlight ?? null;
  const shownKey = shown === null ? 'off' : keyed([shown.mode, shown.grade, shown.node]);

  // Another engine, or a timeline taken away: nothing is on.
  useEffect(() => { setHighlight('off'); sentKey.current = null; }, [session]);
  // The mode that the monitor shows, set by this page or another client: shown, and not told back.
  useEffect(() => {
    if (state === null) return;
    sentKey.current = shownKey;
    const mode = shown === null ? 'off' : shown.mode;
    if (mode !== now.current.highlight) { taking.current = mode; setHighlight(mode); }
  }, [shownKey, state === null]);

  // A timeline loaded anew is told the mode again.
  useEffect(() => { if (!loaded) sentKey.current = null; }, [loaded]);
  // The mode, and the node it shows, as they change.
  const target = keyed(grade);
  useEffect(() => {
    const at = now.current;
    if (taking.current !== null && taking.current !== highlight) return;
    taking.current = null;
    if (!at.loaded) return;
    const on = highlight !== 'off' && grade !== null && node !== null;
    const key = on ? keyed([highlight, grade, node]) : 'off';
    if (key === sentKey.current) return;
    if (!on && sentKey.current === null) { sentKey.current = 'off'; return; }
    sentKey.current = key;
    void callEngine(at.bridge, 'viewer.highlight', on ? { mode: highlight, grade, node } : { mode: 'off' }, { expectedSession: at.session })
      .catch((failure) => { if (sentKey.current === key) sentKey.current = null; at.onFailure(failure); });
  }, [highlight, target, node, loaded]);
  // The page left: the monitor shows no key on another page.
  useEffect(() => () => {
    const at = now.current;
    if (!at.loaded || sentKey.current === null || sentKey.current === 'off') return;
    sentKey.current = 'off';
    void callEngine(at.bridge, 'viewer.highlight', { mode: 'off' }, { expectedSession: at.session }).catch(at.onFailure);
  }, []);

  const choose = useCallback((mode: HighlightMode) => { taking.current = null; setHighlight(mode); }, []);

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || key.repeat || key.ctrl || key.meta || !key.shift) return;
      const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
      if (letter === 'h') {
        event.preventDefault();
        // Shift+H: Highlight on or off; Alt+Shift+H: Highlight BW on or off (p. 3171).
        const wanted: HighlightMode = key.alt ? 'bw' : 'highlight';
        taking.current = null;
        setHighlight(mode => (mode === wanted ? 'off' : wanted));
      }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  return { highlight, setHighlight: choose, bypass, toggleBypass };
}
