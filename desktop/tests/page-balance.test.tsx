// @vitest-environment jsdom
/** The balance of the service in use, in the settings, against a shell that the test plays. */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BalanceLine } from '../src/app/BalanceLine';
import { Words } from '../src/app/locale';
import { messagesFor } from '../src/i18n';
import { likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const KNOWN = {
  service: 'deepseek', title: 'DeepSeek', supported: true, available: true,
  balances: [{ currency: 'CNY', total: '110.00', granted: '10.00', topped_up: '100.00' }],
};

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  const promise = new Promise<T>((resolve) => { release = resolve; });
  return { promise, release };
}

function shell(answer: unknown = KNOWN) {
  const balance = vi.fn(async (_service: string): Promise<unknown> => answer);
  vi.stubGlobal('columbina', { balance });
  return balance;
}

async function drawn(service = 'deepseek', language = 'en') {
  const result = render(<Words.Provider value={messagesFor(language)}><BalanceLine service={service} /></Words.Provider>);
  await act(async () => {});
  return result;
}

describe('the balance in the settings', () => {
  it('asks the service in use once, and says what its account has left', async () => {
    const balance = shell();
    await drawn();
    expect(balance.mock.calls).toEqual([['deepseek']]);
    expect(one('balance').dataset.kind).toBe('known');
    expect(one('balance-amount').textContent).toBe('DeepSeek: ¥110.00 left, of which ¥100.00 paid in and ¥10.00 given.');
    expect(marked('balance-cannot-pay')).toEqual([]);
  });

  it('says that it asks while the answer has not come, and asks again at a click', async () => {
    const balance = shell();
    const later = held<unknown>();
    balance.mockImplementationOnce(() => later.promise);
    await drawn();
    expect(one('balance').textContent).toContain('Asking the service how much money its account has left.');
    expect((one('balance-again') as HTMLButtonElement).disabled).toBe(true);
    await act(async () => later.release(KNOWN));
    expect((one('balance-again') as HTMLButtonElement).disabled).toBe(false);
    // Asked again, it says that it asks until the new answer has come.
    const again = held<unknown>();
    balance.mockImplementationOnce(() => again.promise);
    await act(async () => { fireEvent.click(one('balance-again')); });
    expect(balance).toHaveBeenCalledTimes(2);
    expect(one('balance').dataset.kind).toBe('unknown');
    expect(marked('balance-amount')).toEqual([]);
    await act(async () => again.release(KNOWN));
    expect(one('balance').dataset.kind).toBe('known');
  });

  it('takes the answer to the last question alone', async () => {
    const balance = shell();
    const first = held<unknown>();
    balance.mockImplementationOnce(() => first.promise);
    const { rerender } = await drawn('deepseek');
    rerender(<Words.Provider value={messagesFor('en')}><BalanceLine service="moonshot" /></Words.Provider>);
    await act(async () => {});
    await act(async () => first.release({ ...KNOWN, balances: [{ currency: 'CNY', total: '1.00' }] }));
    expect(one('balance-amount').textContent).toContain('¥110.00');
  });

  it('says that an account cannot pay, or names no money', async () => {
    shell({ ...KNOWN, available: false, balances: [] });
    await drawn();
    expect(one('balance').textContent).toContain('DeepSeek names no money in its account.');
    expect(one('balance-cannot-pay').textContent).toBe('The account cannot pay for calls: top it up on the page of the service.');
  });

  it('says that a service does not tell it', async () => {
    shell({ service: 'openai', title: 'OpenAI', supported: false });
    await drawn('openai');
    expect(one('balance').textContent).toContain('OpenAI does not tell a program how much money its account has left.');
  });

  it('says why it could not be asked, whether the program said it or the shell did not answer', async () => {
    const balance = shell({ service: 'deepseek', title: 'DeepSeek', supported: true, error: 'the key was refused', kind: 'key' });
    await drawn();
    expect(one('balance').dataset.kind).toBe('failed');
    expect(one('balance').textContent).toContain('DeepSeek could not be asked how much money its account has left.');
    expect(one('balance').querySelector('details')?.textContent).toContain('The key was refused.');
    balance.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await act(async () => { fireEvent.click(one('balance-again')); });
    expect(one('balance').querySelector('details')?.textContent).toContain('The setup program did not end in time.');
    expect(one('balance').textContent).toContain('deepseek could not be asked');
    balance.mockRejectedValueOnce('down');
    await act(async () => { fireEvent.click(one('balance-again')); });
    expect(one('balance').querySelector('details')?.textContent).toContain('Down.');
  });

  it('is not there without a service program to ask', async () => {
    vi.stubGlobal('columbina', undefined);
    await drawn();
    expect(marked('balance')).toEqual([]);
  });

  it('speaks Chinese', async () => {
    shell();
    await drawn('deepseek', 'zh-CN');
    expect(one('balance-amount').textContent).toBe('DeepSeek 账户余额 ¥110.00，其中充值 ¥100.00，赠送 ¥10.00。');
    expect(one('balance-again').textContent).toBe('查询余额');
  });
});
