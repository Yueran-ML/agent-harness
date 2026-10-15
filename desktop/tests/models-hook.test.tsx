// @vitest-environment jsdom
/**
 * The models of the services, as the page asks the shell for them: what it holds of each
 * service, and how seldom it asks.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useModels } from '../src/useModels';

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

/** A shell that answers as the test says. */
function shellOfModels() {
  const shell = { listModels: vi.fn(async (_service: string): Promise<unknown> => ({ models: ['deepseek-flash', 'deepseek-pro'] })) };
  vi.stubGlobal('columbina', shell);
  return shell;
}

describe('the models of a service', () => {
  it('are asked for nowhere where there is no shell, as in a replay', async () => {
    const { result } = renderHook(() => useModels());
    await act(async () => { await result.current.ask('deepseek'); result.current.freshen('deepseek'); });
    expect(result.current.of('deepseek')).toBeNull();
  });

  it('are what the service lists, once it has answered', async () => {
    vi.useFakeTimers({ now: 1_000_000, toFake: ['Date'] });
    const shell = shellOfModels();
    const answer = held<unknown>();
    shell.listModels.mockReturnValueOnce(answer.promise);
    const { result } = renderHook(() => useModels());
    act(() => { void result.current.ask('deepseek'); });
    expect(shell.listModels.mock.calls).toEqual([['deepseek']]);
    expect(result.current.of('deepseek')).toEqual({ kind: 'asking', models: [] });
    await act(async () => { answer.release({ models: ['deepseek-flash', 'deepseek-pro', 'deepseek-flash', 7] }); });
    // Each once, and nothing that is no name.
    expect(result.current.of('deepseek')).toEqual({ kind: 'listed', models: ['deepseek-flash', 'deepseek-pro'], at: 1_000_000 });
  });

  it('say why the service listed none', async () => {
    const shell = shellOfModels();
    shell.listModels.mockResolvedValueOnce({ error: 'The key was refused.', kind: 'key' });
    const { result } = renderHook(() => useModels());
    await act(() => result.current.ask('deepseek'));
    expect(result.current.of('deepseek')).toMatchObject({ kind: 'failed', failure: 'key', said: 'The key was refused.' });
    shell.listModels.mockRejectedValueOnce(new Error('That service is not among those that are set up.'));
    await act(() => result.current.ask('deepseek'));
    expect(result.current.of('deepseek'))
      .toMatchObject({ kind: 'failed', failure: 'unknown', said: 'That service is not among those that are set up.' });
    shell.listModels.mockRejectedValueOnce('gone');
    await act(() => result.current.ask('deepseek'));
    expect(result.current.of('deepseek')).toMatchObject({ kind: 'failed', said: 'gone' });
  });

  it('are shown as they were while the service is asked again, and none after a failure', async () => {
    const shell = shellOfModels();
    const { result } = renderHook(() => useModels());
    await act(() => result.current.ask('deepseek'));
    const again = held<unknown>();
    shell.listModels.mockReturnValueOnce(again.promise);
    act(() => { void result.current.ask('deepseek'); });
    expect(result.current.of('deepseek')).toEqual({ kind: 'asking', models: ['deepseek-flash', 'deepseek-pro'] });
    await act(async () => { again.release({ error: 'down', kind: 'network' }); });
    shell.listModels.mockReturnValueOnce(new Promise(() => {}));
    act(() => { void result.current.ask('deepseek'); });
    expect(result.current.of('deepseek')).toEqual({ kind: 'asking', models: [] });
  });

  it('are those of the last ask, whichever answer comes first', async () => {
    const shell = shellOfModels();
    const first = held<unknown>();
    const second = held<unknown>();
    shell.listModels.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => useModels());
    act(() => { void result.current.ask('deepseek'); void result.current.ask('deepseek'); });
    await act(async () => { second.release({ models: ['deepseek-pro'] }); });
    await act(async () => { first.release({ models: ['deepseek-old'] }); });
    expect(result.current.of('deepseek')).toMatchObject({ kind: 'listed', models: ['deepseek-pro'] });
    const third = held<unknown>();
    shell.listModels.mockReturnValueOnce(third.promise).mockResolvedValueOnce({ models: ['deepseek-flash'] });
    act(() => { void result.current.ask('deepseek'); });
    await act(() => result.current.ask('deepseek'));
    await act(async () => { third.refuse(new Error('too late')); });
    expect(result.current.of('deepseek')).toMatchObject({ kind: 'listed', models: ['deepseek-flash'] });
  });

  it('of one service are kept apart from those of another', async () => {
    const shell = shellOfModels();
    shell.listModels.mockResolvedValueOnce({ models: ['qwen3:8b'] });
    const { result } = renderHook(() => useModels());
    await act(() => result.current.ask('ollama'));
    await act(() => result.current.ask('deepseek'));
    expect(result.current.of('ollama')).toMatchObject({ models: ['qwen3:8b'] });
    expect(result.current.of('deepseek')).toMatchObject({ models: ['deepseek-flash', 'deepseek-pro'] });
    expect(result.current.of('openai')).toBeNull();
  });
});

describe('asking a service for its models no more often than it must', () => {
  it('asks where nothing is known, and not again while it asks', async () => {
    const shell = shellOfModels();
    const answer = held<unknown>();
    shell.listModels.mockReturnValueOnce(answer.promise);
    const { result } = renderHook(() => useModels());
    act(() => result.current.freshen('deepseek'));
    act(() => result.current.freshen('deepseek'));
    expect(shell.listModels).toHaveBeenCalledTimes(1);
    await act(async () => { answer.release({ models: ['deepseek-flash'] }); });
  });

  it('asks again only once what was listed is older than five minutes, and at once after a failure', async () => {
    vi.useFakeTimers({ now: 1_000_000, toFake: ['Date'] });
    const shell = shellOfModels();
    const { result } = renderHook(() => useModels());
    await act(async () => { result.current.freshen('deepseek'); });
    expect(shell.listModels).toHaveBeenCalledTimes(1);
    vi.setSystemTime(1_000_000 + 299_999);
    await act(async () => { result.current.freshen('deepseek'); });
    expect(shell.listModels).toHaveBeenCalledTimes(1);
    vi.setSystemTime(1_000_000 + 300_000);
    shell.listModels.mockResolvedValueOnce({ error: 'down', kind: 'network' });
    await act(async () => { result.current.freshen('deepseek'); });
    expect(shell.listModels).toHaveBeenCalledTimes(2);
    await act(async () => { result.current.freshen('deepseek'); });
    expect(shell.listModels).toHaveBeenCalledTimes(3);
  });
});
