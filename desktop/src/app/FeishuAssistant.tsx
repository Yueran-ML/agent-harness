import { useEffect, useRef, useState } from 'react';
import { Check, Copy, ExternalLink, FileText, Play, RefreshCw, Settings2, Square, TriangleAlert } from 'lucide-react';
import { sentenceOf } from '../key/form';
import { useFeishu, type Feishu } from '../useFeishu';
import { doingOf, type App, type Ended } from './feishu';
import { Details } from './GitHubSignIn';
import { useWords } from './locale';

/** Open a page of the web in the browser of the system. */
type Open = (url: string) => void;

/** The code of pairing, large enough to be read off the screen and typed into a phone. */
function PairCard({ code, platform }: { code: string; platform: string }) {
  const words = useWords();
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [copied]);
  const copy = () => {
    void navigator.clipboard.writeText(code).then(() => setCopied(true)).catch(() => {
      // Without a clipboard the code can still be typed in by hand.
    });
  };
  return <div className="github-card feishu-card" data-test="feishu-pair">
    <div className="overline">{words.label.feishu.pair(platform)}</div>
    <div className="github-code-row">
      <output className="github-code mono" data-test="feishu-code">{code}</output>
      <button type="button" className="btn" onClick={copy} data-test="feishu-copy">
        {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        {copied ? words.label.copied : words.label.feishu.copyCode}
      </button>
    </div>
    <p className="caption">{words.hint.feishu.pairNote}</p>
    <p className="caption" data-test="feishu-not-heard">{words.hint.feishu.pairNotHeard}</p>
  </div>;
}

/** What went wrong, as the panel says it, with what the program said folded away beneath. */
function Trouble({ tone, words: said, details, test }: { tone: 'bad' | 'warn'; words: string; details: string; test: string }) {
  return <div className="callout" data-tone={tone} role="alert" data-test={test}>
    <TriangleAlert size={15} aria-hidden="true" />
    <div className="callout-words">
      <p>{said}</p>
      <Details said={details} />
    </div>
  </div>;
}

/** How the last assistant that the application ran ended badly: the line that it wrote last, as a sentence. */
function EndedNote({ ended }: { ended: Ended }) {
  const words = useWords();
  return <Trouble tone="bad" words={words.hint.feishu.ended} details={sentenceOf(ended.said)} test="feishu-ended" />;
}

/** How an assistant that runs is doing, and what went wrong lately where it is connected. */
function Doing({ app }: { app: App }) {
  const words = useWords();
  const doing = doingOf(app);
  const hints = words.hint.feishu;
  const platform = words.label.feishu.platforms[app.platform];
  // While it is connected, what went wrong is something that did not go through, such as an answer.
  const trouble = (doing === 'connected' || doing === 'pair') && app.error !== null
    ? <Trouble tone="warn" words={hints.trouble} details={sentenceOf(app.error)} test="feishu-trouble" />
    : null;
  if (doing === 'pair') return <><PairCard code={app.code ?? ''} platform={platform} />{trouble}</>;
  const waiting = doing === 'connecting' || doing === 'reconnecting' || doing === 'stopping';
  const said = doing === 'connecting' ? hints.connecting(platform)
    : doing === 'reconnecting' ? hints.reconnecting(platform)
      : doing === 'connected' ? hints.connected(platform) : hints[doing];
  return <>
    <p className="github-wait-words feishu-doing" role="status" data-doing={doing} data-test="feishu-doing">
      {waiting ? <span className="moon" aria-hidden="true" /> : doing === 'connected' ? <Check size={15} aria-hidden="true" /> : null}
      <span>{said}</span>
    </p>
    {doing === 'reconnecting' && <Details said={sentenceOf(app.error ?? '')} />}
    {trouble}
  </>;
}

