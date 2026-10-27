import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { FolderOpen, Scissors, X } from 'lucide-react';
import { useWords } from '../app/locale';
import { sentenceOf } from '../key/form';
import type { StudioEngine } from './connection';
import { callEngine, type PageMethod, type TransportState } from './engine';
import { afterCommand, useStudioKeys, type TransportCommand } from './keys';
import { MediaPool } from './MediaPool';
import { fileName, framesOf, nominalFps, rateOf } from './timecode';
import { Transport } from './Transport';
import { Viewer, type ViewerNotice } from './Viewer';

/** The pages of the studio, at its foot as in DaVinci Resolve. Those of the later phases come with them. */
export type StudioPage = 'media' | 'edit';
const pages: readonly StudioPage[] = ['media', 'edit'];

/** What failed, in the words of whatever failed, without what Electron says of the call that carried it. */
export function failureOf(problem: unknown): string {
  const said = problem instanceof Error ? problem.message : String(problem);
  return said.replace(/^Error invoking remote method '[^']*': (?:[A-Za-z]*Error: )?/, '').trim();
}

/** What the shell says where it has no engine program to start: the engine is not installed. */
const NO_ENGINE = /\bno engine\b/i;

type Opening = { state: 'opening' } | { state: 'open'; session: number } | { state: 'failed'; message: string };

/**
 * The studio: the media of the project and a source monitor that plays one of them. It opens the
 * project of the folder in the engine as it is shown (the engine is kept while the app is on the
 * folder), and hides the engine's view as it goes.
 *
 * Args:
 *   cwd: The folder of the project, which the app has opened.
 *   engine: The engine of the studio, as `studioEngine` gives it.
 *   language: The language of the page, in which the dialog of the system is titled.
 *   onClose: Leave the studio.
 */
export function Studio({ cwd, engine, language, onClose }: {
  cwd: string;
  engine: StudioEngine;
  language: string;
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
  const [problem, setProblem] = useState<string | null>(null);
  const root = useRef<HTMLElement>(null);

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
  const timelines = project?.timelines ?? [];
  const transport = snapshot.transport ?? (asked !== null && asked.session === snapshot.session ? asked.state : null);
  const loaded = project?.media.find(media => media.id === transport?.media) ?? null;
  const rate = rateOf(loaded, timelines);
  const last = transport?.media == null ? null
    : transport.last ?? (loaded ? Math.max(0, framesOf(loaded.facts.duration, rate) - 1) : null);

  const fail = (hint: string) => (failure: unknown) => setProblem(`${hint} ${sentenceOf(failureOf(failure))}`.trim());
  const ask = (method: PageMethod, params: Record<string, unknown> | undefined, hint: string) => {
    void callEngine(bridge, method, params).catch(fail(hint));
  };

  // What the transport does as the keys know it: what they asked for last, until the transport tells.
  const intent = useRef({ playing: false, speed: 1 });
  useEffect(() => { if (transport) intent.current = { playing: transport.playing, speed: transport.speed }; }, [transport]);
  const run = (command: TransportCommand) => {
    intent.current = afterCommand(intent.current, command);
    ask(command.method, command.params, words.hint.studio.failed);
  };
  useStudioKeys(
    transport?.media != null,
    () => ({ ...intent.current, fps: nominalFps(rate), last }),
    run,
  );

  const load = (media: string) => {
    setProblem(null);
    ask('transport.load', { media }, words.hint.studio.loadFailed);
  };
  const importMedia = async () => {
    setProblem(null);
    try {
      const answer = await bridge.importFiles({ language });
      if (!answer.ok) fail(words.hint.studio.importFailed)(new Error(answer.error.message));
    } catch (failure) {
      fail(words.hint.studio.importFailed)(failure);
    }
  };

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

  return <section ref={root} className="studio" tabIndex={-1} aria-label={words.label.studio.open} data-page={page} data-test="studio">
    <header className="studio-head">
      <h1 className="studio-title">{words.label.studio.open}</h1>
      <span className="studio-media" title={loaded?.path} data-test="loaded">
        {loaded ? fileName(loaded.path) : words.label.studio.nothingLoaded}
      </span>
      <button type="button" className="icon-btn" onClick={onClose} aria-label={words.label.studio.close} title={words.label.studio.close} data-test="close-studio">
        <X size={16} />
      </button>
    </header>
    {(problem ?? snapshot.error) && <div className="studio-problem" role="alert" data-test="studio-problem">
      <p className="caption">{problem ?? snapshot.error}</p>
      {problem && <button type="button" className="icon-btn" onClick={() => setProblem(null)} aria-label={words.label.close} title={words.label.close}>
        <X size={14} />
      </button>}
    </div>}
    <div className="studio-body">
      {page === 'media' && <MediaPool
        media={project?.media ?? []}
        timelines={timelines}
        proxies={snapshot.proxies}
        loaded={loaded?.id ?? null}
        onImport={() => void importMedia()}
        onLoad={load}
        onMakeAgain={media => ask('media.make_proxy', { media }, words.hint.studio.failed)}
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
    <nav className="studio-pages" role="tablist" aria-label={words.label.studio.pages}>
      {pages.map(each => <button
        key={each}
        type="button"
        role="tab"
        aria-selected={page === each}
        onClick={() => setPage(each)}
        data-test={`page-${each}`}
      >
        {each === 'media' ? <FolderOpen size={15} aria-hidden="true" /> : <Scissors size={15} aria-hidden="true" />}
        <span>{words.label.studio.page[each]}</span>
      </button>)}
    </nav>
  </section>;
}
