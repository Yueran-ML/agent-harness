import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react';
import { FolderOpen, PanelRightClose, PanelRightOpen, Scissors, X } from 'lucide-react';
import { useWords } from '../app/locale';
import { shortcut } from '../app/system';
import { sentenceOf } from '../key/form';
import type { StudioEngine } from './connection';
import { callEngine, type PageMethod, type TransportState } from './engine';
import { Intent, useStudioKeys, type TransportCommand } from './keys';
import { MediaPool } from './MediaPool';
import { fileName, framesOf, nominalFps, rateOf } from './timecode';
import { Transport } from './Transport';
import { Viewer, type ViewerNotice } from './Viewer';

/** The pages of the studio, at its foot as in DaVinci Resolve. Those of the later phases come with them. */
export type StudioPage = 'media' | 'edit';
const pages: readonly StudioPage[] = ['media', 'edit'];

/** The timelines of a project that is not there yet: the same list each time, so that the pool is not drawn again for it. */
const NO_TIMELINES: readonly never[] = [];

/** What failed, in the words of whatever failed, without what Electron says of the call that carried it. */
export function failureOf(problem: unknown): string {
  const said = problem instanceof Error ? problem.message : String(problem);
  return said.replace(/^Error invoking remote method '[^']*': (?:[A-Za-z]*Error: )?/, '').trim();
}

/** What the shell says where it has no engine program to start: the engine is not installed. */
const NO_ENGINE = /\bno engine\b/i;

type Opening = { state: 'opening' } | { state: 'open'; session: number } | { state: 'failed'; message: string };

/** Something that did not work: what the page says of it, and what whatever failed said, in its own words. */
interface Problem {
  said: string;
  detail: string;
}

/**
 * The studio: the media of the project and a source monitor that plays one of them. It opens the
 * project of the folder in the engine as it is shown (the engine is kept while the app is on the
 * folder), and hides the engine's view as it goes.
 *
 * Args:
 *   cwd: The folder of the project, which the app has opened.
 *   engine: The engine of the studio, as `studioEngine` gives it.
 *   language: The language of the page, in which the dialog of the system is titled.
 *   conversationHidden: Whether the conversation beside the studio is put away.
 *   onToggleConversation: Put the conversation away, or bring it back.
 *   onClose: Leave the studio.
 */
