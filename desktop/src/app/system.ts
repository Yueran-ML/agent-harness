/**
 * What the page has to know of the system that it is shown on.
 *
 * On macOS the buttons of the window stand at the left end of the bar, and a shortcut is written
 * with ⌘, the key that it is pressed with there. The keys themselves are the same everywhere:
 * Ctrl and ⌘ are both taken on every system.
 */

/** Whether the application can start with the system, in the tray, and whether it does. */
export interface LoginItem {
  offered: boolean;
  on: boolean;
}

/** What the shell says of the start with the system. Anything else offers none. */
export function loginItemOf(answer: unknown): LoginItem {
  const given = answer !== null && typeof answer === 'object' ? answer as Record<string, unknown> : {};
  return { offered: given.offered === true, on: given.on === true };
}

/** Whether the page is shown in the application on macOS. */
export function onMac(): boolean {
  return window.columbina?.platform === 'darwin';
}

/** The key that is held for a shortcut, as the system names it. */
export function modifier(): string {
  return onMac() ? '⌘' : 'Ctrl';
}

/** A shortcut of a key with Ctrl, or with ⌘ on macOS, as the system writes it: Ctrl+B there, ⌘B here. */
export function shortcut(key: string): string {
  return onMac() ? `⌘${key}` : `Ctrl+${key}`;
}
