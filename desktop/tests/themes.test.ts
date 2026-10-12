import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { languages, localeOf, shownLanguage, storedLanguage, storeLanguage, wordsFor } from '../src/app/locale';
import { languageFor } from '../src/i18n';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
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
    for (const words of [en, zhCN, ja]) {
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

describe('the faces of the type', () => {
  /** The faces that a block of the stylesheet names for a kind of type, in their order. */
  const facesOf = (selector: string, name: string): string[] => {
    const found = new RegExp(`--${name}:([^;]+);`).exec(blockOf(tokens, selector));
    if (!found) throw new Error(`${selector} names no faces for ${name}.`);
    return found[1].split(',').map(face => face.trim().replace(/^"|"$/g, ''));
  };
  const kinds = ['font-ui', 'font-display', 'font-mono'];

  it('are named for every language that has words and is not written in Latin letters', () => {
    const written = languages.map(language => languageFor(language.id).split('-')[0]).filter(language => language !== 'en');
    expect(written.sort()).toEqual(['ja', 'zh']);
    for (const language of written) {
      // The display face of the page as it comes is a serif that no computer has for these languages.
      expect(facesOf(`:root:lang(${language})`, 'font-display')).not.toEqual(facesOf(':root', 'font-display'));
    }
  });

  it('are Japanese ones where the page is in Japanese', () => {
    // A face for Chinese has kana and kanji too. Named before a Japanese one, it would draw them.
    const chinese = /\bSC\b|\bTC\b|YaHei|PingFang|Songti|STSong|SimSun|JhengHei/;
    for (const kind of kinds) {
      const faces = facesOf(':root:lang(ja)', kind);
      expect(faces.filter(face => chinese.test(face)), kind).toEqual([]);
      // Latin letters are drawn as in every other language. The display face is a sans here,
      // and its Latin letters are those of the words of the interface.
      expect(faces[0], kind).toBe(facesOf(':root', kind === 'font-display' ? 'font-ui' : kind)[0]);
      // And the last is a kind of face, for a computer that has none of those named.
      expect(faces.at(-1), kind).toMatch(/^(sans-serif|monospace)$/);
    }
    // What the page as it comes names for Chinese is why the block is needed.
    expect(facesOf(':root', 'font-ui').filter(face => chinese.test(face))).not.toEqual([]);
    // The face that every Windows has for Japanese draws the words of the interface.
    for (const kind of ['font-ui', 'font-display']) {
      const faces = facesOf(':root:lang(ja)', kind);
      expect(faces, kind).toContain('Yu Gothic UI');
      expect(faces.indexOf('Yu Gothic UI'), kind).toBeLessThan(faces.indexOf('Meiryo UI'));
    }
  });
});

describe('the lines of a paragraph', () => {
  const base = read('src/ui/base.css');
  const rulesOf = (selector: string) => blockOf(base, selector).split('\n').slice(2).map(rule => rule.trim());

  it('break between phrases where the page is in Japanese, and do not end with a character or two', () => {
    // Japanese may break between any two characters: in the middle of a word, and before the last one of a sentence.
    expect(rulesOf(':root:lang(ja)')).toEqual(['word-break: auto-phrase;', 'text-wrap: pretty;']);
  });

  it('stay where they are in a field that is typed into', () => {
    expect(rulesOf(':root:lang(ja) :is(input, textarea)')).toEqual(['text-wrap: wrap;']);
    // The rule for a field comes after the one for the page, and so goes before it.
    expect(base.indexOf(':root:lang(ja) :is(input, textarea) {')).toBeGreaterThan(base.indexOf(':root:lang(ja) {'));
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
    storeLanguage('ja');
    expect(storedLanguage()).toBe('ja');
    expect(wordsFor(storedLanguage())).toBe(ja);
  });

  it('can be every language that the settings offer, and each has words of its own', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    vi.stubGlobal('navigator', { language: 'de-DE' });
    const offered = languages.map(language => language.id);
    expect(offered).toEqual(['zh', 'en', 'ja']);
    const shown = offered.map((id) => {
      storeLanguage(id);
      expect(storedLanguage()).toBe(id);
      return wordsFor(id);
    });
    expect(new Set(shown).size).toBe(offered.length);
    // Each is named in itself, so that it is found by who reads no other.
    expect(languages.map(language => language.name)).toEqual(['中文', 'English', '日本語']);
  });

  it('is named to the page as the language of the words that are shown', () => {
    // A page that says it is German and shows English words is read out wrongly.
    vi.stubGlobal('localStorage', storage());
    const shown = [
      ['zh-CN', 'zh-CN'], ['zh-TW', 'zh-CN'], ['zh', 'zh-CN'], ['en-GB', 'en'], ['de-DE', 'en'], ['ja-JP', 'ja'], ['ja', 'ja'],
      ['ko-KR', 'en'],
    ];
    for (const [system, language] of shown) {
      vi.stubGlobal('navigator', { language: system });
      expect(shownLanguage('system')).toBe(language);
      expect(languageFor(system)).toBe(language);
    }
    vi.stubGlobal('navigator', { language: 'de-DE' });
    expect(shownLanguage('zh')).toBe('zh-CN');
    expect(shownLanguage('en')).toBe('en');
    expect(shownLanguage('ja')).toBe('ja');
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
    // What is kept is the short name of a language, and not the name that the settings show it by.
    for (const kept of ['ko', 'ja-JP', 'system', '', 'EN', 'toString', 'English', '日本語']) {
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
