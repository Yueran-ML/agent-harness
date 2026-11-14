// @vitest-environment jsdom
/**
 * What the Fairlight page says of a failure is a sentence of the reader's language, not a bare
 * `String(error)`; and Punch In has its Chinese name (穿插写入: punched in, written).
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import type { Timeline } from '../src/studio/engine';
import { VcaAssignment } from '../src/studio/fairlight/VcaAssignment';
import { zhCN } from '../src/messages/zh-CN';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const song = { ...timeline([]), tracks: [{ id: 'a1', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [],
  transitions: [], strip: { fader_db: 0 } }] } as unknown as Timeline;
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.spyOn(console, 'warn').mockImplementation(() => {}); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it('names Punch In as the Chinese DaVinci page does', () => {
  expect(zhCN.label.studio.fairlightPage['Punch In']).toBe('穿插写入');
});

it.each([
  ['a refusal, by its reason', { ok: false, error: { code: -32001, message: 'refused: locked', data: { reason: 'locked' } } },
    zhCN.hint.studio.edit.locked],
  ['anything else, by the general sentence', 'socket closed', zhCN.hint.studio.edit.failed],
])('says %s, in the language of the page, and logs what failed', async (_, answer, said) => {
  const call = vi.fn(async () => { if (typeof answer === 'string') throw answer; return answer; });
  render(<Words.Provider value={zhCN}>
    <VcaAssignment context={{ bridge: { call } as never, session: 1, revision: 7, timeline: song }} of={{ track: 'a1' }} />
  </Words.Provider>);
  fireEvent.click(screen.getByRole('button', { name: 'VCA Voice' }));
  fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'VCA 1' })); await settle();
  expect(screen.getByRole('alert').textContent).toBe(said);
  expect(console.warn).toHaveBeenCalled();
});
