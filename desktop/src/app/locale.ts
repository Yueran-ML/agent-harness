/** The words of the interface, for every part of it that says something. */
import { createContext, useContext } from 'react';
import { languageFor, messagesFor } from '../i18n';
import { en, type Messages } from '../messages/en';

/** What the user chose: a language, or to go by the system. */
export type LanguageChoice = 'system' | 'en' | 'zh';

/** The languages that can be chosen, each named in itself. */
export const languages: { id: Exclude<LanguageChoice, 'system'>; name: string }[] = [
  { id: 'zh', name: '中文' },
  { id: 'en', name: 'English' },
];

const storageKey = 'columbina-language';

export function storedLanguage(): LanguageChoice {
  try {
    const stored = localStorage.getItem(storageKey);
    if (stored === 'en' || stored === 'zh') return stored;
  } catch {
    // Without storage the choice holds for this window.
  }
  return 'system';
}

export function storeLanguage(choice: LanguageChoice): void {
  try {
    if (choice === 'system') localStorage.removeItem(storageKey);
    else localStorage.setItem(storageKey, choice);
  } catch {
    // See above.
  }
}

/** The locale that a choice stands for. */
export function localeOf(choice: LanguageChoice): string {
  return choice === 'system' ? navigator.language : choice;
}

export function wordsFor(choice: LanguageChoice): Messages {
  return messagesFor(localeOf(choice));
}

/** The language of the words that a choice leads to, as a page names its language. */
export function shownLanguage(choice: LanguageChoice): string {
  return languageFor(localeOf(choice));
}

export const Words = createContext<Messages>(en);

/** The catalogue of the language in use. */
export function useWords(): Messages {
  return useContext(Words);
}
