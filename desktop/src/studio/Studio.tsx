import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react';
import { Film, FolderOpen, PanelRightClose, PanelRightOpen, Scissors, X } from 'lucide-react';
import { useWords } from '../app/locale';
import { shortcut } from '../app/system';
import { sentenceOf } from '../key/form';
import type { StudioEngine } from './connection';
import { callEngine, type Monitor, type PageMethod, type SourceTransportState, type TimelineTransportState } from './engine';
import { useStudioKeys } from './keys';
import { MediaPool } from './MediaPool';
import { MonitorPane } from './MonitorPane';
import { CutPage } from './pages/CutPage';
import { EditPage } from './pages/EditPage';
import { MediaPage } from './pages/MediaPage';
import { duration } from './timeline/geometry';
import { framesOf, rateOf } from './timecode';
import { useMonitor } from './useMonitor';
import type { ViewerNotice } from './Viewer';

/** The pages of the studio, at its foot as in DaVinci Resolve. Those of the later phases come with them. */
export type StudioPage = 'media' | 'cut' | 'edit';
const pages: readonly StudioPage[] = ['media', 'cut', 'edit'];
const PAGE_ICONS = { media: FolderOpen, cut: Scissors, edit: Film } as const;

/** The timelines of a project that is not there yet: the same list each time, so that the pool is not drawn again for it. */
const NO_TIMELINES: readonly never[] = [];
const NO_BINS: readonly never[] = [];
const NO_MEDIA: readonly never[] = [];
/** The rate of a timeline monitor that has no timeline to go by. */
const NO_RATE = { num: 25, den: 1 } as const;

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

/** What the transports said when they were asked, before they told anything: after the page was loaded again. */
interface Asked {
  session: number;
  source: SourceTransportState | null;
  timeline: TimelineTransportState | null;
}

