import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { languages, localeOf, shownLanguage, storedLanguage, storeLanguage, wordsFor } from '../src/app/locale';
import { languageFor } from '../src/i18n';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import { initialSize, sizes, sizeStorageKey, storeSize } from '../src/size';
import { initialTheme, storeTheme, themes, themeStorageKey } from '../src/themes';
import { blockOf, colourOf, contrast, over, rgbOf, selectorOf, themeColour, tokens, washOf } from './stylesheet';

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

describe('the size of type', () => {
  const sheets = ['src/ui/tokens.css', 'src/ui/base.css', 'src/ui/shell.css', 'src/ui/conversation.css', 'src/ui/composer.css', 'src/key/key.css'];
  const scaleOf = (block: string) => Number(/^\s*--type:\s*([\d.]+);/m.exec(block)?.[1]);

  it('is the standard one until another is chosen', () => {
    vi.stubGlobal('localStorage', storage());
    expect(initialSize()).toBe('standard');
    expect(sizes).toEqual(['small', 'standard', 'large']);
  });

  it('is the one that was chosen last', () => {
    const kept = storage();
    vi.stubGlobal('localStorage', kept);
    storeSize('large');
    expect(kept.kept).toEqual({ [sizeStorageKey]: 'large' });
    expect(initialSize()).toBe('large');
    storeSize('small');
    expect(initialSize()).toBe('small');
    storeSize('standard');
    expect(initialSize()).toBe('standard');
  });

  it('is the standard one where what was kept names no size', () => {
    for (const kept of ['huge', '', 'LARGE', 'constructor', '1.14']) {
      vi.stubGlobal('localStorage', storage({ [sizeStorageKey]: kept }));
      expect(initialSize()).toBe('standard');
    }
  });

  it('does without a place to keep the choice', () => {
    vi.stubGlobal('localStorage', broken);
    expect(initialSize()).toBe('standard');
    expect(() => storeSize('large')).not.toThrow();
  });

  it('has a name in every language', () => {
    for (const words of [en, zhCN, ja]) {
      expect(Object.keys(words.label.typeSizes).sort()).toEqual([...sizes].sort());
      expect(new Set(Object.values(words.label.typeSizes)).size).toBe(sizes.length);
    }
  });

  it('is smaller and larger than the standard one by a step that can be seen and does not break a line of buttons', () => {
    expect(scaleOf(blockOf(tokens, ':root'))).toBe(1);
    const small = scaleOf(blockOf(tokens, ":root[data-size='small']"));
    const large = scaleOf(blockOf(tokens, ":root[data-size='large']"));
    expect(small).toBeGreaterThanOrEqual(0.88);
    expect(small).toBeLessThanOrEqual(0.95);
    expect(large).toBeGreaterThanOrEqual(1.1);
    expect(large).toBeLessThanOrEqual(1.2);
    // No size in the stylesheet that cannot be chosen, and none that can be chosen without its rule.
    const named = [...tokens.matchAll(/data-size='([a-z]+)'/g)].map(found => found[1]);
    expect(named.sort()).toEqual(sizes.filter(size => size !== 'standard').sort());
  });

  it('is followed by every size that a stylesheet writes', () => {
    // A size that is written in pixels alone would stay as it is when the user chooses another.
    for (const sheet of sheets) {
      const written = [...read(sheet).matchAll(/font-size:\s*([^;]+);/g)].map(found => found[1]);
      for (const size of written) {
        expect(size, sheet).toMatch(/^(var\(--text-[a-z]+\)|calc\([\d.]+px \* var\(--type\)\)|[\d.]+em)$/);
      }
    }
    for (const name of ['caption', 'ui', 'read', 'title', 'display']) {
      expect(blockOf(tokens, ':root')).toMatch(new RegExp(`^\\s*--text-${name}: calc\\([\\d.]+px \\* var\\(--type\\)\\);$`, 'm'));
    }
  });

  it('is kept under a name that nothing else is kept under', () => {
    // Every choice that the window keeps has a name of its own. Two under one name would undo each other.
    const sources = ['src/app/App.tsx', 'src/app/locale.ts', 'src/app/opening.ts', 'src/BackgroundMusic.tsx', 'src/size.ts', 'src/themes.ts'];
    const names = sources.flatMap(file => [...read(file).matchAll(/[kK]ey = '(columbina-[^']+)';/g)].map(found => found[1]));
    // One of them keeps whether the conversation beside the studio is put away.
    expect(names).toHaveLength(7);
    expect(names).toContain('columbina-studio-conversation');
    expect(new Set(names).size).toBe(names.length);
    expect(names).toContain(sizeStorageKey);
    expect(names).toContain(themeStorageKey);
  });

  it('is on both pages before anything is drawn on them', () => {
    for (const page of ['src/main.tsx', 'src/key/main.tsx']) {
      expect(read(page), page).toContain('document.documentElement.dataset.size = initialSize();');
    }
  });
});

describe('a sheet', () => {
  const base = read('src/ui/base.css');
  const rulesOf = (selector: string) => blockOf(base, selector).split('\n').slice(2).map(rule => rule.trim());

  it('gives its head and its foot the room that they take, and what lies between them the rest', () => {
    // Were the three taller together than the sheet, whatever is scrolled into view in it would
    // push its head out at the top: the settings do that when they open at one of their parts.
    expect(rulesOf('dialog.sheet[open]')).toEqual(expect.arrayContaining(['display: flex;', 'flex-direction: column;']));
    expect(rulesOf('.sheet-head')).toContain('flex: none;');
    expect(rulesOf('.sheet-foot')).toContain('flex: none;');
    expect(rulesOf('.sheet-body')).toEqual(expect.arrayContaining(['flex: 1;', 'min-height: 0;', 'overflow: auto;']));
    // The sheet is as tall as the window lets it be. Nothing in it has a height of its own to add to that.
    expect(rulesOf('dialog.sheet')).toContain('max-height: calc(100vh - 96px);');
    expect(rulesOf('.sheet-body').filter(rule => rule.includes('height') && rule !== 'min-height: 0;')).toEqual([]);
  });

  it('is laid out only while it is open', () => {
    // A dialog that is closed is not shown. A rule of the page that lays it out would show it.
    expect(rulesOf('dialog.sheet').filter(rule => rule.startsWith('display'))).toEqual([]);
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
  const rulesIn = (stylesheet: string, selector: string) =>
    blockOf(stylesheet, selector).split('\n').slice(2).map(rule => rule.trim());
  const rulesOf = (selector: string) => rulesIn(base, selector);

  it('break between phrases where the page is in Japanese, and do not end with a character or two', () => {
    // Japanese may break between any two characters: in the middle of a word, and before the last one of a sentence.
    expect(rulesOf(':root:lang(ja)')).toEqual(['word-break: auto-phrase;', 'text-wrap: pretty;']);
  });

  it('stay where they are in a field that is typed into', () => {
    expect(rulesOf(':root:lang(ja) :is(input, textarea)')).toEqual(['text-wrap: wrap;']);
    // The rule for a field comes after the one for the page, and so goes before it.
    expect(base.indexOf(':root:lang(ja) :is(input, textarea) {')).toBeGreaterThan(base.indexOf(':root:lang(ja) {'));
  });

  it('are of about the same length in what the window for the key says', () => {
    // Lines that were filled to their end one after the other cut the name of a model at its
    // hyphen, in English as in Japanese: "with the model deepseek-" and then "flash."
    expect(rulesIn(read('src/key/key.css'), '.key-said p')).toEqual(['text-wrap: balance;']);
  });
});

describe('the status line under full access', () => {
  const shell = read('src/ui/shell.css');
  const line = read('src/app/StatusLine.tsx');
  const rules = blockOf(shell, ".status[data-tone='warn']").split('\n').slice(2).map(rule => rule.trim());

  it('is in the colour of a warning as a whole', () => {
    expect(rules).toContain('background: var(--warn-wash);');
    expect(rules).toContain('color: var(--warn);');
    // The line has the tone, and not one part of it. Full access is what gives it, outside a chat.
    expect(line).toContain("const full = !chat && status?.sandbox?.mode === 'danger-full-access';");
    expect(line).toContain(`<footer className="status" data-tone={full ? 'warn' : undefined} data-test="status">`);
    expect(line.match(/data-tone=\{full/g)).toHaveLength(1);
  });

  it('keeps its height, so that nothing moves when it comes and goes', () => {
    // A border would add to the line. The edge is drawn inside it.
    expect(rules.filter(rule => /^(border|padding|margin|min-height|height)/.test(rule))).toEqual([]);
    expect(rules.some(rule => rule.startsWith('box-shadow: inset 0 1px 0 '))).toBe(true);
    // As a band of colour it has an edge, which must not lie against the field above it.
    expect(blockOf(shell, '.status')).toContain('  margin-top: var(--s2);');
  });

  it.each(themes)('can be read in the theme %s', (theme) => {
    // The words of a warning on the wash of a warning, over the page.
    const under = over(washOf(theme, 'warn-wash'), rgbOf(themeColour(theme, 'ink-0')));
    expect(contrast(rgbOf(themeColour(theme, 'warn')), under)).toBeGreaterThanOrEqual(4.5);
  });

  it('measures contrast as the guidelines do', () => {
    expect(contrast([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 5);
    expect(contrast([255, 255, 255], [0, 0, 0])).toBeCloseTo(21, 5);
    expect(contrast([119, 119, 119], [255, 255, 255])).toBeCloseTo(4.48, 2);
    expect(contrast([10, 20, 30], [10, 20, 30])).toBe(1);
    expect(rgbOf('#e3c07e')).toEqual([227, 192, 126]);
    // Half of white over black is the grey between them.
    expect(over({ rgb: [255, 255, 255], alpha: 0.5 }, [0, 0, 0])).toEqual([127.5, 127.5, 127.5]);
    expect(washOf(themes[0], 'warn-wash')).toEqual({ rgb: [227, 192, 126], alpha: 0.1 });
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
