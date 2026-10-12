import { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, ChevronRight, CornerDownRight, SquareTerminal } from 'lucide-react';
import type { Listed } from '../key/form';
import { classifyModelFailure, turnOutcome, type ActionKey, type TurnOutcome } from '../outcomes';
import type { ConversationItem, StreamPreview, UserItem } from '../state';
import { groupTurns, placed, type Turn } from '../turns';
import type { Workspace } from '../useWorkspace';
import { sayModelFailure, sayTurn } from '../words';
import { CopyButton, Markdown } from './Markdown';
import { ToolRow } from './ToolRow';
import { useWords } from './locale';
import { keyAtHand, serviceTitle } from './tell';

/**
 * The services that are set up and the one that the service process talks to, for whatever
 * part names the service of a request or asks whether a key is there for the next one.
 */
const Services = createContext<{ listed: Listed | null; inUse: string | null }>({ listed: null, inUse: null });

/** After this long, a wait says how long it has lasted. */
const SAY_HOW_LONG_AFTER = 5;

/** What a person said, as the block that a turn opens with. */
function Said({ item }: { item: UserItem }) {
  // What a plugin or a hook put in is of interest to few, and is kept folded.
  if (item.source !== 'user' && item.source !== 'steer') {
    return <details className="aside">
      <summary><ChevronRight size={13} aria-hidden="true" />{item.source}</summary>
      <pre className="block" tabIndex={0}>{item.text}</pre>
    </details>;
  }
  return <div className="said" data-test="said">{item.text}</div>;
}

function Summary({ text }: { text: string }) {
  const words = useWords();
  return <details className="aside summary">
    <summary><ChevronRight size={13} aria-hidden="true" />{words.label.summary}</summary>
    <pre className="block" tabIndex={0}>{text}</pre>
  </details>;
}

/**
 * The line of a request that is tried again. While the wait lasts it counts
 * down, and afterwards it tells what happened.
 */
