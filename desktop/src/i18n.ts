/** Picking the words of the interface by the language of the user. */
import { en, type Messages } from './messages/en';
import { zhCN } from './messages/zh-CN';

// The catalogues by language. A language that has several, one for each
// region, would need more than this.
const byLanguage: Record<string, Messages> = {
  en,
  zh: zhCN,
};

/**
 * Return the catalogue for a locale such as `zh-CN` or `en_US`.
 *
 * Only the language counts. Every kind of Chinese gets the one Chinese
 * catalogue that there is: whoever reads traditional characters reads the
 * simplified ones more easily than English. A language without a catalogue
 * gets English.
 */
export function messagesFor(locale: string): Messages {
  const language = locale.trim().toLowerCase().split(/[-_]/)[0];
  return Object.hasOwn(byLanguage, language) ? byLanguage[language] : en;
}
