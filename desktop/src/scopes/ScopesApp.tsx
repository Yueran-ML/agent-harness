/**
 * The floating window of the video scopes of the Color page, as a window of its own (Reference
 * Manual 20, pp. 3007–3008, 3018–3019; T19): 1, 2, 4 or 9 panes of the timeline viewer's scopes,
 * each with its own settings, kept on this computer with the docked one's (`columbina-scopes`).
 * The shell opens it over the main window, so the engine's view of the viewer never covers it,
 * and it may go to another display. It reaches the engine for its own scopes alone, and closes
 * with its button, Ctrl+Shift+W or the main window.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { keyOf } from '../app/keys';
import { Words } from '../app/locale';
import { messagesFor } from '../i18n';
import { themes } from '../themes';
import type { PageMethod } from '../studio/engine';
import { keepWindowBounds, ScopesWindowBody, usePageShown, type ScopesContext } from '../studio/color/Scopes';
import { loadScopes, scopesStorageKey, storeScopes, type ScopesSettings } from '../studio/color/scopeSettings';
import type { ScopesWindowContext } from './bridge';

/** How long the window rests at a place and size before they are kept. */
const KEEP_MS = 300;

export function ScopesApp() {
  const shell = window.columbinaScopes;
  const [context, setContext] = useState<ScopesWindowContext | null>(null);
  const [settings, setSettings] = useState<ScopesSettings>(loadScopes);
  const shown = usePageShown();
  const words = useMemo(() => messagesFor(context?.language ?? 'en'), [context?.language]);

  useEffect(() => {
    if (!shell) return undefined;
    void shell.context().then(setContext, () => {});
    return shell.onContext(setContext);
  }, [shell]);
  useEffect(() => {
    if (context === null) return;
    document.documentElement.lang = context.language;
    if (themes.some(theme => theme === context.theme)) document.documentElement.dataset.theme = context.theme;
  }, [context?.language, context?.theme]);
  // The main window keeps the docked scope in the same place: each reads again what the other changed.
  useEffect(() => {
    const changed = (event: StorageEvent) => { if (event.key === null || event.key === scopesStorageKey) setSettings(loadScopes()); };
    window.addEventListener('storage', changed);
    return () => window.removeEventListener('storage', changed);
  }, []);
  const change = useCallback((how: (old: ScopesSettings) => ScopesSettings) => setSettings((old) => {
    const next = how(old);
    storeScopes(next);
    return next;
  }), []);
  const close = useCallback(() => { void shell?.close(); }, [shell]);
  // Ctrl+Shift+W closes it, as it opened it (Workspace › Video Scopes › On/Off).
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      const key = keyOf(event);
      if (key.repeat || !key.ctrl || !key.shift || key.alt || key.meta) return;
      if (key.code !== 'KeyW' && key.key.toLowerCase() !== 'w') return;
      event.preventDefault();
      close();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [close]);
  // Where it is and how large, kept for the next time it is opened.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const keep = () => keepWindowBounds({ x: window.screenX, y: window.screenY, width: window.outerWidth, height: window.outerHeight });
    const later = () => { if (timer !== null) clearTimeout(timer); timer = setTimeout(keep, KEEP_MS); };
    window.addEventListener('resize', later);
    window.addEventListener('beforeunload', keep);
    return () => { if (timer !== null) clearTimeout(timer); window.removeEventListener('resize', later); window.removeEventListener('beforeunload', keep); };
  }, []);

  const bridge = useMemo<ScopesContext['bridge']>(() => ({
    call: (method: PageMethod, params?: Record<string, unknown>) => (shell ? shell.call(method, params)
      : Promise.resolve({ ok: false as const, error: { code: null, message: 'There is no shell.', data: null } })),
    onEvent: listener => (shell ? shell.onEvent(listener) : () => {}),
  }), [shell]);
  const scopes: ScopesContext = { bridge, session: context?.session ?? null, view: 'timeline', loaded: context?.loaded ?? false, native: shell !== undefined };
  return <Words.Provider value={words}>
    <main className="scopes-window-page">
      <ScopesWindowBody context={scopes} active={shown && context !== null} settings={settings} onSettings={change} onClose={close} />
    </main>
  </Words.Provider>;
}
