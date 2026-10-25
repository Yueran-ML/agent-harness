// @vitest-environment jsdom
/** The pet on the page: what the main window tells it of the task, and its part of the settings. */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PetChoices } from '../src/bridge';
import { en } from '../src/messages/en';
import { PetSettings } from '../src/app/PetSettings';
import { usePet } from '../src/app/usePet';
import { AFTER_MS } from '../src/pet/state';
import type { PendingApproval, Workspace } from '../src/useWorkspace';
import { begin, end, likeABrowser, log, reply, statusOf, user, workspaceWith } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => marked(name)[0];

function Telling({ workspace }: { workspace: Workspace }) {
  usePet(workspace, en);
  return null;
}
const asked: PendingApproval = {
  requestId: 7, session_id: 's-1', call_id: 'c-1', tool: 'shell', summary: 'Run a command.',
  reason: 'The command is not among those that run without a question.', target: 'npm test', standing: null,
};

describe('what the main window tells the pet', () => {
  it('is what the task does whenever that changes, and nothing more often', () => {
    const petState = vi.fn();
    vi.stubGlobal('columbina', { petState });
    const { rerender } = render(<Telling workspace={workspaceWith()} />);
    expect(petState.mock.calls).toEqual([[{ state: 'idle', label: '', bubble: '' }]]);
    rerender(<Telling workspace={workspaceWith()} />);
    expect(petState).toHaveBeenCalledTimes(1);
    rerender(<Telling workspace={workspaceWith({ status: statusOf({ status: 'running' }), approval: asked })} />);
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'waiting', label: 'shell', bubble: 'Waiting for your yes to shell.' }]);
    rerender(<Telling workspace={workspaceWith({ status: statusOf({ status: 'running' }) })} />);
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'thinking', label: '', bubble: '' }]);
  });

  it('says that a turn is done where the user is not looking, and rests again a while later', () => {
    vi.useFakeTimers({ now: 1300 });
    const petState = vi.fn();
    vi.stubGlobal('columbina', { petState });
    const events = log(begin(1), user('Hi.'), reply('Hello.'), end(1));
    const looking = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const { unmount } = render(<Telling workspace={workspaceWith({ events })} />);
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'done', label: '', bubble: '' }]);
    unmount();
    looking.mockReturnValue(false);
    render(<Telling workspace={workspaceWith({ events })} />);
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'done', label: '', bubble: 'Done.' }]);
    act(() => { vi.advanceTimersByTime(AFTER_MS); });
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'idle', label: '', bubble: '' }]);
  });

  it('wakes for nothing while the pet rests', () => {
    vi.useFakeTimers();
    vi.stubGlobal('columbina', { petState: vi.fn() });
    render(<Telling workspace={workspaceWith()} />);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('says that a turn failed, and tells a shell without a pet nothing', () => {
    vi.useFakeTimers({ now: 1200 });
    const petState = vi.fn();
    vi.stubGlobal('columbina', { petState });
    const failed = log(begin(1), user('Hi.'), end(1, 'error', 'no'));
    render(<Telling workspace={workspaceWith({ events: failed })} />);
    expect(petState.mock.calls.at(-1)).toEqual([{ state: 'failed', label: '', bubble: 'Something went wrong.' }]);
    cleanup();
    vi.stubGlobal('columbina', {});
    expect(() => render(<Telling workspace={workspaceWith()} />)).not.toThrow();
  });
});

