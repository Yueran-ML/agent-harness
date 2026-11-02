import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../app/keys';
import type { EngineBridge } from '../bridge';
import { callEngine, type TimelineTransportState } from './engine';
import { editingAllowed, inStudio } from './keys';

/**
 * Bypass Color of the timeline monitor (`viewer.bypass`; Reference Manual 20, pp. 504, 550, 672,
 * 2987): the picture without its grades. It is the monitor's, as DaVinci Resolve's Bypass Color
 * Grades and Fusion Effects is the viewer's on the Cut, Edit and Color pages alike: a page that
 * shows the timeline monitor shows it as the state of the transport tells it, turns it with
 * Shift+D and its button, and leaves it as it is when the page is left. This studio has no
 * Fusion, so Bypass Color and Fusion is the one switch.
 */
export function useBypassColor({ bridge, session, loaded, state, onFailure }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  /** Whether a timeline is loaded in the monitor. */
  loaded: boolean;
  state: TimelineTransportState | null;
  onFailure: (failure: unknown) => void;
}) {
  const [bypass, setBypass] = useState(false);
  const now = useRef({ bridge, session, loaded, onFailure, bypass });
  now.current = { bridge, session, loaded, onFailure, bypass };
  const shown = state?.bypass ?? false;

  useEffect(() => { setBypass(false); }, [session]);
  useEffect(() => { setBypass(shown); }, [shown]);

  const toggle = useCallback(() => {
    const at = now.current;
    if (!at.loaded) return;
    const on = !at.bypass;
    setBypass(on);
    void callEngine(at.bridge, 'viewer.bypass', { on }, { expectedSession: at.session })
      .then(answer => setBypass(typeof answer?.on === 'boolean' ? answer.on : on))
      .catch((failure) => { setBypass(!on); at.onFailure(failure); });
  }, []);

  // Shift+D (pp. 504, 550).
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || key.repeat || key.ctrl || key.meta || key.alt || !key.shift) return;
      const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
      if (letter !== 'd') return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [toggle]);

  return { bypass, toggleBypass: toggle };
}

/** The button of Bypass Color in a viewer's title bar (pp. 550, 672, 2985). */
export function BypassButton({ bypass, disabled, label, className, onToggle }: {
  bypass: boolean; disabled: boolean; label: string; className: string; onToggle: () => void;
}) {
  return <button type="button" className={className} aria-pressed={bypass} disabled={disabled} title={`${label} (Shift+D)`}
    onClick={onToggle} data-test="bypass-color">{label}</button>;
}
