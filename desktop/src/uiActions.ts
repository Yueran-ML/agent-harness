import type { SessionStatus } from './protocol';

export type PermissionPreset = 'read-only' | 'workspace-write' | 'full-access';

const sandboxLevels: Record<string, number> = {
  'read-only': 0,
  'workspace-write': 1,
  'danger-full-access': 2,
};

/** Match the service's lower-case command name and case-sensitive preset. */
export function permissionPresetFromCommand(input: string): PermissionPreset | null {
  const line = input.trim();
  const separator = line.indexOf(' ');
  if (separator < 0 || line.slice(0, separator).toLowerCase() !== '/permissions') return null;
  // Python str.strip also treats NEL and the ASCII separators as whitespace.
  const preset = line.slice(separator + 1).replace(/^[\s\u0085\u001c-\u001f]+|[\s\u0085\u001c-\u001f]+$/g, '');
  return preset === 'read-only' || preset === 'workspace-write' || preset === 'full-access'
    ? preset : null;
}

export function permissionNeedsConfirmation(preset: PermissionPreset, status: SessionStatus | null): boolean {
  const currentLevel = sandboxLevels[status?.sandbox?.mode ?? ''] ?? 0;
  const targetLevel = preset === 'full-access' ? 2 : sandboxLevels[preset];
  return targetLevel > currentLevel || (preset === 'full-access' && status?.approval !== 'never');
}

export function canSubmitInput(text: string, state: { unavailable: boolean; sending: boolean; running: boolean }): boolean {
  return !!text.trim() && !state.unavailable && !state.sending
    && !(state.running && text.trim().startsWith('/'));
}
