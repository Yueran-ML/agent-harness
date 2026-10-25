// @vitest-environment jsdom
/** The part of the settings for the pet. */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PetChoices } from '../src/bridge';
import { en } from '../src/messages/en';
import { PetSettings } from '../src/app/PetSettings';
import { likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => marked(name)[0];

describe('the part of the settings for the pet', () => {
  const stateFile = 'C:\\Users\\me\\AppData\\Roaming\\Columbina\\pet-state.json';
  const choices = (given: Partial<PetChoices> = {}): PetChoices => ({
    on: false, chosen: 'builtin/moon',
    pets: [
      // The pets of the application are named in the words of the page, and not as their packages name them.
      { key: 'builtin/moon', name: 'moon package', description: 'A small moon.', origin: 'builtin' },
      { key: 'builtin/cat', name: 'cat package', description: 'A navy cat.', origin: 'builtin' },
      { key: 'builtin/owl', name: 'Owl', description: 'Not drawn here.', origin: 'builtin' },
      { key: 'user/mochi', name: 'Mochi', description: 'A round friend.', origin: 'user' },
    ],
    folder: 'C:\\Users\\me\\AppData\\Roaming\\Columbina\\pets',
    stateFile,
    ...given,
  });
  function shell(first: PetChoices) {
    let now = first;
    let told: (said: { on: boolean }) => void = () => {};
    const bridge = {
      pets: vi.fn(async () => now),
      setPet: vi.fn(async (asked: { on?: boolean; chosen?: string }) => {
        now = { ...now, ...(asked.on === undefined ? {} : { on: asked.on }), ...(asked.chosen ? { chosen: asked.chosen } : {}) };
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

  it('shows the pet and hides it, chooses another one, and names the pets of the user as theirs', async () => {
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

  it('says where the task is told for other programs, and opens the folder for pets of the user', async () => {
    const { bridge } = shell(choices());
    await drawn();
    expect(one('pet-state-file').textContent).toBe(en.hint.pet.stateFile(stateFile));
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

  it('says where nothing can be shown, and what went wrong until something goes right', async () => {
    const { bridge } = shell(choices({ pets: [], chosen: null }));
    await drawn();
    expect(marked('pet-choices')).toEqual([]);
    expect(one('pet').textContent).toContain(en.hint.pet.none);
    expect((one('pet-on') as HTMLInputElement).disabled).toBe(true);
    bridge.openPetsFolder.mockRejectedValueOnce(new Error('No folder.'));
    await act(async () => { fireEvent.click(one('pet-folder')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('No folder.');
    cleanup();
    const other = shell(choices());
    await drawn();
    other.bridge.setPet.mockRejectedValueOnce('no');
    await act(async () => { fireEvent.click(one('pet-on')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('no');
    other.bridge.setPet.mockRejectedValueOnce(new Error('The pet cannot be shown.'));
    await act(async () => { fireEvent.click(one('pet-on')); });
    expect(one('pet').querySelector('[role="alert"]')?.textContent).toBe('The pet cannot be shown.');
    await act(async () => { fireEvent.click(one('pet-on')); });
    expect(one('pet').querySelector('[role="alert"]')).toBeNull();
  });
});
