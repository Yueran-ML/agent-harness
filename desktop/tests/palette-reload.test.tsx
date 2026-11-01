// @vitest-environment jsdom
import * as React from 'react';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { EditedNode } from '../src/studio/color/useNodeEdit';

afterEach(() => { cleanup(); vi.doUnmock('react'); vi.resetModules(); });

it('does not reuse an open gesture when the renderer reloads against the same engine connection', async () => {
  // main keeps this connection and session when its renderer reopens the same live project.
  const ids: string[] = [];
  const call = vi.fn(async (_method: string, params?: Record<string, unknown>) => {
    ids.push((params!.gesture as { id: string }).id);
    return { ok: true as const, result: { revision: ids.length + 10 } };
  });
  const context: EditedNode = { bridge: { call }, revision: 10, target: { level: 'clip', timeline: 't1', clip: 'c1' },
    node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: {} as never }, frame: 0, onFailure: vi.fn() };
  for (let reload = 0; reload < 2; reload += 1) {
    vi.resetModules();
    vi.doMock('react', () => React);
    const { useNodeEdit } = await import('../src/studio/color/useNodeEdit');
    const hook = renderHook(() => useNodeEdit(context));
    await act(async () => { hook.result.current.preview([['primaries.saturation', 60]]); });
    hook.unmount();
  }
  expect(ids).toHaveLength(2);
  expect(ids[1]).not.toBe(ids[0]);
});
