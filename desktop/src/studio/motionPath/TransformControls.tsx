import { useCallback, useEffect, useState } from 'react';

/** Where the page keeps whether the on-screen Transform controls of the timeline viewer are on. */
export const TRANSFORM_CONTROLS_KEY = 'columbina.studio.edit.transform-controls';

function kept(): boolean {
  try { return localStorage.getItem(TRANSFORM_CONTROLS_KEY) === 'true'; } catch { return false; }
}

/**
 * Whether the on-screen Transform controls of the Edit page's timeline viewer are on (DaVinci Resolve
 * Reference Manual 20, p. 1106: the pop-up at the lower left of the viewer, set to Transform), and
 * what turns them. Off at first. The page remembers it, in this window's storage, not the project:
 * it is how one looks at the picture, and nothing of the film.
 */
export function useTransformControls(): { on: boolean; toggle: () => void } {
  const [on, setOn] = useState(kept);
  const toggle = useCallback(() => setOn(before => !before), []);
  useEffect(() => {
    try { localStorage.setItem(TRANSFORM_CONTROLS_KEY, String(on)); } catch { /* Not kept where storage is unavailable; it still applies now. */ }
  }, [on]);
  return { on, toggle };
}

/** The button of the on-screen Transform controls in the title bar of the timeline viewer. */
export function TransformControlsButton({ on, disabled, label, className, onToggle }: {
  on: boolean; disabled: boolean; label: string; className: string; onToggle: () => void;
}) {
  return <button type="button" className={className} aria-pressed={on} disabled={disabled} title={label}
    onClick={onToggle} data-test="transform-controls">{label}</button>;
}
