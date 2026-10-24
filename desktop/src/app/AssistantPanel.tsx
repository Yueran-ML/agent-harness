import { useEffect, useRef, useState } from 'react';
import { Check, Copy, ExternalLink, FileText, FolderOpen, Play, RefreshCw, Settings2, Square, TriangleAlert } from 'lucide-react';
import { sentenceOf } from '../key/form';
import { useAssistant, type AssistantHandle } from '../useAssistant';
import { doingOf, type App, type Ended, type Kind } from './assistant';
import { Details } from './GitHubSignIn';
import { useWords } from './locale';

/** Open a page of the web in the browser of the system. */
type Open = (url: string) => void;

/** The code of pairing, large enough to be read off the screen and typed into a phone. */
function PairCard({ kind, code, platform }: { kind: Kind; code: string; platform: string }) {
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
  return <div className="github-card" data-test={`${kind}-pair`}>
    <div className="overline">{words.label[kind].pair(platform)}</div>
    <div className="github-code-row">
      <output className="github-code mono" data-test={`${kind}-code`}>{code}</output>
      <button type="button" className="btn" onClick={copy} data-test={`${kind}-copy`}>
        {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        {copied ? words.label.copied : words.label[kind].copyCode}
      </button>
    </div>
    <p className="caption">{words.hint[kind].pairNote}</p>
    <p className="caption" data-test={`${kind}-not-heard`}>{words.hint[kind].pairNotHeard}</p>
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
function EndedNote({ kind, ended }: { kind: Kind; ended: Ended }) {
  const words = useWords();
  return <Trouble tone="bad" words={words.hint[kind].ended} details={sentenceOf(ended.said)} test={`${kind}-ended`} />;
}

/** How an assistant that runs is doing, and what went wrong lately where it is connected. */
function Doing({ kind, app }: { kind: Kind; app: App }) {
  const words = useWords();
  const doing = doingOf(app);
  const hints = words.hint[kind];
  const platform = words.label.platforms[app.platform];
  // While it is connected, what went wrong is something that did not go through, such as an answer.
  const trouble = (doing === 'connected' || doing === 'pair') && app.error !== null
    ? <Trouble tone="warn" words={hints.trouble} details={sentenceOf(app.error)} test={`${kind}-trouble`} />
    : null;
  if (doing === 'pair') return <><PairCard kind={kind} code={app.code ?? ''} platform={platform} />{trouble}</>;
  const waiting = doing === 'connecting' || doing === 'reconnecting' || doing === 'stopping';
  const said = doing === 'connecting' ? hints.connecting(platform)
    : doing === 'reconnecting' ? hints.reconnecting(platform)
      : doing === 'connected' ? hints.connected(platform) : hints[doing];
  return <>
    <p className="github-wait-words assistant-doing" role="status" data-doing={doing} data-test={`${kind}-doing`}>
      {waiting ? <span className="moon" aria-hidden="true" /> : doing === 'connected' ? <Check size={15} aria-hidden="true" /> : null}
      <span>{said}</span>
    </p>
    {doing === 'reconnecting' && <Details said={sentenceOf(app.error ?? '')} />}
    {trouble}
  </>;
}

/** The app or the bot that is set up, its assistant, and what can be done with them. */
function AppPart({ kind, app, assistant, onOpen }: { kind: Kind; app: App; assistant: AssistantHandle; onOpen: Open }) {
  const words = useWords();
  const { busy } = assistant;
  const [asking, setAsking] = useState(false);
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (asking) keep.current?.focus(); }, [asking]);
  const labels = words.label[kind];
  const hints = words.hint[kind];
  const forget = async () => {
    if (await assistant.forget()) setAsking(false);
  };
  return <>
    <div className="assistant-app" data-test={`${kind}-app`}>
      <span className="assistant-platform">{words.label.platforms[app.platform]}</span>
      <span className="mono assistant-id">{app.id}</span>
      <span className="assistant-light" data-on={app.running ? '' : undefined} aria-hidden="true" />
    </div>

    {app.running ? <Doing kind={kind} app={app} />
      : <p className="setting-now" data-test={`${kind}-not-running`}>{hints.notRunning}</p>}
    {!app.running && app.state === 'failed' && app.error !== null && <Trouble
      tone="bad"
      words={hints.lastError}
      details={sentenceOf(app.error)}
      test={`${kind}-last-error`}
    />}
    {!app.running && app.ended !== null && app.state !== 'failed' && <EndedNote kind={kind} ended={app.ended} />}

    {/* An assistant that runs elsewhere goes on with what it read: what is set up is changed where it is stopped. */}
    {!(app.running && !app.own) && <div className="setting-row assistant-actions">
      {!app.running && <button
        type="button"
        className="btn btn-main"
        disabled={busy !== null}
        onClick={() => void assistant.start()}
        data-test={`${kind}-start`}
      >
        <Play size={14} aria-hidden="true" />{busy === 'starting' ? labels.starting : labels.start}
      </button>}
      {app.running && app.own && <button
        type="button"
        className="btn"
        disabled={busy !== null || app.state === 'stopping'}
        onClick={() => void assistant.stop()}
        data-test={`${kind}-stop`}
      >
        <Square size={13} aria-hidden="true" />{busy === 'stopping' || app.state === 'stopping' ? labels.stopping : labels.stop}
      </button>}
      <button type="button" className="btn" disabled={busy !== null} onClick={() => void assistant.setUp()} data-test={`${kind}-change`}>
        <Settings2 size={14} aria-hidden="true" />{labels.change}
      </button>
      {!asking && <button
        type="button"
        className="btn btn-quiet"
        disabled={busy !== null}
        onClick={() => setAsking(true)}
        data-test={`${kind}-forget`}
      >
        {labels.forget}
      </button>}
    </div>}
    {asking && <div className="callout" data-tone="warn" role="alert" data-test={`${kind}-forget-ask`}>
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{hints.forgetAsk(app.id)}</p>
        <div className="callout-actions">
          <button type="button" ref={keep} className="btn" disabled={busy !== null} onClick={() => setAsking(false)} data-test={`${kind}-keep`}>
            {labels.keep}
          </button>
          <button type="button" className="btn btn-bad" disabled={busy !== null} onClick={() => void forget()} data-test={`${kind}-forget-sure`}>
            {labels.forgetConfirm}
          </button>
        </div>
      </div>
    </div>}
    {/* The program changes it only while no assistant runs: one that runs elsewhere is stopped there first.
        The agents of the assistant in QQ change nothing at all. */}
    {kind !== 'qq' && <>
      <div className="setting-row">
        <label className="check">
          <input
            type="checkbox"
            checked={app.writable}
            disabled={busy !== null || (app.running && !app.own)}
            onChange={event => void assistant.setWritable(event.target.checked)}
            data-test={`${kind}-writable`}
          />
          {words.label[kind].writable}
        </label>
      </div>
      <p className="caption">{words.hint[kind].writable}</p>
    </>}
    <div className="setting-row assistant-links">
      <button type="button" className="btn btn-quiet" onClick={() => onOpen(app.console)} data-test={`${kind}-console`}>
        <ExternalLink size={14} aria-hidden="true" />{labels.console}
      </button>
      {app.folder !== null && <button
        type="button"
        className="btn btn-quiet"
        title={app.folder}
        onClick={() => void assistant.openFolder()}
        data-test={`${kind}-folder`}
      >
        <FolderOpen size={14} aria-hidden="true" />{labels.folder}
      </button>}
      <button type="button" className="btn btn-quiet" onClick={assistant.showLog} data-test={`${kind}-log`}>
        <FileText size={14} aria-hidden="true" />{labels.log}
      </button>
    </div>
    {(!app.running || app.own) && <p className="caption">{hints.withColumbina}</p>}
    {/* It reads the service when it starts, and does not follow another that is chosen while it runs. */}
    {app.running && <p className="caption" data-test={`${kind}-service`}>{hints.service}</p>}
  </>;
}

