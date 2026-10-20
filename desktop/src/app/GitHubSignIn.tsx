import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Check, ChevronRight, Copy, ExternalLink, LogIn, RefreshCw, TriangleAlert } from 'lucide-react';
import { useRefreshOnFocus, type GitHub } from '../useGitHub';
import {
  clockOf, failureOf, initialOf, repositoriesToShow, secondsLeft, type Person, type SignIn,
} from './github';
import { useWords } from './locale';

/** Open a page of GitHub in the browser of the system. */
type Open = (url: string) => void;

/** What a program said, where a person may want to see it: under the words of the page, folded away. */
export function Details({ said }: { said: string }) {
  const words = useWords();
  if (said === '') return null;
  return <details className="aside">
    <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
    <p className="tool-code mono">{said}</p>
  </details>;
}

/**
 * Give the keyboard to what is done next, where the button that had it is gone: unless the user
 * has gone on to something else meanwhile.
 */
function takeFocus(element: HTMLElement | null) {
  const here = document.activeElement;
  const free = here === null || here === document.body || here instanceof HTMLDialogElement
    || here.closest('[data-github]') !== null;
  if (element !== null && free) element.focus();
}

/** A clock that goes on while a code holds, and stands still once it has run out. */
function useSecondsLeft(expiresAt: number): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => {
      const at = Date.now();
      setNow(at);
      if (at >= expiresAt) clearInterval(timer);
    }, 1000);
    return () => clearInterval(timer);
  }, [expiresAt]);
  return secondsLeft(expiresAt, now);
}

/** The code to enter on GitHub, the way to the page, and the time that the code holds yet. */
function CodeCard({ state, github, onOpen }: { state: Extract<SignIn, { kind: 'code' }>; github: GitHub; onOpen: Open }) {
  const words = useWords();
  const left = useSecondsLeft(state.expiresAt);
  const [copied, setCopied] = useState(false);
  const copyButton = useRef<HTMLButtonElement>(null);
  const newCode = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [copied]);
  // The keyboard goes on with the code: to its copy when it comes, and to a new one when it has run out.
  const expired = left === 0;
  useEffect(() => {
    takeFocus((expired ? newCode : copyButton).current);
  }, [expired]);
  const copy = () => {
    void navigator.clipboard.writeText(state.code).then(() => setCopied(true)).catch(() => {
      // Without a clipboard the code can still be marked and typed in by hand.
    });
  };
  return <div className="github-card" data-expired={expired ? '' : undefined} data-test="github-code">
    <div className="overline">{words.label.github.enterCode}</div>
    <div className="github-code-row">
      <output className="github-code mono" data-test="github-code-value">{state.code}</output>
      {!expired && <button type="button" ref={copyButton} className="btn" onClick={copy} data-test="github-copy">
        {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        {copied ? words.label.copied : words.label.github.copyCode}
      </button>}
    </div>
    {!expired && <div className="github-open">
      <button type="button" className="btn btn-main" onClick={() => onOpen(state.url)} data-test="github-open">
        <ExternalLink size={14} aria-hidden="true" />{words.label.github.openGitHub}
      </button>
      <span className="caption mono github-url" data-test="github-url">{state.url}</span>
    </div>}
    <div className="github-wait">
      {expired
        ? <p className="github-wait-words" role="status" data-test="github-expired">{words.hint.github.expired}</p>
        : <p className="github-wait-words">
            <span className="moon" aria-hidden="true" />
            <span className="github-wait-text">{words.hint.github.waiting}</span>
            <span className="github-left mono" data-test="github-left">{words.label.github.left(clockOf(left))}</span>
          </p>}
      {expired && <button type="button" ref={newCode} className="btn" onClick={() => void github.begin()} data-test="github-new-code">
        {words.label.github.newCode}
      </button>}
      <button type="button" className="btn btn-quiet" onClick={github.cancel} data-test="github-cancel">{words.label.cancel}</button>
    </div>
  </div>;
}

/**
 * The way to sign in to GitHub: a button, then the code to enter on GitHub while the program
 * waits for the user to confirm it there, then whatever went wrong. Once somebody is signed in,
 * the account is shown in its place.
 *
 * Args:
 *   lead: Whether it says first what signing in allows.
 *   main: Whether the button that signs in is the main one of its place. Where signing in is
 *     one choice among others, it is not.
 *   aside: What stands beside the button that signs in, such as a way to leave it for later.
 */
