import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { localeOf, storedLanguage, storeLanguage, wordsFor } from '../src/app/locale';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { initialTheme, storeTheme, themes, themeStorageKey } from '../src/themes';
import { blockOf, colourOf, selectorOf, themeColour, tokens } from './stylesheet';

const read = (file: string) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');

/** A place to keep things, as a window has one. */
function storage(kept: Record<string, string> = {}) {
  return {
    getItem: (key: string) => (Object.hasOwn(kept, key) ? kept[key] : null),
    setItem: (key: string, value: string) => { kept[key] = value; },
    removeItem: (key: string) => { delete kept[key]; },
    kept,
  };
}
const broken = {
  getItem: () => { throw new Error('no storage'); },
  setItem: () => { throw new Error('no storage'); },
  removeItem: () => { throw new Error('no storage'); },
};

afterEach(() => vi.unstubAllGlobals());

describe('the themes', () => {
  it('have their colours in the stylesheet, each under its own name', () => {
    for (const theme of themes) {
      const block = blockOf(tokens, selectorOf(theme));
      // Every theme says for itself what its surfaces and its text look like.
      for (const name of ['ink-0', 'ink-1', 'ink-2', 'ink-3', 'ink-4', 'moon-0', 'moon-1', 'moon-2', 'moon-3', 'accent']) {
        expect(colourOf(block, name)).toMatch(/^#[0-9a-f]{6}$/);
      }
    }
    // No theme in the stylesheet that cannot be chosen.
    const named = [...tokens.matchAll(/data-theme='([a-z]+)'/g)].map(found => found[1]);
    expect([...new Set(named)].sort()).toEqual(themes.slice(1).sort());
  });

  it('have a name in every language', () => {
    for (const words of [en, zhCN]) {
      expect(Object.keys(words.label.themeNames).sort()).toEqual([...themes].sort());
    }
  });

  it('begin with the dark one, which is what the page is drawn in before anything is read', () => {
    vi.stubGlobal('localStorage', storage());
    expect(initialTheme()).toBe('moonveil');
    expect(themes[0]).toBe('moonveil');
    const page = read('index.html');
    expect(page).toContain('data-theme="moonveil"');
    expect(page).toContain(`<meta name="theme-color" content="${themeColour('moonveil', 'ink-0')}" />`);
  });

  it('are the one that was chosen last', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    storeTheme('moonwhite');
    expect(kept.kept).toEqual({ [themeStorageKey]: 'moonwhite' });
    expect(initialTheme()).toBe('moonwhite');
    storeTheme('moonveil');
    expect(initialTheme()).toBe('moonveil');
  });

  it('are the dark one where what was kept names no theme of today', () => {
    for (const kept of ['frostlight', 'silvermoon', '', 'MOONWHITE', 'constructor']) {
      vi.stubGlobal('localStorage', storage({ [themeStorageKey]: kept }));
      expect(initialTheme()).toBe('moonveil');
    }
  });

  it('do without a place to keep the choice', () => {
    vi.stubGlobal('localStorage', broken);
    expect(initialTheme()).toBe('moonveil');
    expect(() => storeTheme('moonwhite')).not.toThrow();
  });
});

describe('the language', () => {
  it('is that of the system until one is chosen', () => {
    vi.stubGlobal('localStorage', storage());
    expect(storedLanguage()).toBe('system');
    vi.stubGlobal('navigator', { language: 'zh-CN' });
    expect(localeOf('system')).toBe('zh-CN');
    expect(wordsFor('system')).toBe(zhCN);
    vi.stubGlobal('navigator', { language: 'de-DE' });
    expect(wordsFor('system')).toBe(en);
  });

  it('is the one that was chosen, whatever the system has', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    vi.stubGlobal('navigator', { language: 'zh-CN' });
    storeLanguage('en');
    expect(storedLanguage()).toBe('en');
    expect(wordsFor(storedLanguage())).toBe(en);
    storeLanguage('zh');
    expect(storedLanguage()).toBe('zh');
    expect(localeOf('zh')).toBe('zh');
    expect(wordsFor('zh')).toBe(zhCN);
  });

  it('goes by the system again once the choice is given up', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    storeLanguage('en');
    storeLanguage('system');
    expect(kept.kept).toEqual({});
    expect(storedLanguage()).toBe('system');
  });

  it('goes by the system where what was kept names no language that there is', () => {
    for (const kept of ['ja', 'system', '', 'EN', 'toString']) {
      vi.stubGlobal('localStorage', storage({ 'columbina-language': kept }));
      expect(storedLanguage()).toBe('system');
    }
  });

  it('does without a place to keep the choice', () => {
    vi.stubGlobal('localStorage', broken);
    expect(storedLanguage()).toBe('system');
    expect(() => storeLanguage('en')).not.toThrow();
    expect(() => storeLanguage('system')).not.toThrow();
  });
});
