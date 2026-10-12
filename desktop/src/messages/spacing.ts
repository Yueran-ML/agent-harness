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
