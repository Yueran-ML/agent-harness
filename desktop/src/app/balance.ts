/**
 * What the page makes of the balance of a service: how much money the account of its key has
 * left, as `columbina balance --json` says. Nothing here draws, so all of it can be tried with
 * plain values.
 */

/** What an account has in one currency, as the service wrote the numbers. */
export interface Amount {
  currency: string;
  total: string;
  granted: string;
  toppedUp: string;
}

/** The balance of a service, as far as the page knows. */
export type Balance =
  /** Not asked yet, or the answer has not come. */
  | { kind: 'unknown' }
  /** The service does not tell a program how much money its account has left. */
  | { kind: 'unsupported'; title: string }
  /** It could not be asked: what the program said. */
  | { kind: 'failed'; title: string; said: string }
  | { kind: 'known'; title: string; available: boolean; amounts: Amount[] };

const text = (value: unknown): string => (typeof value === 'string' ? value : '');
const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});

/** Read what `columbina balance --json` answered. A service of no title is called by its name. */
export function balanceOf(answer: unknown): Balance {
  const given = fields(answer);
  const title = text(given.title) || text(given.service);
  if (given.supported === false) return { kind: 'unsupported', title };
  if (given.supported !== true || 'error' in given) return { kind: 'failed', title, said: text(given.error) };
  const amounts = (Array.isArray(given.balances) ? given.balances : []).map(fields)
    .filter(one => text(one.currency) !== '' && text(one.total) !== '')
    .map(one => ({
      currency: text(one.currency), total: text(one.total), granted: text(one.granted) || '0', toppedUp: text(one.topped_up) || '0',
    }));
  return { kind: 'known', title, available: given.available === true, amounts };
}

const SIGNS: Record<string, string> = { CNY: '¥', USD: '$' };

/** An amount of money as the page shows it: with the sign of its currency, or with the name of one that has none here. */
export function money(amount: string, currency: string): string {
  const sign = SIGNS[currency];
  return sign === undefined ? `${amount} ${currency}` : `${sign}${amount}`;
}
