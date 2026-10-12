import { describe, expect, it } from 'vitest';
import type { SessionStatus } from '../src/protocol';
import { canSubmitInput, isCommand, permissionNeedsConfirmation, permissionPresetFromCommand } from '../src/uiActions';

const status = (mode: string, approval = 'ask'): SessionStatus => ({
  session_id: 'test', status: 'idle', detail: null,
  model: { provider: 'test', model: 'test', context_window: null },
  sandbox: { mode, strength: 'test' }, approval,
});

describe('permission changes', () => {
  it('recognizes slash commands using the service parser rules', () => {
    expect(permissionPresetFromCommand('  /PERMISSIONS   full-access  ')).toBe('full-access');
    expect(permissionPresetFromCommand('/permissions \tworkspace-write\n')).toBe('workspace-write');
    expect(permissionPresetFromCommand('/permissions \u0085full-access\u001f')).toBe('full-access');
    expect(permissionPresetFromCommand('/permissions\tfull-access')).toBeNull();
    expect(permissionPresetFromCommand('/permissions FULL-ACCESS')).toBeNull();
    expect(permissionPresetFromCommand('/permissions full-access extra')).toBeNull();
    expect(permissionPresetFromCommand('/permissions')).toBeNull();
  });

  it('requires confirmation for expanded sandbox access from either UI entry', () => {
    expect(permissionNeedsConfirmation('workspace-write', status('read-only'))).toBe(true);
    expect(permissionNeedsConfirmation('full-access', status('workspace-write'))).toBe(true);
    expect(permissionNeedsConfirmation(permissionPresetFromCommand('/PERMISSIONS full-access')!, status('workspace-write'))).toBe(true);
  });

  it('also confirms removing approval while sandbox access is already unrestricted', () => {
    expect(permissionNeedsConfirmation('full-access', status('danger-full-access', 'ask'))).toBe(true);
    expect(permissionNeedsConfirmation('full-access', status('danger-full-access', 'never'))).toBe(false);
  });

  it('does not prompt for a downgrade or an unchanged preset', () => {
    expect(permissionNeedsConfirmation('workspace-write', status('danger-full-access', 'never'))).toBe(false);
    expect(permissionNeedsConfirmation('read-only', status('workspace-write'))).toBe(false);
    expect(permissionNeedsConfirmation('workspace-write', status('workspace-write'))).toBe(false);
  });

  it('does not assume missing current permissions are safe to expand', () => {
    expect(permissionNeedsConfirmation('full-access', null)).toBe(true);
  });
});

describe('composer submission', () => {
  it('blocks running slash commands for Enter and submit while keeping follow-ups available', () => {
    const running = { unavailable: false, sending: false, running: true };
    expect(canSubmitInput('  /permissions full-access ', running)).toBe(false);
    expect(canSubmitInput('/help', running)).toBe(false);
    expect(canSubmitInput('Also update the docs', running)).toBe(true);
    expect(canSubmitInput('/help', { ...running, running: false })).toBe(true);
  });

  it('does not submit blank input or duplicate/unavailable requests', () => {
    expect(canSubmitInput(' ', { unavailable: false, sending: false, running: false })).toBe(false);
    expect(canSubmitInput('hello', { unavailable: true, sending: false, running: false })).toBe(false);
    expect(canSubmitInput('hello', { unavailable: false, sending: true, running: false })).toBe(false);
  });
});

describe('a line that is a command', () => {
  it('begins with a slash, whatever stands before it that is blank', () => {
    expect(isCommand('/compact')).toBe(true);
    expect(isCommand('  /model deepseek-flash')).toBe(true);
    expect(isCommand('\n/help')).toBe(true);
    expect(isCommand('/')).toBe(true);
  });

  it('is no line that has its slash further in, and no empty one', () => {
    expect(isCommand('Fix src/app.py')).toBe(false);
    expect(isCommand('a /compact')).toBe(false);
    expect(isCommand('')).toBe(false);
    expect(isCommand('   ')).toBe(false);
  });
});
