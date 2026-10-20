/** The blanks between the words of Chinese or Japanese and what is put into them. */

/**
 * A count, as it stands before a word of Chinese or Japanese.
 *
 * A blank stands between a digit or a Latin letter and a character of these
 * languages, so one follows a count that ends in a digit, as `2670` does. None
 * stands between two of their characters, so none follows a count that ends in
 * one, as `1.2万` does.
 */
export function beforeWord(count: string): string {
  return /[0-9A-Za-z]$/.test(count) ? `${count} ` : count;
}

/** A name, as it stands after a word of Chinese or Japanese: with a blank before it where it begins in a Latin letter or a digit. */
export function afterWord(name: string): string {
  return /^[0-9A-Za-z]/.test(name) ? ` ${name}` : name;
}

/** A name, as it stands between two words of Chinese or Japanese. */
export function between(name: string): string {
  return afterWord(beforeWord(name));
}
