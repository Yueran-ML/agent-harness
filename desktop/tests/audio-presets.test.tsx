// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { ABCompare } from '../src/studio/audio/ABCompare';
import { cleanAudioPreset, factoryAudioPresets } from '../src/studio/audio/audioPresets';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { defaultEffect } from '../src/studio/audio/effectModel';
import { EffectPresets } from '../src/studio/audio/EffectPresets';
import type { AudioPresetPayload } from '../src/studio/engine';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
beforeEach(likeABrowser); afterEach(cleanup);
it('uses the specified factory settings and NR band presets', () => {
  const eq = factoryAudioPresets({ kind: 'eq', settings: defaultEq() });
  expect(eq[1].payload.settings).toMatchObject({ on: true, bands: [expect.objectContaining({ kind: 'high_pass', on: true, freq: 80, slope: 12 }), expect.anything(), expect.objectContaining({ freq: 300, gain_db: -3 }), expect.anything(), expect.anything(), expect.anything()] });
  expect((eq[2].payload.settings as any).bands[4]).toMatchObject({ kind: 'high_shelf', freq: 8000, gain_db: 3 });
  const dy = factoryAudioPresets({ kind: 'dynamics', settings: defaultDynamics() });
  expect(dy[1].payload.settings).toMatchObject({ on: true, compressor: { on: true, threshold_db: -20, ratio: 3, attack_ms: 5, release_ms: 120 } });
  expect(dy[2].payload.settings).toMatchObject({ expander: { on: true, mode: 'gate', threshold_db: -45, range_db: 18 } });
  expect(dy[3].payload.settings).toMatchObject({ limiter: { on: true, threshold_db: -1 } });
  const nr = factoryAudioPresets({ kind: 'effect', settings: defaultEffect('noise_reduction') });
  expect(nr.map(p => (p.payload.settings as any).band)).toEqual(['full', 'hiss', 'rumble', 'rumble_and_hiss']);
});
it('strips project source references while retaining the actual processor parameters', () => {
  const settings = defaultDynamics(); settings.compressor.sidechain = { on: true, listen: true, source: { track: 'secret-project-id' } }; settings.compressor.ratio = 4;
  expect(cleanAudioPreset({ kind: 'dynamics', settings })).toMatchObject({ settings: { compressor: { ratio: 4, sidechain: { on: false, listen: false, source: null } } } });
  expect(settings.compressor.sidechain.source).toEqual({ track: 'secret-project-id' });
});
it('switches actual A/B settings, copies only the cache and isolates owner identities on reopen', () => {
  const bridge = {}, writes = vi.fn();
  function Host({ identity }: { identity: string }) { const [value, setValue] = useState<AudioPresetPayload>({ kind: 'effect', settings: { kind: 'gain', gain_db: 0 } }); return <><button onClick={() => setValue({ kind: 'effect', settings: { kind: 'gain', gain_db: 5 } })}>Edit</button><ABCompare bridge={bridge} identity={identity} value={value} disabled={false} apply={next => { writes(next); setValue(next); }} /></>; }
  const view = render(<Host identity="session/track:a/fx1" />); fireEvent.click(screen.getByText('Edit')); fireEvent.click(screen.getByRole('button', { name: 'B', exact: false } as any));
  expect(writes).toHaveBeenLastCalledWith({ kind: 'effect', settings: { kind: 'gain', gain_db: 0 } });
  fireEvent.click(screen.getByRole('button', { name: 'A', exact: false } as any)); expect(writes).toHaveBeenLastCalledWith({ kind: 'effect', settings: { kind: 'gain', gain_db: 5 } });
  const calls = writes.mock.calls.length; fireEvent.click(screen.getByRole('button', { name: 'Copy comparison' })); expect(writes).toHaveBeenCalledTimes(calls);
  view.unmount(); render(<Host identity="session/track:b/fx1" />); fireEvent.click(screen.getByRole('button', { name: 'B' })); expect(writes).toHaveBeenLastCalledWith({ kind: 'effect', settings: { kind: 'gain', gain_db: 0 } });
});
it('persists a named preset with an audio payload and recalls it in one apply', async () => {
  const apply = vi.fn(), presets: any[] = [], requests: any[] = [];
  const bridge: any = { call: async (method: string, params: any) => { requests.push({ method, params }); if (method === 'audio_presets.list') return { ok: true, result: { presets } }; const preset = { id: 'ap1', name: params.name, payload: params.payload }; presets.push(preset); return { ok: true, result: preset }; } };
  render(<EffectPresets context={{ bridge, session: 1, revision: 1, timeline: timeline([]), target: { track: 'a' } }} identity="insert/fx1/gain" value={{ kind: 'effect', settings: { kind: 'gain', gain_db: -3 } }} disabled={false} apply={apply} />);
  fireEvent.click(screen.getByRole('button', { name: 'Save preset' })); fireEvent.change(screen.getByRole('textbox', { name: 'Preset name' }), { target: { value: 'My gain' } }); fireEvent.submit(screen.getByRole('textbox', { name: 'Preset name' }).closest('form')!);
  await act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
  expect(requests[0]).toEqual({ method: 'audio_presets.save', params: { name: 'My gain', payload: { kind: 'effect', settings: { kind: 'gain', gain_db: -3 } } } });
  expect(apply).not.toHaveBeenCalled(); fireEvent.change(screen.getByRole('combobox', { name: 'Preset' }), { target: { value: 'ap1' } }); expect(apply).toHaveBeenCalledTimes(1);
});
