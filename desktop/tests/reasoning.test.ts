// @vitest-environment jsdom
/** The language that the model reasons in: what a choice leads to, and what is kept of it. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  reasoningLanguage, rememberTold, storedReasoning, storeReasoning, toldReasoning,
} from '../src/app/reasoning';

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('the language that a choice leads to', () => {
  it('is the one that was chosen, or none', () => {
    expect(reasoningLanguage('zh', 'en')).toBe('zh');
    expect(reasoningLanguage('en', 'zh-CN')).toBe('en');
    expect(reasoningLanguage('none', 'ja')).toBe('none');
  });

  it('is that of the interface where the choice says so, and English for one that is not known', () => {
    expect(reasoningLanguage('interface', 'zh-CN')).toBe('zh');
    expect(reasoningLanguage('interface', 'ja')).toBe('ja');
    expect(reasoningLanguage('interface', 'en')).toBe('en');
    expect(reasoningLanguage('interface', 'fr')).toBe('en');
  });
});

describe('what is kept of the choice', () => {
  it('asks for no language where nothing was chosen, or what was kept is no choice', () => {
    expect(storedReasoning()).toBe('none');
    localStorage.setItem('columbina-reasoning-language', 'fr');
    expect(storedReasoning()).toBe('none');
  });

  it('is the choice, and the language that the program was told last', () => {
    storeReasoning('interface');
    expect(storedReasoning()).toBe('interface');
    expect(toldReasoning()).toBeNull();
    rememberTold('ja');
    expect(toldReasoning()).toBe('ja');
  });

  it('holds for the window alone where there is no storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied'); });
    storeReasoning('zh');
    rememberTold('zh');
    expect(storedReasoning()).toBe('none');
    expect(toldReasoning()).toBeNull();
  });
});