export function GitHubSignIn({ github, onOpen, lead = true, main = true, aside }: {
  github: GitHub;
  onOpen: Open;
  lead?: boolean;
  main?: boolean;
  aside?: ReactNode;
}) {
  const words = useWords();
  const { signIn } = github;
  const cancel = useRef<HTMLButtonElement>(null);
  const again = useRef<HTMLButtonElement>(null);
  // The button that was pressed is gone by the time there is something to do next. The keyboard goes there.
  useEffect(() => {
    if (signIn.kind === 'asking') takeFocus(cancel.current);
    else if (signIn.kind === 'failed') takeFocus(again.current);
  }, [signIn.kind]);
  const failure = signIn.kind === 'failed' ? failureOf(signIn.code) : null;
  return <div className="github-flow" data-github="">
    {signIn.kind === 'code' && <CodeCard key={signIn.code} state={signIn} github={github} onOpen={onOpen} />}
    {signIn.kind === 'asking' && <div className="github-wait" data-test="github-asking">
      <p className="github-wait-words" role="status"><span className="moon" aria-hidden="true" />{words.hint.github.asking}</p>
      <button type="button" ref={cancel} className="btn btn-quiet" onClick={github.cancel}>{words.label.cancel}</button>
    </div>}
    {signIn.kind === 'failed' && failure !== null && <div className="callout" data-tone="bad" role="alert" data-test="github-failed">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{words.hint.github.failed[failure]}</p>
        <Details said={signIn.said} />
      </div>
    </div>}
    {lead && (signIn.kind === 'idle' || signIn.kind === 'done') && <p className="caption github-lead">{words.hint.github.lead}</p>}
    {signIn.kind !== 'code' && signIn.kind !== 'asking' && <div className="setting-row">
      {/* A build that cannot sign in cannot be helped by trying again. */}
      {failure !== 'unregistered' && <button
        type="button"
        ref={again}
        className={failure === null && main ? 'btn btn-main' : 'btn'}
        onClick={() => void github.begin()}
        data-test="github-sign-in"
      >
        <LogIn size={14} aria-hidden="true" />
        {failure === null ? words.label.github.signIn : failure === 'expired' ? words.label.github.newCode : words.label.github.tryAgain}
      </button>}
      {aside}
    </div>}
  </div>;
}

/** The picture of who signed in, or the first letter of the name where there is no picture. */
export function Avatar({ person, size = 40 }: { person: Person; size?: number }) {
  return person.avatar
    ? <img className="avatar" src={person.avatar} alt="" width={size} height={size} />
    : <span className="avatar avatar-letter" style={{ width: size, height: size }} aria-hidden="true">{initialOf(person)}</span>;
}

/**
 * Who is signed in to GitHub, and what Columbina may reach in that name: in the settings. Where
 * nobody is, the way to sign in stands here.
 */
