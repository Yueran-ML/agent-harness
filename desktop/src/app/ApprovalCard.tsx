import { useEffect, useRef, useState } from 'react';
import { ChevronRight, ShieldQuestion } from 'lucide-react';
import type { PendingApproval } from '../useWorkspace';
import { useWords } from './locale';
import { commandTools, scopeOf } from './tell';

/** For this long after a question appears, the keyboard cannot say yes to it. */
const SETTLE_MS = 500;

const writers = new Set(['write', 'edit']);

/**
 * A question of the agent: may it do this.
 *
 * It stands above the field, and the conversation stays where it can be read.
 * It does not take the keyboard away from whoever is writing. Ctrl+Enter says
 * yes for this once, and Enter alone says yes to nothing.
 */
export function ApprovalCard({ approval, cancelling, onAnswer, onStop }: {
  approval: PendingApproval;
  cancelling: boolean;
  onAnswer: (outcome: 'allowed-once' | 'rejected', remember?: boolean) => void;
  onStop: () => void;
}) {
  const words = useWords();
  const [answered, setAnswered] = useState(false);
  const shownAt = useRef(0);
  useEffect(() => {
    shownAt.current = Date.now();
    setAnswered(false);
  }, [approval.requestId]);

  const answer = (outcome: 'allowed-once' | 'rejected', remember = false) => {
    if (answered) return;
    setAnswered(true);
    onAnswer(outcome, remember);
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Enter' || !(event.ctrlKey || event.metaKey) || event.shiftKey || event.altKey) return;
      if (event.isComposing || event.repeat || document.querySelector('dialog[open]')) return;
      // A key that was on its way when the question appeared was not meant for it.
      if (Date.now() - shownAt.current < SETTLE_MS) return;
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
      <button type="button" className="btn" disabled={answered} onClick={() => answer('rejected')} data-test="consent-reject">
        {words.label.reject}
      </button>
      {scope !== null && <button
        type="button"
        className="btn"
        disabled={answered}
        aria-describedby="consent-scope"
        onClick={() => answer('allowed-once', true)}
        data-test="consent-always"
      >
        {words.label.allowAlways}
      </button>}
      <button type="button" className="btn btn-main" disabled={answered} onClick={() => answer('allowed-once')} data-test="consent-once">
        {words.label.allowOnce}
        <kbd>Ctrl</kbd><kbd>Enter</kbd>
      </button>
    </div>
    {scope !== null && <p className="caption consent-scope" id="consent-scope">{scope}</p>}
  </section>;
}
