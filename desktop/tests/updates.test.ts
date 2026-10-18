// @vitest-environment jsdom
/**
 * Newer versions of the application, as the page reads what the shell says of them, and keeps
 * what the user chose about them.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  aboutOf, foundOf, storeChecking, storedChecking, storedSkipped, storeSkipped, toldOf, UPDATE_EVERY_MS, UPDATE_FIRST_MS,
} from '../src/app/updates';

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

const page = 'https://github.com/Yueran-ML/agent-harness/releases/tag/desktop-v0.1.3';

describe('what the shell says of the application', () => {
  it('is the version that runs, and whether it looks for newer ones', () => {
    expect(aboutOf({ version: '0.1.2', checks: true })).toEqual({ version: '0.1.2', checks: true });
    expect(aboutOf({ version: '0.1.2-dev', checks: false, more: 1 })).toEqual({ version: '0.1.2-dev', checks: false });
    expect(aboutOf({ version: 'x'.repeat(40), checks: true })).toEqual({ version: 'x'.repeat(40), checks: true });
  });

  it('is nothing where it is not said in so many words', () => {
    const unsaid = [
      null, undefined, 'about', [], {}, { version: '0.1.2' }, { checks: true }, { version: '', checks: true },
      { version: '0.1 2', checks: true }, { version: 'x'.repeat(41), checks: true }, { version: 2, checks: true },
      { version: '0.1.2', checks: 'yes' },
    ];
    for (const answer of unsaid) expect(aboutOf(answer)).toBeNull();
  });
});

describe('what a question for a newer version found', () => {
  it('is a newer version, with the page of its release', () => {
    expect(foundOf({ newer: '0.1.3', page, failure: null })).toEqual({ newer: { version: '0.1.3', page }, failure: null });
    expect(foundOf({ newer: '12.0.10', page, failure: null, more: true })).toEqual({ newer: { version: '12.0.10', page }, failure: null });
  });

  it('is none, or why that is not known', () => {
    expect(foundOf({ newer: null, page: null, failure: null })).toEqual({ newer: null, failure: null });
    for (const failure of ['network', 'busy', 'unreadable'] as const) {
      expect(foundOf({ newer: null, page: null, failure })).toEqual({ newer: null, failure });
    }
  });

  it('is a list that did not come, where the answer cannot be read', () => {
    const unread = [
      null, undefined, 'newer', [], {}, { newer: null }, { newer: null, failure: 'later' }, { newer: null, failure: 7 },
      { newer: '0.1', page, failure: null }, { newer: '0.1.3-beta', page, failure: null }, { newer: 'v0.1.3', page, failure: null },
      { newer: '1234567.0.0', page, failure: null }, { newer: 3, page, failure: null },
      // The page of a release is an address of the web over https.
      { newer: '0.1.3', page: 'http://github.com/releases', failure: null },
      { newer: '0.1.3', page: 'https://github.com/a b', failure: null },
      { newer: '0.1.3', page: 'https://', failure: null }, { newer: '0.1.3', page: 7, failure: null }, { newer: '0.1.3', failure: null },
      // A version that was found is no failure.
      { newer: '0.1.3', page, failure: 'busy' }, { newer: '0.1.3', page },
    ];
    for (const answer of unread) expect(foundOf(answer)).toEqual({ newer: null, failure: 'unreadable' });
  });
});

describe('the newer version that the bar of the window points to', () => {
  const found = { newer: { version: '0.1.3', page }, failure: null };

  it('is one that was found and not skipped', () => {
    expect(toldOf(found, null)).toEqual({ version: '0.1.3', page });
    expect(toldOf(found, '0.1.2')).toEqual({ version: '0.1.3', page });
    expect(toldOf(found, '0.1.3')).toBeNull();
  });

  it('is none where none was found', () => {
    expect(toldOf({ newer: null, failure: null }, null)).toBeNull();
    expect(toldOf({ newer: null, failure: 'network' }, null)).toBeNull();
    expect(toldOf(null, null)).toBeNull();
  });
});

describe('what the user chose', () => {
  it('is to look by itself, unless the user said no', () => {
    expect(storedChecking()).toBe(true);
    storeChecking(false);
    expect(localStorage.getItem('columbina-check-updates')).toBe('false');
    expect(storedChecking()).toBe(false);
    storeChecking(true);
    expect(localStorage.getItem('columbina-check-updates')).toBe('true');
    expect(storedChecking()).toBe(true);
    localStorage.setItem('columbina-check-updates', 'no');
    expect(storedChecking()).toBe(true);
  });

  it('keeps the version that was skipped, where it is one', () => {
    expect(storedSkipped()).toBeNull();
    storeSkipped('0.1.3');
    expect(localStorage.getItem('columbina-update-skipped')).toBe('0.1.3');
    expect(storedSkipped()).toBe('0.1.3');
    localStorage.setItem('columbina-update-skipped', '0.1.3 and more');
    expect(storedSkipped()).toBeNull();
  });

  it('holds where nothing can be kept', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('No storage.'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('No storage.'); });
    expect(storedChecking()).toBe(true);
    expect(storedSkipped()).toBeNull();
    expect(() => storeChecking(false)).not.toThrow();
    expect(() => storeSkipped('0.1.3')).not.toThrow();
  });
});

it('looks a few seconds after the window opened, and once a day after that', () => {
  expect([UPDATE_FIRST_MS, UPDATE_EVERY_MS]).toEqual([5_000, 86_400_000]);
});
