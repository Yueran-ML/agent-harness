import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Target, type TimelineTransportState } from '../engine';
import { editingAllowed, inStudio } from '../keys';

/** The Highlight modes of the viewer (pp. 2986, 3170–3171): off, Highlight, and Highlight BW; Difference is not done. */
export type HighlightMode = 'off' | 'highlight' | 'bw';

/**
 * The Highlight modes and Bypass Color of the Color page's viewer (Reference Manual 20, pp. 2986–
 * 2987, 3170–3171; the interaction reference, §1.2):
 *
 * - **Shift+H** turns Highlight on and off, **Alt+Shift+H** Highlight BW: the key of the current
 *   node, its picture where the key is and grey where it is not, or the key in black and white.
 *   The engine is told the grade and the node (`viewer.highlight`), again whenever another is
 *   current; a node of no key of its own is shown alone (p. 3171), which the engine does.
 * - **Shift+D** turns Bypass Color on and off (`viewer.bypass`): the picture without its grades.
 *   The manual gives Shift-D on the Cut and Edit pages (pp. 504, 550) and no key on the Color
 *   page; the interaction reference takes it here too. This studio has no Fusion, so Bypass Color
 *   and Fusion is the one switch.
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
  const [bypass, setBypass] = useState(false);
  const now = useRef({ bridge, session, loaded, onFailure, highlight, bypass });
  now.current = { bridge, session, loaded, onFailure, highlight, bypass };
  const sentKey = useRef<string | null>(null);
  const shownBypass = state?.bypass ?? false;

  // Another engine, or a timeline taken away: nothing is on.
  useEffect(() => { setHighlight('off'); setBypass(false); sentKey.current = null; }, [session]);
  useEffect(() => { setBypass(shownBypass); }, [shownBypass]);

  // A timeline loaded anew is told the mode again.
  useEffect(() => { if (!loaded) sentKey.current = null; }, [loaded]);
  // The mode, and the node it shows, as they change.
  const target = JSON.stringify(grade);
  useEffect(() => {
    const at = now.current;
    if (!at.loaded) return;
    const on = highlight !== 'off' && grade !== null && node !== null;
    const key = on ? JSON.stringify([highlight, grade, node]) : 'off';
    if (key === sentKey.current) return;
    if (!on && sentKey.current === null) { sentKey.current = 'off'; return; }
    sentKey.current = key;
    void callEngine(at.bridge, 'viewer.highlight', on ? { mode: highlight, grade, node } : { mode: 'off' }, { expectedSession: at.session })
      .catch((failure) => { if (sentKey.current === key) sentKey.current = null; at.onFailure(failure); });
  }, [highlight, target, node, loaded]);

  const choose = useCallback((mode: HighlightMode) => setHighlight(mode), []);
  const toggleBypass = useCallback(() => {
    const at = now.current;
    if (!at.loaded) return;
    const on = !at.bypass;
    setBypass(on);
    void callEngine(at.bridge, 'viewer.bypass', { on }, { expectedSession: at.session }).then(answer => setBypass(typeof answer?.on === 'boolean' ? answer.on : on))
      .catch((failure) => { setBypass(!on); at.onFailure(failure); });
  }, []);

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
        setHighlight(mode => (mode === wanted ? 'off' : wanted));
      } else if (letter === 'd' && !key.alt) {
        event.preventDefault();
        toggleBypass();
      }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [toggleBypass]);

  return { highlight, setHighlight: choose, bypass, toggleBypass };
}