function Retry({ at, delayMs, attempt, live, came }: {
  at: number;
  delayMs: number;
  attempt: number;
  live: boolean;
  /** Whether the request was made again. A turn that is stopped while it waits makes none. */
  came: boolean;
}) {
  const words = useWords();
  const [now, setNow] = useState(Date.now);
  const left = Math.max(0, Math.ceil((at + delayMs - now) / 1000));
  useEffect(() => {
    if (!live || left === 0) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [live, left === 0]);
  if (!live) return <p className="note">{came ? words.wait.retried(delayMs / 1000, attempt) : words.wait.gaveUp}</p>;
  return <p className="note" role="status" data-test="retrying">
    <span className="moon" aria-hidden="true" />{words.wait.retry(left, attempt)}
  </p>;
}

/**
 * One thing that happened on the way of a turn.
 *
 * Args:
 *   live: Whether this is the last thing of a turn that is under way.
 *   last: Whether nothing came after this in its turn.
 */
function Step({ item, live = false, last = false, onLink }: {
  item: ConversationItem;
  live?: boolean;
  last?: boolean;
  onLink: (url: string) => void;
}) {
  const words = useWords();
  const services = useContext(Services).listed;
  switch (item.type) {
    case 'tool':
      return <ToolRow item={item} />;
    case 'assistant':
      return <div className="step-text">
        {item.reasoning && <details className="aside">
          <summary><ChevronRight size={13} aria-hidden="true" />{words.label.thought}</summary>
          <p className="thought">{item.reasoning}</p>
        </details>}
        {item.text && <Markdown text={item.text} onLink={onLink} />}
      </div>;
    case 'user':
      if (item.source === 'steer') {
        return <div className="steer">
          <span className="caption"><CornerDownRight size={12} aria-hidden="true" />{words.label.saidOnTheWay}</span>
          <div className="said said-small">{item.text}</div>
        </div>;
      }
      return <Said item={item} />;
    case 'compaction':
      return <Summary text={item.text} />;
    case 'notice': {
      const cause = item.cause;
      if (cause.kind === 'attempt') {
        // Of the service that the request went to. Another one may be in use by now.
        const saying = sayModelFailure(words, classifyModelFailure(cause.failure), serviceTitle(cause.provider, services));
        return <p className="note" data-tone="warn">{saying.what}</p>;
      }
      if (cause.kind === 'retry') {
        return <Retry at={item.time} delayMs={cause.delayMs} attempt={cause.attempt} live={live} came={!last} />;
      }
      if (cause.kind === 'compaction-failed') return <p className="note" data-tone="warn">{words.notice.compactionFailed.what}</p>;
      if (cause.kind === 'compaction-interrupted') return <p className="note" data-tone="warn">{words.notice.compactionInterrupted.what}</p>;
      // The end of a turn has a place of its own, below the turn.
      return null;
    }
  }
}

/** A line that says what is waited for, and after a while for how long. */
function Waiting({ children }: { children: string }) {
  const words = useWords();
  const [seconds, setSeconds] = useState(0);
  useEffect(() => {
    const began = Date.now();
    const timer = setInterval(() => setSeconds(Math.floor((Date.now() - began) / 1000)), 1000);
    return () => clearInterval(timer);
  }, []);
  return <p className="working" role="status" data-test="working">
    <span className="moon" aria-hidden="true" />
    {children}
    {seconds >= SAY_HOW_LONG_AFTER && <span className="working-time">{words.duration(seconds)}</span>}
  </p>;
}

/** What the model is writing at this moment. It is gone when the reply is in the log. */
function Live({ preview, onLink }: { preview: StreamPreview; onLink: (url: string) => void }) {
  const words = useWords();
  const busy = preview.tools.length > 0 ? words.label.preparing
    : preview.text ? words.label.writing : words.label.thinking;
  return <div className="live" data-test="live">
    {preview.reasoning && !preview.text && <details className="aside">
      <summary><ChevronRight size={13} aria-hidden="true" />{words.label.thought}</summary>
      <p className="thought">{preview.reasoning}</p>
    </details>}
    {preview.text && <Markdown text={preview.text} onLink={onLink} />}
    <Waiting>{busy}</Waiting>
  </div>;
}

/** How a turn ended that did not come to its end as turns are meant to. */
function Ending({ turn, outcome, onAction }: {
  turn: Turn;
  outcome: TurnOutcome;
  /** Null where nothing can be done about this end any more. */
  onAction: ((action: ActionKey, turn: Turn) => void) | null;
}) {
  const words = useWords();
  const services = useContext(Services);
  const saying = sayTurn(words, outcome, serviceTitle(turn.provider, services.listed));
  if (saying === null) return null;
  // Somebody stopping a turn is no failure. Everything else here is something that went wrong.
  const calm = outcome.kind === 'stopped' || outcome.kind === 'interrupted' || outcome.kind === 'forked';
  const action = outcome.next === 'none' ? null : outcome.next;
  // A turn that failed for its key leads to the settings. Once a key is there it can be tried
  // again as well, without the task being typed a second time.
  const again = action === 'openModelSettings' && keyAtHand(services.inUse, services.listed);
  return <div className="ending" data-tone={calm ? 'calm' : 'bad'} data-test="ending" role={calm ? undefined : 'alert'}>
    <div className="ending-words">
      <p className="ending-what">{saying.what}</p>
      {saying.why && <p className="ending-why">{saying.why}</p>}
      {/* The reason of the hook, as the hook gave it. */}
      {outcome.kind === 'inputStopped' && outcome.said && <p className="ending-why">{outcome.said}</p>}
      {!calm && outcome.kind !== 'inputStopped' && outcome.detail && <details className="aside">
        <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
        <p className="tool-code mono">{outcome.detail}</p>
      </details>}
    </div>
    {action && onAction && <div className="ending-actions">
      <button type="button" className="btn" onClick={() => onAction(action, turn)} data-test="ending-action">
        {words.actions[action]}
      </button>
      {again && <button type="button" className="btn" onClick={() => onAction('retry', turn)} data-test="ending-again">
        {words.actions.retry}
      </button>}
    </div>}
  </div>;
}

function TurnView({ turn, preview, waiting, onLink, onAction }: {
  turn: Turn;
  preview: StreamPreview | null;
  /** What the turn waits for while nothing of it is being written, or null. */
  waiting: string | null;
  onLink: (url: string) => void;
  onAction: ((action: ActionKey, turn: Turn) => void) | null;
}) {
  const words = useWords();
  const outcome = turnOutcome(turn);
  // With an answer below them, the steps fold into one line. The user may unfold them,
  // and what the user chose stays.
  const folds = turn.answer !== null && turn.steps.length > 0;
  const [chosen, setChosen] = useState<boolean | null>(null);
  const open = chosen ?? !folds;
  const seconds = turn.end ? Math.max(0, (turn.end.time - turn.startedAt) / 1000) : 0;
  // The request that failed last is what the turn ended with, and the end says it.
  const lastStep = turn.steps.at(-1);
  const toldAtEnd = outcome?.kind === 'modelFailed' && lastStep?.type === 'notice' && lastStep.cause.kind === 'attempt'
    ? lastStep : null;
  const steps = toldAtEnd ? turn.steps.slice(0, -1) : turn.steps;

  return <section className="turn" data-test="turn" data-running={turn.running || undefined}>
    {turn.opening.map(item => <Said key={item.id} item={item} />)}

    {folds && <button type="button" className="steps-toggle" aria-expanded={open} onClick={() => setChosen(!open)} data-test="steps-toggle">
      <ChevronRight size={13} aria-hidden="true" />
      {words.turn.took(seconds, turn.rounds)}
    </button>}
    {open && steps.length > 0 && <div className="steps">
      {steps.map((item, index) => <Step
        key={item.id}
        item={item}
        live={turn.running && index === steps.length - 1}
        // What the end of the turn tells of came after it all the same.
        last={item === turn.steps.at(-1)}
        onLink={onLink}
      />)}
    </div>}

    {preview && <Live preview={preview} onLink={onLink} />}
    {waiting !== null && <Waiting>{waiting}</Waiting>}

    {turn.answer && <div className="answer" data-test="answer">
      <Markdown text={turn.answer.text} onLink={onLink} />
      <div className="answer-tools"><CopyButton text={turn.answer.text} /></div>
    </div>}

    {outcome && <Ending turn={turn} outcome={outcome} onAction={onAction} />}
  </section>;
}

/**
 * The conversation of a task, turn by turn.
 *
 * It follows what is written for as long as the user stays at its end. Whoever
 * scrolls up to read is left there, and is offered a way back.
 */
export function Conversation({ workspace, onLink, onAction }: {
  workspace: Workspace;
  onLink: (url: string) => void;
  onAction: (action: ActionKey, turn: Turn) => void;
}) {
  const words = useWords();
  const { projection, preview, outputs, status } = workspace;
  const entries = useMemo(() => groupTurns(projection), [projection]);
  // What commands answered stands where it was asked, between the turns.
  const drawn = useMemo(() => placed(entries, outputs), [entries, outputs]);
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const follows = useRef(true);
  const [away, setAway] = useState(false);

  const writing = preview.text !== '' || preview.reasoning !== '' || preview.tools.length > 0;
  const running = status?.status === 'running';
  const inUse = status?.model.provider ?? null;
  const services = useMemo(() => ({ listed: workspace.services, inUse }), [workspace.services, inUse]);
  const last = entries.at(-1);
  const liveTurn = last?.kind === 'turn' && last.running ? last : null;
  const lastItem = projection.items.at(-1);
  // A call that runs, or waits for a yes, says so in its own row, and so does a request
  // that is about to be tried again.
  const callOpen = lastItem?.type === 'tool' && lastItem.result === null && !lastItem.interrupted;
  const retrying = lastItem?.type === 'notice' && lastItem.cause.kind === 'retry';
  const quiet = running && !writing && !callOpen && !retrying && !workspace.approval;
  const waiting = !quiet ? null : projection.compacting ? words.hint.compacting : words.label.thinking;

  // Whatever makes the conversation longer or its window smaller: stay at the end
  // for as long as the user has not left it.
  useEffect(() => {
    const node = scroller.current;
    const inner = content.current;
    if (!node || !inner) return;
    const stay = () => { if (follows.current) node.scrollTop = node.scrollHeight; };
    const observer = new ResizeObserver(stay);
    observer.observe(node);
    observer.observe(inner);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    follows.current = true;
    setAway(false);
    const node = scroller.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [workspace.sessionId]);

  const toLatest = () => {
    const node = scroller.current;
    if (!node) return;
    const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    node.scrollTo({ top: node.scrollHeight, behavior: still ? 'instant' : 'smooth' });
    follows.current = true;
    setAway(false);
  };

  return <div className="talk-wrap">
    <div
      className="talk-scroll"
      ref={scroller}
      data-test="talk-scroll"
      onScroll={() => {
        const node = scroller.current;
        if (!node) return;
        follows.current = node.scrollHeight - node.scrollTop - node.clientHeight < 80;
        setAway(!follows.current);
      }}
    >
      <div className="talk" ref={content} data-test="talk">
        <Services.Provider value={services}>
        {drawn.map(entry => entry.kind === 'beside'
          ? <div className="command-said" key={`said-${entry.said.id}`} data-test="command-said">
              <div className="caption"><SquareTerminal size={12} aria-hidden="true" /><code>{entry.said.line}</code></div>
              <pre className="block" tabIndex={0}>{entry.said.text}</pre>
            </div>
          : entry.kind === 'turn'
          ? <TurnView
              key={`turn-${entry.number}`}
              turn={entry}
              preview={entry === liveTurn && writing ? preview : null}
              waiting={entry === liveTurn ? waiting : null}
              onLink={onLink}
              // Only about the end of the last turn can something be done, and only at rest.
              // Nothing can be tried again while the service is gone.
              onAction={entry === last && !running && workspace.connected ? onAction : null}
            />
          : entry.item.type === 'compaction'
            ? <Summary key={entry.item.id} text={entry.item.text} />
            : <Step key={entry.item.id} item={entry.item} onLink={onLink} />)}
        </Services.Provider>

        {/* Between a task and the turn that takes it, the agent is at work and no turn shows it. */}
        {waiting !== null && !liveTurn && <Waiting>{waiting}</Waiting>}
      </div>
    </div>
    {away && <button type="button" className="btn latest" onClick={toLatest} data-test="latest">
      <ArrowDown size={14} aria-hidden="true" />{words.label.latest}
    </button>}
  </div>;
}
