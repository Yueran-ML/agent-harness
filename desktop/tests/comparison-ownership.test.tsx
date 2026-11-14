// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useComparison } from '../src/studio/color/useComparison';
import { timeline, clip } from './timeline-fixture';
import { settle } from './colorEngine';

afterEach(cleanup);
describe('comparison request ownership', () => {
  it.each(['timeline', 'session', 'unload'] as const)('ignores old wipe and split replies after changing %s', async change => {
    const answers: ((answer: unknown) => void)[] = [];
    const call = vi.fn(() => new Promise<any>(resolve => answers.push(resolve)));
    const onEvent = vi.fn(() => () => {}), onProblem = vi.fn();
    let props: Parameters<typeof useComparison>[0] = { bridge: { call, onEvent }, session: 1, native: true, loaded: true,
      timeline: timeline([clip('c1', 0)]), clips: [clip('c1', 0)], current: clip('c1', 0), frame: 0, playing: false,
      chosen: new Set(['c1']), stills: ['s1'], currentStill: 's1', onGo: vi.fn(), onProblem };
    const h = renderHook(at => useComparison(at), { initialProps: props });
    act(() => { h.result.current.toggleWipe(); h.result.current.toggleSplit(); });
    props = { ...props, ...(change === 'timeline' ? { timeline: { ...props.timeline!, id: 't2' } }
      : change === 'session' ? { session: 2 } : { loaded: false }) };
    h.rerender(props);
    await act(async () => answers.shift()!({ ok: true, result: { mode: 'wipe', wipe: { reference: { kind: 'still', still: 's1' } } } }));
    await settle();
    expect(h.result.current.compared).toEqual({ mode: 'off' });
    expect(call).toHaveBeenCalledTimes(1); // The queued old split was discarded as well.
    expect(call.mock.calls[0]).toMatchObject(['viewer.wipe', expect.anything(), { expectedSession: 1 }]);
    expect(onProblem).not.toHaveBeenCalled();
    expect(onEvent).not.toHaveBeenCalled(); // Input is owned solely by useViewerInput.
  });
});