describe('the part of the settings for the pet', () => {
  const choices = (given: Partial<PetChoices> = {}): PetChoices => ({
    on: false, share: false, chosen: 'builtin/moon',
    pets: [
      // The pets of the application are named in the words of the page, and not as their packages name them.
      { key: 'builtin/moon', name: 'moon package', description: 'A small moon.', origin: 'builtin' },
      { key: 'builtin/cat', name: 'cat package', description: 'A navy cat.', origin: 'builtin' },
      { key: 'builtin/owl', name: 'Owl', description: 'Not drawn here.', origin: 'builtin' },
      { key: 'user/mochi', name: 'Mochi', description: 'A round friend.', origin: 'user' },
    ],
    folder: 'C:\\Users\\me\\AppData\\Roaming\\Columbina\\pets',
    stateFile: 'C:\\Users\\me\\AppData\\Roaming\\Columbina\\pet-state.json',
    ...given,
  });
  function shell(first: PetChoices) {
    let now = first;
    let told: (said: { on: boolean }) => void = () => {};
    const bridge = {
      pets: vi.fn(async () => now),
      setPet: vi.fn(async (asked: { on?: boolean; chosen?: string; share?: boolean }) => {
        now = { ...now, ...(asked.on === undefined ? {} : { on: asked.on }), ...(asked.share === undefined ? {} : { share: asked.share }), ...(asked.chosen ? { chosen: asked.chosen } : {}) };
      }),
      openPetsFolder: vi.fn(async () => {}),
      onPet: vi.fn((listener: (said: { on: boolean }) => void) => { told = listener; return () => {}; }),
    };
    vi.stubGlobal('columbina', bridge);
    return { bridge, hide: () => { now = { ...now, on: false }; told({ on: false }); } };
  }
  const drawn = async () => {
    render(<PetSettings />);
    await act(async () => {});
  };

  it('is not there without a shell that has pets', async () => {
    await drawn();
    expect(marked('pet')).toEqual([]);
    vi.stubGlobal('columbina', { setTheme: vi.fn() });
    await drawn();
    expect(marked('pet')).toEqual([]);
  });

  it('shows the pet, chooses another one, and names the pets of the user as theirs', async () => {
    const { bridge } = shell(choices());
    await drawn();
    const names = [...one('pet-choices').querySelectorAll('label')].map(label => label.textContent);
    expect(names).toEqual(['Little Moon', 'Little Cat', 'Owl', 'Mochi · yours']);
    expect([...one('pet-choices').querySelectorAll('label')].map(label => label.title)).toEqual(['A small moon.', 'A navy cat.', 'Not drawn here.', 'A round friend.']);
    await act(async () => { fireEvent.click(one('pet-on')); });
    expect(bridge.setPet).toHaveBeenLastCalledWith({ on: true });
    expect((one('pet-on') as HTMLInputElement).checked).toBe(true);
    await act(async () => { fireEvent.click(one('pet-choices').querySelectorAll('input')[1]); });
    expect(bridge.setPet).toHaveBeenLastCalledWith({ chosen: 'builtin/cat' });
    expect((one('pet-choices').querySelectorAll('input')[1] as HTMLInputElement).checked).toBe(true);
    await act(async () => { fireEvent.click(one('pet-on')); });
    expect(bridge.setPet).toHaveBeenLastCalledWith({ on: false });
  });

  it('tells other programs what the task does where it is asked to, and says where', async () => {
    const { bridge } = shell(choices());
    await drawn();
    expect(one('pet-share-hint').textContent).toBe(en.hint.pet.share('C:\\Users\\me\\AppData\\Roaming\\Columbina\\pet-state.json'));
    await act(async () => { fireEvent.click(one('pet-share')); });
    expect(bridge.setPet).toHaveBeenLastCalledWith({ share: true });
    expect((one('pet-share') as HTMLInputElement).checked).toBe(true);
    await act(async () => { fireEvent.click(one('pet-share')); });
    expect(bridge.setPet).toHaveBeenLastCalledWith({ share: false });
    await act(async () => { fireEvent.click(one('pet-folder')); });
    expect(bridge.openPetsFolder).toHaveBeenCalledTimes(1);
  });

  it('hears that the pet was hidden from its own menu', async () => {
    const { hide } = shell(choices({ on: true }));
    await drawn();
    expect((one('pet-on') as HTMLInputElement).checked).toBe(true);
    await act(async () => { hide(); });
    expect((one('pet-on') as HTMLInputElement).checked).toBe(false);
  });

  it('says where nothing can be shown, and what went wrong', async () => {
    const { bridge } = shell(choices({ pets: [], chosen: null }));
    await drawn();
    expect(marked('pet-choices')).toEqual([]);
    expect(one('pet').textContent).toContain(en.hint.pet.none);
    expect((one('pet-on') as HTMLInputElement).disabled).toBe(true);
    bridge.setPet.mockRejectedValueOnce(new Error('The pet cannot be shown.'));
    await act(async () => { fireEvent.click(one('pet-share')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('The pet cannot be shown.');
    bridge.setPet.mockRejectedValueOnce('no');
    await act(async () => { fireEvent.click(one('pet-share')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('no');
    bridge.openPetsFolder.mockRejectedValueOnce(new Error('No folder.'));
    await act(async () => { fireEvent.click(one('pet-folder')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('No folder.');
    // What went wrong goes once something goes right.
    await act(async () => { fireEvent.click(one('pet-share')); });
    expect(one('pet').querySelector('[role="alert"]')).toBeNull();
  });
});