/**
 * The studio: the media of the project, a source monitor that plays one of them, and on the Cut and
 * Edit pages a timeline monitor and the timeline beside it. It opens the project of the folder in the
 * engine as it is shown (the engine is kept while the app is on the folder), and hides the
 * engine's views as it goes.
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
  const [asked, setAsked] = useState<Asked | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  // What the engine said last that the user put away: it is not shown again until it says something else.
  const [putAway, setPutAway] = useState<string | null>(null);
  // The monitor that the keys go to on the Edit page, and the timeline chosen there.
  const [focus, setFocus] = useState<Monitor>('timeline');
  const [chosenTimeline, setChosenTimeline] = useState<string | null>(null);
  // The bin that the pool shows: the Cut page's source tape is made of its media.
  const [bin, setBin] = useState<string | null>(null);
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
      const told = (state: SourceTransportState | TimelineTransportState) => setAsked(old => {
        const kept = old !== null && old.session === opened.session ? old : { session: opened.session, source: null, timeline: null };
        return { ...kept, [state.monitor]: state };
      });
      for (const monitor of ['source', 'timeline'] as const) {
        callEngine(bridge, 'transport.state', { monitor }).then((state) => {
          if (!gone && state.monitor === monitor) told(state);
        }, () => {});
      }
    }, (failure: unknown) => {
      if (!gone) setOpening({ state: 'failed', message: failureOf(failure) });
    });
    return () => { gone = true; };
  }, [bridge, cwd, attempt]);

  useEffect(() => bridge.onEvent((event) => {
    if (event.kind === 'exited') setEnded(event.session);
    else if (event.kind === 'opened') setEnded(null);
  }), [bridge]);

  // A timeline chosen in one engine's project is not one of another's.
  useEffect(() => { setChosenTimeline(null); }, [snapshot.session]);

  const project = snapshot.project;
  const timelines = project?.timelines ?? NO_TIMELINES;
  const thisSession = asked !== null && asked.session === snapshot.session ? asked : null;
  const sourceState = snapshot.transports.source ?? thisSession?.source ?? null;
  const timelineState = snapshot.transports.timeline ?? thisSession?.timeline ?? null;
  const loaded = project?.media.find(media => media.id === sourceState?.media) ?? null;
  const rate = rateOf(loaded, timelines);
  const last = sourceState?.media == null ? null
    : sourceState.last ?? (loaded ? Math.max(0, framesOf(loaded.facts.duration, rate) - 1) : null);
  // The timeline of the Edit page: the one chosen there, else the one the monitor has, else the first.
  const known = (id: string | null | undefined): id is string => id != null && timelines.some(timeline => timeline.id === id);
  const selectedId = known(chosenTimeline) ? chosenTimeline : known(timelineState?.timeline) ? timelineState.timeline : timelines[0]?.id ?? null;
  const selected = timelines.find(timeline => timeline.id === selectedId) ?? null;
  const playing = timelines.find(timeline => timeline.id === timelineState?.timeline) ?? null;
  const timelineLast = timelineState?.timeline == null ? null
    : timelineState.last ?? (playing ? Math.max(0, duration(playing) - 1) : null);

  const fail = (said: string) => (failure: unknown) => setProblem({ said, detail: sentenceOf(failureOf(failure)) });
  const ask = (method: PageMethod, params: Record<string, unknown> | undefined, said: string) => {
    void callEngine(bridge, method, params).catch(fail(said));
  };
  const source = useMonitor(bridge, 'source', sourceState, rate, last, fail(words.hint.studio.failed));
  const timelineMonitor = useMonitor(bridge, 'timeline', timelineState, (playing ?? selected)?.rate ?? NO_RATE, timelineLast, fail(words.hint.studio.failed));
  // The keys of the Media page go to its source monitor; the Edit page takes its own.
  useStudioKeys(page === 'media' && source.loaded, source.context, source.run);

  // What the pool is given stays the same from one moment to the next, so that it is drawn
  // again only when the project or the proxies change, not at every move of the playhead.
  const now = useRef({ load: (_media: string) => {}, importMedia: async (_bin: string | null) => {}, makeAgain: (_media: string) => {} });
  now.current = {
    load: (media) => {
      setProblem(null);
      // As in DaVinci Resolve, media opened from the pool is seen, and its keys taken, in the source monitor.
      setFocus('source');
      ask('transport.load', { media, monitor: 'source' }, words.hint.studio.loadFailed);
    },
    importMedia: async (bin: string | null) => {
      setProblem(null);
      try {
        const answer = await bridge.importFiles({ language, ...(bin === null ? {} : { bin }) });
        if (!answer.ok) fail(words.hint.studio.importFailed)(new Error(answer.error.message));
      } catch (failure) {
        fail(words.hint.studio.importFailed)(failure);
      }
    },
    makeAgain: media => ask('media.make_proxy', { media }, words.hint.studio.failed),
  };
  const onLoad = useCallback((media: string) => now.current.load(media), []);
  const onImport = useCallback((bin: string | null) => void now.current.importMedia(bin), []);
  const onMakeAgain = useCallback((media: string) => now.current.makeAgain(media), []);

  const noticeOf = (monitor: Monitor): ViewerNotice | null => (opening.state === 'failed'
    ? { kind: 'failed', missing: NO_ENGINE.test(opening.message), detail: opening.message }
    : snapshot.takingOver ? { kind: 'takingOver' }
      : ended !== null && ended === snapshot.session ? { kind: 'stopped' }
        : opening.state === 'opening' ? { kind: 'opening' }
          : snapshot.views[monitor].state === 'unsupported' ? { kind: snapshot.views[monitor].reason === 'headless' ? 'headless' : 'unsupported' }
            : native ? null : { kind: 'browser' });
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

  const pool = <MediaPool
    key={snapshot.session}
    bridge={bridge}
    session={snapshot.session ?? 0}
    bins={project?.bins ?? NO_BINS}
    media={project?.media ?? null}
    timelines={timelines}
    proxies={snapshot.proxies}
    loaded={loaded?.id ?? null}
    onImport={onImport}
    onLoad={onLoad}
    onMakeAgain={onMakeAgain}
    initialGrid={page === 'cut'}
    onBin={setBin}
  />;
  const conversationWords = conversationHidden ? words.label.studio.showConversation : words.label.studio.hideConversation;
  return <section ref={root} className="studio" tabIndex={-1} aria-label={words.label.studio.open} data-page={page} data-test="studio">
    <header className="studio-head">
      <h1 className="studio-title">{words.label.studio.open}</h1>
      <span className="studio-media" title={loaded?.path} data-test="loaded">
        {loaded ? loaded.name : words.label.studio.nothingLoaded}
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
    {page === 'media' && <MediaPage pool={pool} monitor={<MonitorPane controls={source} bridge={bridge} native={native} active
      notice={noticeOf('source')} onRetry={again} hint={words.hint.studio.load} />} />}
    {page === 'cut' && <CutPage
      bridge={bridge}
      native={native}
      session={snapshot.session ?? 0}
      revision={snapshot.revision ?? 0}
      pool={pool}
      media={project?.media ?? NO_MEDIA}
      bin={bin}
      timelines={timelines}
      selected={selected}
      onSelect={setChosenTimeline}
      source={source}
      sourceMedia={loaded}
      timelineMonitor={timelineMonitor}
      focus={focus}
      onFocus={setFocus}
      noticeOf={noticeOf}
      onRetry={again}
      onProblem={(said, failure) => fail(said)(failure)}
    />}
    {page === 'edit' && <EditPage
      bridge={bridge}
      native={native}
      session={snapshot.session ?? 0}
      revision={snapshot.revision ?? 0}
      pool={pool}
      media={project?.media ?? NO_MEDIA}
      timelines={timelines}
      selected={selected}
      onSelect={setChosenTimeline}
      source={source}
      sourceMedia={loaded}
      timelineMonitor={timelineMonitor}
      focus={focus}
      onFocus={setFocus}
      noticeOf={noticeOf}
      onRetry={again}
      onProblem={(said, failure) => fail(said)(failure)}
    />}
    <nav ref={tabs} className="studio-pages" role="tablist" aria-label={words.label.studio.pages} onKeyDown={tabKey}>
      {pages.map((each) => {
        const Icon = PAGE_ICONS[each];
        return <button
          key={each}
          type="button"
          role="tab"
          aria-selected={page === each}
          tabIndex={page === each ? 0 : -1}
          onClick={() => setPage(each)}
          data-test={`page-${each}`}
        >
          <Icon size={15} aria-hidden="true" />
          <span>{words.label.studio.page[each]}</span>
        </button>;
      })}
    </nav>
  </section>;
}
