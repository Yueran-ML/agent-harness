// @vitest-environment jsdom
/**
 * Timeline › Audio Scrubbing (Reference Manual 20, p. 731; phase 3 T18): Shift+S turns the sound
 * of a dragged playhead off and on again through `transport.scrub_audio` of the timeline monitor;
 * the switch shows what the state of the transport tells; a key typed into a field, S without
 * Shift, or Shift+S with another modifier asks nothing; nothing is asked while no timeline is
 * loaded; and a refusal turns the switch back.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useAudioScrubbing } from '../src/studio/timeline/scrub';
import type { TimelineTransportState } from '../src/studio/engine';

afterEach(() => { cleanup(); vi.restoreAllMocks(); document.body.innerHTML = ''; });

const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });

function state(scrub_audio?: boolean): TimelineTransportState {
  return {
    monitor: 'timeline', timeline: 't1', frame: 0, playing: false, speed: 1, exact: true, shown: 0, last: 99,
    sound: 'off', proxies: true, source: 'original', late: 0, ...(scrub_audio === undefined ? {} : { scrub_audio }),
  };
}

function mount(told: TimelineTransportState | null, refuse = false, loaded = true) {
  const failures: unknown[] = [];
  const bridge = {
    call: vi.fn(async () => (refuse
      ? { ok: false as const, error: { code: -32602, message: 'refused', data: { reason: 'monitor' } } }
      : { ok: true as const, result: {} })),
  };
  const hook = renderHook(({ shown }: { shown: TimelineTransportState | null }) => useAudioScrubbing({
    bridge, session: 1, loaded, state: shown, onFailure: failure => failures.push(failure),
  }), { initialProps: { shown: told } });
  return { bridge, hook, failures };
}

function press(init: KeyboardEventInit, target: EventTarget = document.body) {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  act(() => { target.dispatchEvent(event); });
  return event;
}

describe('Audio Scrubbing', () => {
  it('is on at first, and Shift+S turns it off and on again on the timeline monitor', async () => {
    const { bridge, hook } = mount(state());
    expect(hook.result.current.scrubAudio).toBe(true);
    const pressed = press({ key: 'S', code: 'KeyS', shiftKey: true });
    expect(pressed.defaultPrevented).toBe(true);
    await settle();
    expect(hook.result.current.scrubAudio).toBe(false);
    expect(bridge.call).toHaveBeenLastCalledWith('transport.scrub_audio', { monitor: 'timeline', on: false }, { expectedSession: 1 });
    press({ key: 'S', code: 'KeyS', shiftKey: true });
    await settle();
    expect(hook.result.current.scrubAudio).toBe(true);
    expect(bridge.call).toHaveBeenLastCalledWith('transport.scrub_audio', { monitor: 'timeline', on: true }, { expectedSession: 1 });
  });

  it('shows what the transport tells', async () => {
    const { hook } = mount(state(false));
    expect(hook.result.current.scrubAudio).toBe(false);
    hook.rerender({ shown: state(true) });
    await settle();
    expect(hook.result.current.scrubAudio).toBe(true);
  });

  it('asks nothing for S alone, another modifier, a key typed into a field, or with no timeline loaded', async () => {
    const { bridge } = mount(state());
    press({ key: 's', code: 'KeyS' });
    press({ key: 'S', code: 'KeyS', shiftKey: true, ctrlKey: true });
    press({ key: 'S', code: 'KeyS', shiftKey: true, altKey: true });
    const field = document.createElement('input');
    document.body.append(field);
    press({ key: 'S', code: 'KeyS', shiftKey: true }, field);
    await settle();
    expect(bridge.call).not.toHaveBeenCalled();
    const unloaded = mount(state(), false, false);
    press({ key: 'S', code: 'KeyS', shiftKey: true });
    await settle();
    expect(unloaded.bridge.call).not.toHaveBeenCalled();
  });

  it('turns back where the engine refuses', async () => {
    const { hook, failures } = mount(state(true), true);
    press({ key: 'S', code: 'KeyS', shiftKey: true });
    await settle();
    expect(hook.result.current.scrubAudio).toBe(true);
    expect(failures).toHaveLength(1);
  });
});