/**
 * An assistant in Feishu, WeCom or QQ, in the settings: the app or the bot that is set up, how its
 * assistant is doing and the code to pair with it, and the ways to start it, stop it, change what
 * is set up or delete it. The secret is given in a window of its own, and never comes here.
 */
export function AssistantPanel({ kind, language, onOpen }: { kind: Kind; language: string; onOpen: Open }) {
  const words = useWords();
  const handle = useAssistant(kind, language);
  const { assistant, busy, failure } = handle;
  const hints = words.hint[kind];
  const failed = failure === null ? null : <Trouble
    tone="bad"
    words={failure.what === 'start' ? hints.startFailed : failure.what === 'stop' ? hints.stopFailed
      : failure.what === 'forget' ? hints.forgetFailed : failure.what === 'writable' && kind !== 'qq' ? words.hint[kind].writableFailed
        : failure.what === 'folder' ? hints.folderFailed : words.hint.requestFailed}
    details={sentenceOf(failure.said)}
    test={`${kind}-failed`}
  />;

  if (assistant.kind === 'unknown') {
    return <p className="github-wait-words caption" role="status"><span className="moon" aria-hidden="true" />{hints.reading}</p>;
  }
  if (assistant.kind === 'unavailable') return <p className="caption">{hints.unavailable}</p>;
  if (assistant.kind === 'failed') {
    return <div className="callout" data-tone="warn" data-test={`${kind}-status-failed`}>
      <TriangleAlert size={15} aria-hidden="true" />
      <div className="callout-words">
        <p>{hints.statusFailed}</p>
        <Details said={sentenceOf(assistant.said)} />
        <button type="button" className="btn" onClick={() => void handle.refresh()} data-test={`${kind}-check-again`}>
          <RefreshCw size={14} aria-hidden="true" />{words.label[kind].checkAgain}
        </button>
      </div>
    </div>;
  }
  if (assistant.kind === 'none') {
    return <>
      {handle.forgotten && <div className="callout" data-tone="ok" role="status" data-test={`${kind}-forgotten`}>
        <Check size={15} aria-hidden="true" />
        <div className="callout-words"><p>{hints.forgotten}</p></div>
      </div>}
      {assistant.ended !== null && <EndedNote kind={kind} ended={assistant.ended} />}
      <p className="caption">{hints.lead}</p>
      <div className="setting-row">
        <button type="button" className="btn btn-main" disabled={busy !== null} onClick={() => void handle.setUp()} data-test={`${kind}-set-up`}>
          <Settings2 size={14} aria-hidden="true" />{words.label[kind].setUp}
        </button>
      </div>
      {failed}
    </>;
  }
  return <>
    <AppPart kind={kind} app={assistant} assistant={handle} onOpen={onOpen} />
    {failed}
  </>;
}