export function Studio({ cwd, engine, language, conversationHidden = false, onToggleConversation, onClose }: {
  cwd: string;
  engine: StudioEngine;
  language: string;
  conversationHidden?: boolean;
  onToggleConversation?: () => void;
  onClose: () => void;
}) {
  const words = useWords();
  const { bridge, copy, native } = engine;
  const snapshot = useSyncExternalStore(copy.subscribe, copy.getSnapshot, copy.getSnapshot);
  const [page, setPage] = useState<StudioPage>('media');
  const [attempt, setAttempt] = useState(0);
  const [opening, setOpening] = useState<Opening>({ state: 'opening' });
  // The session of an engine that ended, which is said in the viewer until one is opened again.
  const [ended, setEnded] = useState<number | null>(null);
  // What the transport said when it was asked, before it told anything: after the page was loaded again.
  const [asked, setAsked] = useState<{ session: number; state: TransportState } | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  // What the engine said last that the user put away: it is not shown again until it says something else.
  const [putAway, setPutAway] = useState<string | null>(null);
  const root = useRef<HTMLElement>(null);
  const tabs = useRef<HTMLElement>(null);

  // The keyboard comes to the studio as it opens, for its keys.
  useEffect(() => { root.current?.focus({ preventScroll: true }); }, []);

  useEffect(() => {
    let gone = false;
    setOpening({ state: 'opening' });
    bridge.open(cwd).then((opened) => {
      if (gone) return;
      setOpening({ state: 'open', session: opened.session });
      callEngine(bridge, 'transport.state').then((state) => {
        if (!gone) setAsked({ session: opened.session, state });
      }, () => {});
    }, (failure: unknown) => {
      if (!gone) setOpening({ state: 'failed', message: failureOf(failure) });
    });
    return () => { gone = true; };
  }, [bridge, cwd, attempt]);

  useEffect(() => bridge.onEvent((event) => {
    if (event.kind === 'exited') setEnded(event.session);
    else if (event.kind === 'opened') setEnded(null);
  }), [bridge]);

  const project = snapshot.project;
  const timelines = project?.timelines ?? NO_TIMELINES;
  const transport = snapshot.transport ?? (asked !== null && asked.session === snapshot.session ? asked.state : null);
  const loaded = project?.media.find(media => media.id === transport?.media) ?? null;
  const rate = rateOf(loaded, timelines);
  const last = transport?.media == null ? null
    : transport.last ?? (loaded ? Math.max(0, framesOf(loaded.facts.duration, rate) - 1) : null);

  const fail = (said: string) => (failure: unknown) => setProblem({ said, detail: sentenceOf(failureOf(failure)) });
  const ask = (method: PageMethod, params: Record<string, unknown> | undefined, said: string) => {
    void callEngine(bridge, method, params).catch(fail(said));
  };

  // What the transport does as the keys know it.
  const intent = useRef(new Intent());
  useEffect(() => { if (transport) intent.current.told(transport); }, [transport]);
  const run = (command: TransportCommand) => {
    intent.current.sent(command);
    ask(command.method, command.params, words.hint.studio.failed);
  };
  useStudioKeys(
    transport?.media != null,
    () => ({ playing: intent.current.playing, speed: intent.current.speed, fps: nominalFps(rate), last }),
    run,
  );

  // What the pool is given stays the same from one moment to the next, so that it is drawn
  // again only when the project or the proxies change, not at every move of the playhead.
  const now = useRef({ load: (_media: string) => {}, importMedia: async () => {}, makeAgain: (_media: string) => {} });
  now.current = {
    load: (media) => {
      setProblem(null);
      ask('transport.load', { media }, words.hint.studio.loadFailed);
    },
    importMedia: async () => {
      setProblem(null);
      try {
        const answer = await bridge.importFiles({ language });
        if (!answer.ok) fail(words.hint.studio.importFailed)(new Error(answer.error.message));
      } catch (failure) {
        fail(words.hint.studio.importFailed)(failure);
      }
    },
    makeAgain: media => ask('media.make_proxy', { media }, words.hint.studio.failed),
  };
  const onLoad = useCallback((media: string) => now.current.load(media), []);
  const onImport = useCallback(() => void now.current.importMedia(), []);
  const onMakeAgain = useCallback((media: string) => now.current.makeAgain(media), []);

  const notice: ViewerNotice | null = opening.state === 'failed'
    ? { kind: 'failed', missing: NO_ENGINE.test(opening.message), detail: opening.message }
    : snapshot.takingOver ? { kind: 'takingOver' }
      : ended !== null && ended === snapshot.session ? { kind: 'stopped' }
        : opening.state === 'opening' ? { kind: 'opening' }
          : snapshot.view.state === 'unsupported' ? { kind: snapshot.view.reason === 'headless' ? 'headless' : 'unsupported' }
            : native ? null : { kind: 'browser' };
  const again = () => {
    setEnded(null);
    setProblem(null);
    setAttempt(count => count + 1);
  };

  // What is said of what failed: what the page did, else what the engine said by itself.
  const shownProblem: Problem | null = problem
    ?? (snapshot.error !== null && snapshot.error !== putAway ? { said: words.hint.studio.engineError, detail: sentenceOf(snapshot.error) } : null);
  const dismiss = () => {
    if (problem) setProblem(null);
    else setPutAway(snapshot.error);
  };

  // The tabs of the pages, as a keyboard knows them: one takes the Tab key, the arrows go from one to the other.
  const tabKey = (event: KeyboardEvent) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const at = pages.indexOf(page);
    const next = pages[(at + (event.key === 'ArrowRight' ? 1 : pages.length - 1)) % pages.length];
    setPage(next);
    tabs.current?.querySelector<HTMLElement>(`[data-test="page-${next}"]`)?.focus();
  };

  const conversationWords = conversationHidden ? words.label.studio.showConversation : words.label.studio.hideConversation;
  return <section ref={root} className="studio" tabIndex={-1} aria-label={words.label.studio.open} data-page={page} data-test="studio">
    <header className="studio-head">
      <h1 className="studio-title">{words.label.studio.open}</h1>
      <span className="studio-media" title={loaded?.path} data-test="loaded">
        {loaded ? fileName(loaded.path) : words.label.studio.nothingLoaded}
      </span>
      {onToggleConversation && <button
        type="button"
        className="icon-btn"
        aria-pressed={!conversationHidden}
        aria-label={conversationWords}
        title={`${conversationWords} (${shortcut('Shift+B')})`}
        onClick={onToggleConversation}
        data-test="toggle-conversation"
      >
        {conversationHidden ? <PanelRightOpen size={16} /> : <PanelRightClose size={16} />}
      </button>}
      <button type="button" className="icon-btn" onClick={onClose} aria-label={words.label.studio.close} title={words.label.studio.close} data-test="close-studio">
        <X size={16} />
      </button>
    </header>
    {shownProblem && <div className="studio-problem" role="alert" data-test="studio-problem">
      <div className="studio-problem-words">
        <p data-test="problem-said">{shownProblem.said}</p>
        {shownProblem.detail && <p className="caption" lang="en" title={shownProblem.detail} data-test="problem-detail">{shownProblem.detail}</p>}
      </div>
      <button type="button" className="icon-btn" onClick={dismiss} aria-label={words.label.close} title={words.label.close} data-test="problem-close">
        <X size={14} />
      </button>
    </div>}
    <div className="studio-body">
      {page === 'media' && <MediaPool
        media={project?.media ?? null}
        timelines={timelines}
        proxies={snapshot.proxies}
        loaded={loaded?.id ?? null}
        onImport={onImport}
        onLoad={onLoad}
        onMakeAgain={onMakeAgain}
      />}
      <div className="monitor">
        <Viewer bridge={bridge} native={native} active notice={notice} onRetry={again} />
        {!loaded && (notice === null || notice.kind === 'browser') && <p className="caption monitor-hint" data-test="load-hint">{words.hint.studio.load}</p>}
        <Transport
          state={transport}
          rate={rate}
          last={last}
          onCommand={run}
          onSeek={(frame, scrubbing) => ask('transport.seek', { frame, scrubbing }, words.hint.studio.failed)}
          onProxies={on => ask('transport.proxies', { on }, words.hint.studio.failed)}
        />
      </div>
    </div>
    <nav ref={tabs} className="studio-pages" role="tablist" aria-label={words.label.studio.pages} onKeyDown={tabKey}>
      {pages.map(each => <button
        key={each}
        type="button"
        role="tab"
        aria-selected={page === each}
        tabIndex={page === each ? 0 : -1}
        onClick={() => setPage(each)}
        data-test={`page-${each}`}
      >
        {each === 'media' ? <FolderOpen size={15} aria-hidden="true" /> : <Scissors size={15} aria-hidden="true" />}
        <span>{words.label.studio.page[each]}</span>
      </button>)}
    </nav>
  </section>;
}
