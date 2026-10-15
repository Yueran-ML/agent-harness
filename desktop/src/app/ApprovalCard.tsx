import { useEffect, useRef, useState } from 'react';
import { ChevronRight, ShieldQuestion } from 'lucide-react';
import type { PendingApproval } from '../useWorkspace';
import { isYes, keyOf, settled, SETTLE_MS } from './keys';
import { useWords } from './locale';
import { modifier } from './system';
import { commandTools, scopeOf } from './tell';

const writers = new Set(['write', 'edit']);

/**
 * A question of the agent: may it do this.
 *
 * It stands above the field, and the conversation stays where it can be read.
 * It does not take the keyboard away from whoever is writing. Ctrl+Enter, or
 * ⌘Enter, says yes for this once, and Enter alone says yes to nothing.
 *
 * Every question has a card of its own: the caller gives each a key. So what
 * a card remembers, such as that it was answered, is never taken for the
 * next question. In its first moment nothing answers a card, neither a key
 * nor a click, because either may have been on its way to something else.
 */
export function ApprovalCard({ approval, cancelling, onAnswer, onStop }: {
  approval: PendingApproval;
  cancelling: boolean;
  onAnswer: (outcome: 'allowed-once' | 'rejected', remember?: boolean) => void;
  onStop: () => void;
}) {
  const words = useWords();
  const [answered, setAnswered] = useState(false);
  const shownAt = useRef(Date.now());
  // The buttons can be seen to wait out the first moment, so that a click in it is not lost without a sign.
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setReady(true), SETTLE_MS);
    return () => clearTimeout(timer);
  }, []);

  const answer = (outcome: 'allowed-once' | 'rejected', remember = false) => {
    if (answered || !settled(shownAt.current, Date.now())) return;
    setAnswered(true);
    onAnswer(outcome, remember);
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!isYes(keyOf(event)) || document.querySelector('dialog[open]')) return;
      event.preventDefault();
      answer('allowed-once');
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const scope = scopeOf(words, approval);
  const sentence = commandTools.has(approval.tool) ? words.hint.consentRun
    : writers.has(approval.tool) ? words.hint.consentWrite : words.hint.consentOther(approval.tool);
  const text = approval.target ?? approval.summary;

  return <section className="consent" data-test="consent" aria-labelledby="consent-title">
    <header className="consent-head">
      <ShieldQuestion size={17} aria-hidden="true" />
      <h2 id="consent-title">{words.label.consentTitle}</h2>
      <span className="caption" role="status">{answered ? words.hint.consentSent : words.hint.consentWaiting}</span>
    </header>
    {/* Said once to whoever cannot see the card appear. */}
    <p className="sr-only" role="alert"><span>{words.label.consentTitle}</span> <span>{sentence}</span></p>

    <p className="consent-what">{sentence}</p>
    <pre className="block consent-target" tabIndex={0} data-test="consent-target">{text}</pre>
    {approval.reason && <details className="aside">
      <summary><ChevronRight size={13} aria-hidden="true" />{words.label.whyItAsks}</summary>
      <p className="consent-why">{approval.reason}</p>
    </details>}

    <div className="consent-actions">
      <button type="button" className="btn btn-quiet" disabled={cancelling} onClick={onStop} data-test="consent-stop">
        {cancelling ? words.label.stopping : words.label.stopTurn}
      </button>
      <span className="consent-gap" />
      <button type="button" className="btn" disabled={answered || !ready} onClick={() => answer('rejected')} data-test="consent-reject">
        {words.label.reject}
      </button>
      {scope !== null && <button
        type="button"
        className="btn"
        disabled={answered || !ready}
        aria-describedby="consent-scope"
        onClick={() => answer('allowed-once', true)}
        data-test="consent-always"
      >
        {words.label.allowAlways}
      </button>}
      <button type="button" className="btn btn-main" disabled={answered || !ready} onClick={() => answer('allowed-once')} data-test="consent-once">
        {words.label.allowOnce}
        <kbd>{modifier()}</kbd><kbd>Enter</kbd>
      </button>
    </div>
    {scope !== null && <p className="caption consent-scope" id="consent-scope">{scope}</p>}
  </section>;
}