/** The app that is set up, its assistant, and what can be done with them. */
function AppPart({ app, feishu, onOpen }: { app: App; feishu: Feishu; onOpen: Open }) {
  const words = useWords();
  const { busy } = feishu;
  const [asking, setAsking] = useState(false);
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (asking) keep.current?.focus(); }, [asking]);
  const labels = words.label.feishu;
  const forget = async () => {
    if (await feishu.forget()) setAsking(false);
  };
  return <>
    <div className="feishu-app" data-test="feishu-app">
      <span className="feishu-platform">{labels.platforms[app.platform]}</span>
      <span className="mono feishu-id">{app.appId}</span>
      <span className="feishu-light" data-on={app.running ? '' : undefined} aria-hidden="true" />
    </div>

    {app.running ? <Doing app={app} />
      : <p className="setting-now" data-test="feishu-not-running">{words.hint.feishu.notRunning}</p>}
    {!app.running && app.state === 'failed' && app.error !== null && <Trouble
      tone="bad"
      words={words.hint.feishu.lastError}
      details={sentenceOf(app.error)}
      test="feishu-last-error"
    />}
    {!app.running && app.ended !== null && app.state !== 'failed' && <EndedNote ended={app.ended} />}

    {/* An assistant that runs elsewhere goes on with the app that it read: the app is changed where it is stopped. */}
    {!(app.running && !app.own) && <div className="setting-row feishu-actions">
      {!app.running && <button
        type="button"
        className="btn btn-main"
        disabled={busy !== null}
        onClick={() => void feishu.start()}
        data-test="feishu-start"
      >
        <Play size={14} aria-hidden="true" />{busy === 'starting' ? labels.starting : labels.start}
      </button>}
      {app.running && app.own && <button
        type="button"
        className="btn"
        disabled={busy !== null || app.state === 'stopping'}
        onClick={() => void feishu.stop()}
        data-test="feishu-stop"
      >
        <Square size={13} aria-hidden="true" />{busy === 'stopping' || app.state === 'stopping' ? labels.stopping : labels.stop}
      </button>}
      <button type="button" className="btn" disabled={busy !== null} onClick={() => void feishu.setUp()} data-test="feishu-change">
        <Settings2 size={14} aria-hidden="true" />{labels.change}
      </button>
      {!asking && <button
        type="button"
        className="btn btn-quiet"
        disabled={busy !== null}
        onClick={() => setAsking(true)}
        data-test="feishu-forget"
      >
        {labels.forget}
      </button>}
    </div>}
    {asking && <div className="callout" data-tone="warn" role="alert" data-test="feishu-forget-ask">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{words.hint.feishu.forgetAsk(app.appId)}</p>
        <div className="callout-actions">
          <button type="button" ref={keep} className="btn" disabled={busy !== null} onClick={() => setAsking(false)} data-test="feishu-keep">
            {labels.keep}
          </button>
          <button type="button" className="btn btn-bad" disabled={busy !== null} onClick={() => void forget()} data-test="feishu-forget-sure">
            {labels.forgetConfirm}
          </button>
        </div>
      </div>
    </div>}
    <div className="setting-row feishu-links">
      <button type="button" className="btn btn-quiet" onClick={() => onOpen(app.console)} data-test="feishu-console">
        <ExternalLink size={14} aria-hidden="true" />{labels.console}
      </button>
      <button type="button" className="btn btn-quiet" onClick={feishu.showLog} data-test="feishu-log">
        <FileText size={14} aria-hidden="true" />{labels.log}
      </button>
    </div>
    {(!app.running || app.own) && <p className="caption">{words.hint.feishu.withColumbina}</p>}
  </>;
}

/**
 * The assistant in Feishu, in the settings: the app that is set up, how its assistant is doing
 * and the code to pair with it, and the ways to start it, stop it, change the app or delete it.
 * The App Secret is given in a window of its own, and never comes here.
 */
export function FeishuAssistant({ language, onOpen }: { language: string; onOpen: Open }) {
  const words = useWords();
  const feishu = useFeishu(language);
  const { assistant, busy, failure } = feishu;
  const hints = words.hint.feishu;
  const failed = failure === null ? null : <Trouble
    tone="bad"
    words={failure.what === 'start' ? hints.startFailed : failure.what === 'stop' ? hints.stopFailed
      : failure.what === 'forget' ? hints.forgetFailed : words.hint.requestFailed}
    details={sentenceOf(failure.said)}
    test="feishu-failed"
  />;

  if (assistant.kind === 'unknown') {
    return <p className="github-wait-words caption" role="status"><span className="moon" aria-hidden="true" />{hints.reading}</p>;
  }
  if (assistant.kind === 'unavailable') return <p className="caption">{hints.unavailable}</p>;
  if (assistant.kind === 'failed') {
    return <div className="callout" data-tone="warn" data-test="feishu-status-failed">
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{hints.statusFailed}</p>
        <Details said={sentenceOf(assistant.said)} />
        <button type="button" className="btn" onClick={() => void feishu.refresh()} data-test="feishu-check-again">
          <RefreshCw size={14} aria-hidden="true" />{words.label.feishu.checkAgain}
        </button>
      </div>
    </div>;
  }
  if (assistant.kind === 'none') {
    return <>
      {feishu.forgotten && <div className="callout" data-tone="ok" role="status" data-test="feishu-forgotten">
        <Check size={15} aria-hidden="true" />
        <div className="callout-words"><p>{hints.forgotten}</p></div>
      </div>}
      {assistant.ended !== null && <EndedNote ended={assistant.ended} />}
      <p className="caption">{hints.lead}</p>
      <div className="setting-row">
        <button type="button" className="btn btn-main" disabled={busy !== null} onClick={() => void feishu.setUp()} data-test="feishu-set-up">
          <Settings2 size={14} aria-hidden="true" />{words.label.feishu.setUp}
        </button>
      </div>
      {failed}
    </>;
  }
  return <>
    <AppPart app={assistant} feishu={feishu} onOpen={onOpen} />
    {failed}
  </>;
}
