// @vitest-environment jsdom
/**
 * Newer versions of the application, as the page asks the shell for them: when it asks, what it
 * holds meanwhile, and which answer counts.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { UPDATE_EVERY_MS, UPDATE_FIRST_MS } from '../src/app/updates';
import { useUpdates } from '../src/useUpdates';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  let refuse!: (problem: Error) => void;
  const promise = new Promise<T>((resolve, reject) => { release = resolve; refuse = reject; });
  return { promise, release, refuse };
}

const page = (version: string) => `https://github.com/Yueran-ML/agent-harness/releases/tag/desktop-v${version}`;
const newer = (version: string) => ({ newer: version, page: page(version), failure: null });

/** The shell of a package, which looks for newer versions and finds 0.1.3, unless the test says otherwise. */
function shellOfUpdates(about: unknown = { version: '0.1.2', checks: true }) {
  const shell = {
    about: vi.fn(async (): Promise<unknown> => about),
    updates: vi.fn(async (_asked: { now: boolean }): Promise<unknown> => newer('0.1.3')),
  };
  vi.stubGlobal('columbina', shell);
  return shell;
}
/** Let the time of the test go on, and what is due with it. */
const wait = (ms: number) => act(async () => { vi.advanceTimersByTime(ms); });

describe('newer versions of the application', () => {
  it('are asked for nowhere where there is no shell', async () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useUpdates(true));
    await wait(UPDATE_EVERY_MS);
    act(() => result.current.checkNow());
    expect(result.current).toMatchObject({ about: null, found: null, asking: false });
  });

  it('are looked for a few seconds after the window opened, and once a day after that', async () => {
    vi.useFakeTimers();
    const shell = shellOfUpdates();
    const { result } = renderHook(() => useUpdates(true));
    await wait(0);
    expect(result.current.about).toEqual({ version: '0.1.2', checks: true });
    await wait(UPDATE_FIRST_MS - 1);
    expect(shell.updates).not.toHaveBeenCalled();
    await wait(1);
    expect(shell.updates.mock.calls).toEqual([[{ now: false }]]);
    expect(result.current).toMatchObject({ found: { newer: { version: '0.1.3', page: page('0.1.3') }, failure: null }, asking: false });
    shell.updates.mockResolvedValue(newer('0.1.4'));
    await wait(UPDATE_EVERY_MS - UPDATE_FIRST_MS - 1);
    expect(shell.updates).toHaveBeenCalledTimes(1);
    await wait(1);
    expect(shell.updates.mock.calls).toEqual([[{ now: false }], [{ now: false }]]);
    expect(result.current.found?.newer?.version).toBe('0.1.4');
    await wait(UPDATE_EVERY_MS);
    expect(shell.updates).toHaveBeenCalledTimes(3);
  });

  it('are looked for by themselves only while the user lets them, and asked for now all the same', async () => {
    vi.useFakeTimers();
    const shell = shellOfUpdates();
    const { result, rerender } = renderHook(({ checking }) => useUpdates(checking), { initialProps: { checking: false } });
    await wait(UPDATE_EVERY_MS);
    expect(shell.updates).not.toHaveBeenCalled();
    await act(async () => { result.current.checkNow(); });
    expect(shell.updates.mock.calls).toEqual([[{ now: true }]]);
    expect(result.current.found?.newer?.version).toBe('0.1.3');
    // Let again, it looks a few seconds later, and once a day after that, until it is told not to.
    rerender({ checking: true });
    await wait(UPDATE_FIRST_MS);
    expect(shell.updates.mock.calls.slice(1)).toEqual([[{ now: false }]]);
    rerender({ checking: false });
    await wait(2 * UPDATE_EVERY_MS);
    expect(shell.updates).toHaveBeenCalledTimes(2);
  });

  it('are never looked for by an application that runs from source', async () => {
    vi.useFakeTimers();
    const shell = shellOfUpdates({ version: '0.1.2', checks: false });
    const { result } = renderHook(() => useUpdates(true));
    await wait(UPDATE_EVERY_MS);
    act(() => result.current.checkNow());
    expect(shell.updates).not.toHaveBeenCalled();
    expect(result.current).toMatchObject({ about: { version: '0.1.2', checks: false }, found: null, asking: false });
  });

  it('are looked for nowhere where the shell says nothing of the application that can be read', async () => {
    vi.useFakeTimers();
    for (const about of [{ version: '0.1.2' }, null]) {
      const shell = shellOfUpdates(about);
      const { result, unmount } = renderHook(() => useUpdates(true));
      await wait(UPDATE_EVERY_MS);
      expect([result.current.about, shell.updates.mock.calls.length]).toEqual([null, 0]);
      unmount();
    }
    const shell = shellOfUpdates();
    shell.about.mockRejectedValueOnce(new Error('IPC sender is not the application window.'));
    const { result } = renderHook(() => useUpdates(true));
    await wait(UPDATE_EVERY_MS);
    expect([result.current.about, shell.updates.mock.calls.length]).toEqual([null, 0]);
  });

  it('say that a question is under way, and count only the answer to the last one', async () => {
    const shell = shellOfUpdates();
    const { result } = renderHook(() => useUpdates(false));
    await act(async () => {});
    const first = held<unknown>();
    const second = held<unknown>();
    shell.updates.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    act(() => result.current.checkNow());
    expect(result.current.asking).toBe(true);
    act(() => result.current.checkNow());
    // The first answer comes after the second was asked for, and counts for nothing.
    await act(async () => { first.release(newer('0.1.3')); });
    expect(result.current).toMatchObject({ found: null, asking: true });
    await act(async () => { second.release({ newer: null, page: null, failure: null }); });
    expect(result.current).toMatchObject({ found: { newer: null, failure: null }, asking: false });
    // Answered the other way round, the same holds.
    const third = held<unknown>();
    const fourth = held<unknown>();
    shell.updates.mockReturnValueOnce(third.promise).mockReturnValueOnce(fourth.promise);
    act(() => result.current.checkNow());
    act(() => result.current.checkNow());
    await act(async () => { fourth.release({ newer: null, page: null, failure: 'busy' }); });
    expect(result.current).toMatchObject({ found: { newer: null, failure: 'busy' }, asking: false });
    await act(async () => { third.release(newer('0.1.3')); });
    expect(result.current).toMatchObject({ found: { newer: null, failure: 'busy' }, asking: false });
  });

  it('keep a version that is out when a later question fails, and say why it failed', async () => {
    const shell = shellOfUpdates();
    const { result } = renderHook(() => useUpdates(false));
    await act(async () => {});
    await act(async () => { result.current.checkNow(); });
    shell.updates.mockResolvedValueOnce({ newer: null, page: null, failure: 'network' });
    await act(async () => { result.current.checkNow(); });
    expect(result.current.found).toEqual({ newer: { version: '0.1.3', page: page('0.1.3') }, failure: 'network' });
    shell.updates.mockRejectedValueOnce(new Error('The window is gone.'));
    await act(async () => { result.current.checkNow(); });
    expect(result.current.found).toEqual({ newer: { version: '0.1.3', page: page('0.1.3') }, failure: 'unreadable' });
    // An answer that came says what is out, and nothing more.
    shell.updates.mockResolvedValueOnce({ newer: null, page: null, failure: null });
    await act(async () => { result.current.checkNow(); });
    expect(result.current.found).toEqual({ newer: null, failure: null });
    shell.updates.mockResolvedValueOnce({ newer: null, page: null, failure: 'busy' });
    await act(async () => { result.current.checkNow(); });
    expect(result.current.found).toEqual({ newer: null, failure: 'busy' });
  });

  it('come as no list where the shell asked nothing', async () => {
    const shell = shellOfUpdates();
    shell.updates.mockRejectedValueOnce(new Error('This application does not look for newer versions.'));
    const { result } = renderHook(() => useUpdates(false));
    await act(async () => {});
    await act(async () => { result.current.checkNow(); });
    expect(result.current).toMatchObject({ found: { newer: null, failure: 'unreadable' }, asking: false });
  });

  it('are looked for no more once the page is gone', async () => {
    vi.useFakeTimers();
    const shell = shellOfUpdates();
    const { unmount } = renderHook(() => useUpdates(true));
    await wait(0);
    unmount();
    await wait(2 * UPDATE_EVERY_MS);
    expect(shell.updates).not.toHaveBeenCalled();
  });
});
