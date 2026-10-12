export const themes = [
  { id: 'frostlight', name: 'Nod-Krai', subtitle: 'Moonwhite · clear blue', swatches: ['#f4f9ff', '#e9f3fc', '#2f68a5'] },
  { id: 'moonveil', name: 'Moonveil', subtitle: 'Moonlit violet', swatches: ['#12131d', '#252438', '#c6b7ef'] },
  { id: 'silvermoon', name: 'Silvermoon', subtitle: 'Nod-Krai · night', swatches: ['#101522', '#1a2234', '#a7bcf5'] },
] as const;
export type ThemeId = typeof themes[number]['id'];
// Start this palette revision in blue-white; subsequent theme choices stay saved.
export const themeStorageKey = 'columbina-theme-v2';
export function initialTheme(): ThemeId {
  try { const saved = localStorage.getItem(themeStorageKey); if (themes.some(theme => theme.id === saved)) return saved as ThemeId; } catch { /* Preferences remain available for this window. */ }
  return 'frostlight';
}
