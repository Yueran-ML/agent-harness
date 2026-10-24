import { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { sentenceOf } from '../key/form';
import { balanceOf, money, type Balance } from './balance';
import { Details } from './GitHubSignIn';
import { useWords } from './locale';

/**
 * How much money the account of the service in use has left, in the settings, with a way to ask
 * again. The service is asked when the line is drawn and when it is asked again, and no more
 * often. Where there is no service program to ask, as in a replay, the line is not there.
 */
export function BalanceLine({ service }: { service: string }) {
  const words = useWords();
  const [balance, setBalance] = useState<Balance>({ kind: 'unknown' });
  // Only the answer to the last question counts: the service may be another one by then.
  const asked = useRef(0);
  const ask = useCallback(async () => {
    const bridge = window.columbina;
    if (!bridge) return;
    const number = ++asked.current;
    setBalance({ kind: 'unknown' });
    let answer: Balance;
    try {
      answer = balanceOf(await bridge.balance(service));
    } catch (problem) {
      answer = { kind: 'failed', title: service, said: problem instanceof Error ? problem.message : String(problem) };
    }
    if (number === asked.current) setBalance(answer);
  }, [service]);
  useEffect(() => { void ask(); }, [ask]);

  if (!window.columbina) return null;
  const hints = words.hint.balance;
  return <div className="balance" data-test="balance" data-kind={balance.kind}>
    <div className="setting-row">
      <div className="setting-grow">
        {balance.kind === 'unknown' && <p className="caption github-wait-words" role="status">
          <span className="moon" aria-hidden="true" />{hints.reading}
        </p>}
        {balance.kind === 'unsupported' && <p className="caption">{hints.unsupported(balance.title)}</p>}
        {balance.kind === 'failed' && <>
          <p className="caption" data-tone="bad">{hints.failed(balance.title)}</p>
          <Details said={sentenceOf(balance.said)} />
        </>}
        {balance.kind === 'known' && <>
          {balance.amounts.map(one => <p key={one.currency} className="balance-amount" data-test="balance-amount">
            {hints.amount(balance.title, money(one.total, one.currency), money(one.toppedUp, one.currency), money(one.granted, one.currency))}
          </p>)}
          {balance.amounts.length === 0 && <p className="caption">{hints.none(balance.title)}</p>}
          {!balance.available && <p className="caption" data-tone="bad" data-test="balance-cannot-pay">{hints.cannotPay}</p>}
        </>}
      </div>
      <button
        type="button"
        className="btn btn-quiet"
        disabled={balance.kind === 'unknown'}
        onClick={() => void ask()}
        data-test="balance-again"
      >
        <RefreshCw size={14} aria-hidden="true" />{words.label.balanceAgain}
      </button>
    </div>
  </div>;
}
