/** The themes of the application. Their colours are in `ui/tokens.css`. */
export const themes = ['moonveil', 'moonwhite'] as const;
export type ThemeId = typeof themes[number];

// A new key for every set of themes, so that a name that is gone is not looked up.
export const themeStorageKey = 'columbina-theme-v3';

/** The theme that was chosen last, or the dark one. */
export function initialTheme(): ThemeId {
  try {
    const saved = localStorage.getItem(themeStorageKey);
    if (themes.some(theme => theme === saved)) return saved as ThemeId;
  } catch {
    // Without storage the choice holds for this window.
  }
  return 'moonveil';
}

export function storeTheme(theme: ThemeId): void {
  try {
    localStorage.setItem(themeStorageKey, theme);
  } catch {
    // See above.
  }
}