export function GitHubAccount({ github, onOpen }: { github: GitHub; onOpen: Open }) {
  const words = useWords();
  const { account, signIn } = github;
  const [asking, setAsking] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const stay = useRef<HTMLButtonElement>(null);
  // Repositories are chosen on GitHub. When the user comes back from there, what was chosen is asked for.
  const waitsForGitHub = account.kind === 'signedIn' && (account.repositories === null || account.repositories.length === 0);
  useRefreshOnFocus(github.refresh, waitsForGitHub);
  useEffect(() => { if (asking) stay.current?.focus(); }, [asking]);

  if (account.kind === 'unknown') {
    return github.checking
      ? <p className="github-wait-words caption" role="status"><span className="moon" aria-hidden="true" />{words.hint.github.checking}</p>
      : null;
  }
  if (account.kind === 'unavailable') return <p className="caption">{words.hint.github.unavailable}</p>;
  if (account.kind === 'unregistered') {
    return <div className="github-note">
      <p className="caption">{words.hint.github.failed.unregistered}</p>
      <Details said={account.why} />
    </div>;
  }
  if (account.kind === 'failed') {
    return <div className="callout" data-tone="warn" data-test="github-status-failed">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{account.code === 'STORE' ? words.hint.github.unreadable : words.hint.github.statusFailed}</p>
        <Details said={account.said} />
        <button type="button" className="btn" disabled={github.checking} onClick={() => void github.refresh()} data-test="github-check-again">
          <RefreshCw size={14} aria-hidden="true" />{words.label.github.checkAgain}
        </button>
      </div>
    </div>;
  }
  if (account.kind === 'nobody') {
    return <>
      {github.signedOut !== null && <div className="callout" data-tone="ok" role="status" data-test="github-signed-out">
        <Check size={15} aria-hidden="true" />
        <div className="callout-words">
          <p>{words.hint.github.signedOut}</p>
          {github.signedOut.revoke !== null && <button
            type="button"
            className="link"
            onClick={() => onOpen(github.signedOut!.revoke!)}
            data-test="github-revoke"
          >
            {words.label.github.revoke}
          </button>}
        </div>
      </div>}
      {account.ended !== null && signIn.kind === 'idle' && <p className="caption" data-tone="warn" data-test="github-ended">
        {words.hint.github.ended}
      </p>}
      <GitHubSignIn github={github} onOpen={onOpen} />
    </>;
  }

  const { person, repositories, installUrl } = account;
  const leave = async () => {
    setLeaving(true);
    try {
      await github.signOut();
    } finally {
      setLeaving(false);
      setAsking(false);
    }
  };
  const { shown, more } = repositoriesToShow(repositories ?? []);
  return <>
    {signIn.kind === 'done' && <p className="sr-only" role="status">{words.hint.github.signedIn(person.login)}</p>}
    <div className="github-person" data-test="github-person">
      <Avatar person={person} />
      <div className="github-names">
        {person.name && <span className="github-name">{person.name}</span>}
        <span className="github-login mono">@{person.login}</span>
      </div>
      {!asking && <button type="button" className="btn btn-quiet" onClick={() => setAsking(true)} data-test="github-sign-out">
        {words.label.github.signOut}
      </button>}
    </div>
    {asking && <div className="callout" data-tone="warn" role="alert" data-test="github-sign-out-ask">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{words.hint.github.signOutAsk(person.login)}</p>
        <div className="callout-actions">
          <button type="button" ref={stay} className="btn" disabled={leaving} onClick={() => setAsking(false)} data-test="github-stay">
            {words.label.github.stay}
          </button>
          <button type="button" className="btn btn-bad" disabled={leaving} onClick={() => void leave()} data-test="github-sign-out-sure">
            {words.label.github.signOut}
          </button>
        </div>
      </div>
    </div>}
    {github.signOutFailed !== null && <div className="callout" data-tone="bad" role="alert" data-test="github-sign-out-failed">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{words.hint.github.signOutFailed}</p>
        <Details said={github.signOutFailed.said} />
      </div>
    </div>}

    {repositories === null && <div className="github-note" data-test="github-offline">
      <p className="caption">{words.hint.github.offline}</p>
      <Details said={account.offline ?? ''} />
    </div>}
    {repositories !== null && repositories.length === 0 && <p className="caption" data-test="github-no-repositories">
      {words.hint.github.noRepositories}
    </p>}
    {shown.length > 0 && <>
      <div className="overline services-title">{words.label.github.repositories}</div>
      <ul className="services github-repositories" data-test="github-repositories">
        {shown.map(name => <li key={name}><span className="service-name mono">{name}</span></li>)}
      </ul>
      {more > 0 && <p className="caption">{words.hint.github.moreRepositories(more)}</p>}
    </>}
    {(installUrl !== '' || repositories === null) && <div className="setting-row">
      {installUrl !== '' && <button
        type="button"
        className={repositories !== null && repositories.length === 0 ? 'btn btn-main' : 'btn'}
        onClick={() => onOpen(installUrl)}
        data-test="github-repositories-choose"
      >
        <ExternalLink size={14} aria-hidden="true" />
        {repositories !== null && repositories.length === 0 ? words.label.github.chooseRepositories : words.label.github.manageRepositories}
      </button>}
      {repositories === null && <button type="button" className="btn btn-quiet" disabled={github.checking} onClick={() => void github.refresh()}>
        <RefreshCw size={14} aria-hidden="true" />{words.label.github.checkAgain}
      </button>}
    </div>}
  </>;
}
