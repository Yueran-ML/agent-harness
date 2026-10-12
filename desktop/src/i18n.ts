/** Picking the words of the interface by the language of the user. */
import { en, type Messages } from './messages/en';
import { ja } from './messages/ja';
import { zhCN } from './messages/zh-CN';

// The catalogues by language, each with the name that a page gives its language. A
// language that has several, one for each region, would need more than this.
const byLanguage: Record<string, { words: Messages; tag: string }> = {
  en: { words: en, tag: 'en' },
  ja: { words: ja, tag: 'ja' },
  zh: { words: zhCN, tag: 'zh-CN' },
};

function found(locale: string) {
  const language = locale.trim().toLowerCase().split(/[-_]/)[0];
  return Object.hasOwn(byLanguage, language) ? byLanguage[language] : byLanguage.en;
}

/**
 * Return the catalogue for a locale such as `zh-CN` or `en_US`.
 *
 * Only the language counts. Every kind of Chinese gets the one Chinese
 * catalogue that there is: whoever reads traditional characters reads the
 * simplified ones more easily than English. A language without a catalogue
 * gets English.
 */
export function messagesFor(locale: string): Messages {
  return found(locale).words;
}

/**
 * Return the language of the catalogue that `messagesFor` gives for a locale,
 * as a page names its language. It is not always the language of the locale:
 * a German system is shown English words, and the page has to say so.
 */
export function languageFor(locale: string): string {
  return found(locale).tag;
}
