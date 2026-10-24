/** What the page makes of the balance of a service: tried with plain values. */
import { describe, expect, it } from 'vitest';
import { balanceOf, money } from '../src/app/balance';

describe('the balance of a service, as the program says it', () => {
  it('is what the account has in each currency, and whether it can pay', () => {
    expect(balanceOf({
      service: 'deepseek', title: 'DeepSeek', supported: true, available: true,
      balances: [{ currency: 'CNY', total: '110.00', granted: '10.00', topped_up: '100.00' }],
    })).toEqual({
      kind: 'known', title: 'DeepSeek', available: true,
      amounts: [{ currency: 'CNY', total: '110.00', granted: '10.00', toppedUp: '100.00' }],
    });
    // What leaves out a part has none of it, an amount of no currency or total is none, and
    // an account that is not said to pay cannot.
    expect(balanceOf({
      service: 'deepseek', title: 'DeepSeek', supported: true,
      balances: [{ currency: 'USD', total: '0.00' }, { currency: 'CNY' }, { total: '1' }, 'much'],
    })).toEqual({
      kind: 'known', title: 'DeepSeek', available: false,
      amounts: [{ currency: 'USD', total: '0.00', granted: '0', toppedUp: '0' }],
    });
    expect(balanceOf({ service: 'deepseek', title: 'DeepSeek', supported: true, available: true, balances: 'none' }))
      .toMatchObject({ kind: 'known', amounts: [] });
  });

  it('is not told by a service that does not tell it', () => {
    expect(balanceOf({ service: 'openai', title: 'OpenAI', supported: false })).toEqual({ kind: 'unsupported', title: 'OpenAI' });
  });

  it('could not be asked where the program says why, or answers nothing that it can be read from', () => {
    expect(balanceOf({ service: 'deepseek', title: 'DeepSeek', supported: true, error: 'The key was refused.', kind: 'key' }))
      .toEqual({ kind: 'failed', title: 'DeepSeek', said: 'The key was refused.' });
    expect(balanceOf({ error: 'there is no service named \'x\'' })).toEqual({ kind: 'failed', title: '', said: 'there is no service named \'x\'' });
    expect(balanceOf(null)).toEqual({ kind: 'failed', title: '', said: '' });
    // A service of no title is called by its name.
    expect(balanceOf({ service: 'mine', supported: false })).toEqual({ kind: 'unsupported', title: 'mine' });
  });
});

describe('an amount of money', () => {
  it('has the sign of its currency, or the name of one that has none here', () => {
    expect(money('110.00', 'CNY')).toBe('¥110.00');
    expect(money('3.50', 'USD')).toBe('$3.50');
    expect(money('12.5', 'EUR')).toBe('12.5 EUR');
  });
});
