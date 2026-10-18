import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { ArrowDown, Check, ChevronRight, Copy, Search, X } from 'lucide-react';
import type { Usage } from '../protocol';
import type { Workspace } from '../useWorkspace';
import { useWords } from './locale';
import { countOf } from './tell';
import { holds, traceOf, wentWrong, type CallState, type Trace, type TraceKind, type TraceRow, type TraceStep } from './trace';

/** The kinds of rows, in the order of their switches. */
const KINDS: readonly TraceKind[] = ['input', 'thinking', 'reply', 'tool', 'notice'];
/** The kinds that are shown at first. What the model thought would double the rows, and is shown when it is asked for. */
const SHOWN_AT_FIRST: readonly TraceKind[] = ['input', 'reply', 'tool', 'notice'];
/** How long something takes before it is worth saying, in milliseconds. */
const WORTH_SAYING = 50;
/** How long a search waits for the next key before it looks, in milliseconds. */
const SEARCH_WAIT = 100;
/** How close to its end the list is still at its end, in pixels. */
const NEAR_END = 80;

const firstLine = (text: string) => text.split('\n').find(line => line.trim() !== '')?.trim() ?? '';
const read = (usage: Usage) => usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;

/** The arguments of a call on one line, as compact as they were written, or as they are where they are no JSON. */
function argumentsLine(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text));
  } catch {
    return text.replace(/\s+/g, ' ').trim();
  }
}

/** The arguments of a call laid out to be read, or as they are where they are no JSON. */
function argumentsLaidOut(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

/** The worst of how the calls of a step stand, which the overview shows in its colour. */
function worstOf(calls: readonly CallState[]): CallState | null {
  for (const state of ['failed', 'refused', 'waiting', 'running', 'cut', 'done'] as const) {
    if (calls.includes(state)) return state;
  }
  return null;
}

/** How a turn ended, in the words of the page: one of those that it knows, or that it ended. */
function endedAs(ends: Record<'completed' | 'aborted' | 'blocked' | 'error' | 'max-tokens' | 'interrupted' | 'forked' | 'running' | 'other', string>, reason: string): string {
  return reason in ends && reason !== 'running' && reason !== 'other' ? ends[reason as keyof typeof ends] : ends.other;
}

/** A button that copies a text, and says for a moment that it did. */
function CopyButton({ text, label }: { text: string; label: string }) {
  const words = useWords();
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return <button
    type="button"
    className="icon-btn trace-copy"
    aria-label={copied ? words.label.trace.copied : label}
    title={copied ? words.label.trace.copied : label}
    onClick={() => void navigator.clipboard?.writeText(text).then(() => setCopied(true), () => {})}
  >
    {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
  </button>;
}

/**
 * The steps of the task side by side, one column for each, in three lanes: whether input came
 * in for the step, its request to the model, which is the darker the longer it took, and its
 * calls, in the colour of the worst of how they stand. A gap stands between turns. A column
 * leads to the first row of its step.
 */
function Overview({ trace, onStep }: { trace: Trace; onStep: (index: number) => void }) {
  const words = useWords();
  // The step that the keyboard is on, or null before it has been on one.
  const [at, setAt] = useState<number | null>(null);
  const longest = Math.max(1, ...trace.steps.map(step => step.answer?.tookMs ?? 0));
  const WIDTH = 8;
  const GAP = 2;
  const TURN_GAP = 8;
  let x = 0;
  let turn: number | null = null;
  const columns = trace.steps.map((step, index) => {
    if (turn !== null && step.turn !== turn) x += TURN_GAP;
    turn = step.turn;
    const left = x;
    x += WIDTH + GAP;
    return { step, index, left };
  });
  if (columns.length === 0) return null;
  const told = (step: TraceStep) => words.hint.trace.step({
    turn: step.turn, step: step.step,
    took: step.answer?.tookMs == null ? null : words.duration(step.answer.tookMs / 1000),
    calls: step.calls.length,
    wrong: step.calls.filter(state => state === 'failed' || state === 'refused').length,
    failed: step.failed,
  });
  return <div className="trace-overview" data-test="trace-overview">
    <svg
      width={Math.max(x - GAP, 1)}
      height={34}
      role="list"
      tabIndex={0}
      aria-label={words.label.trace.overview}
      aria-describedby={at === null ? undefined : `trace-step-${at}`}
      onKeyDown={(event) => {
        const last = columns.length - 1;
        const now = at ?? last;
        const next = event.key === 'ArrowLeft' ? Math.max(0, now - 1)
          : event.key === 'ArrowRight' ? Math.min(last, now + 1)
            : event.key === 'Home' ? 0 : event.key === 'End' ? last : null;
        if (next !== null) {
          event.preventDefault();
          setAt(next);
        } else if ((event.key === 'Enter' || event.key === ' ') && at !== null) {
          event.preventDefault();
          onStep(at);
        }
      }}
      onBlur={() => setAt(null)}
    >
      {columns.map(({ step, index, left }) => {
        const calls = worstOf(step.calls);
        const took = step.answer?.tookMs ?? 0;
        return <g
          key={step.seq}
          role="listitem"
          id={`trace-step-${index}`}
          className="trace-column"
          data-at={index === at || undefined}
          aria-label={told(step)}
          data-test="trace-column"
          onClick={() => onStep(index)}
        >
          <title>{told(step)}</title>
          <rect x={left} y={0} width={WIDTH} height={34} className="trace-hit" />
          {step.input && <rect x={left} y={1} width={WIDTH} height={8} rx={2} className="trace-cell" data-lane="input" />}
          <rect
            x={left} y={13} width={WIDTH} height={8} rx={2} className="trace-cell" data-lane="model"
            data-failed={step.failed > 0 || undefined}
            style={{ opacity: step.answer ? 0.35 + 0.65 * took / longest : 0.2 }}
          />
          {calls && <rect x={left} y={25} width={WIDTH} height={8} rx={2} className="trace-cell" data-lane="tool" data-state={calls} />}
        </g>;
      })}
    </svg>
  </div>;
}

/** What a row says on its line, beside its kind. */
function Summary({ row, step }: { row: TraceRow; step: TraceStep | null }) {
  const words = useWords();
  switch (row.kind) {
    case 'input':
    case 'thinking':
    case 'notice':
      return <span className="trace-main">{firstLine(row.text)}</span>;
    case 'reply': {
      const line = firstLine(row.text);
      const calls = step?.calls.length ?? 0;
      return <span className="trace-main" data-quiet={line === '' || undefined}>
        {line || (calls > 0 ? words.label.trace.onlyCalls(calls) : words.label.trace.nothingWritten)}
      </span>;
    }
    case 'tool': {
      const { item } = row;
      const after = item.error ? `${item.error.code}${item.error.reason ? ` ${item.error.reason}` : ''}` : firstLine(item.result ?? '');
      return <span className="trace-main">
        <span className="trace-name">{item.name}</span>
        <span className="trace-args mono">{argumentsLine(item.arguments)}</span>
        {after && <span className="trace-result mono">→ {after}</span>}
      </span>;
    }
  }
}

/** What a row says at its end: how long it took, what it read and wrote, or how a call stands. */
function Meta({ row, locale }: { row: TraceRow; locale: string }) {
  const words = useWords();
  if (row.kind === 'reply') {
    const parts = [
      ...(row.tookMs === null || row.tookMs < WORTH_SAYING ? [] : [words.duration(row.tookMs / 1000)]),
      words.label.trace.tokens(countOf(read(row.usage), locale), countOf(row.usage.output_tokens, locale)),
      ...(row.cut ? [words.label.trace.cut] : []),
    ];
    return <span className="trace-meta">{parts.join(' · ')}</span>;
  }
  if (row.kind === 'tool') {
    const state = row.state === 'done' || row.state === 'failed' ? null : words.label.trace.states[row.state];
    return <span className="trace-meta">
      {state ?? (row.item.durationMs === null || row.item.durationMs < WORTH_SAYING ? '' : words.duration(row.item.durationMs / 1000))}
    </span>;
  }
  return null;
}

/** All that a row holds, once it is opened. */
function Detail({ row }: { row: TraceRow }) {
  const words = useWords();
  const block = (title: string, text: string, mono = true): ReactNode => <div className="trace-block">
    <div className="trace-block-head"><span className="overline">{title}</span><CopyButton text={text} label={words.label.trace.copy} /></div>
    <pre className={mono ? 'block mono' : 'block'} tabIndex={0}>{text}</pre>
  </div>;
  if (row.kind !== 'tool') return <div className="trace-detail">{block(words.label.trace.kinds[row.kind], row.text, false)}</div>;
  const { item } = row;
  return <div className="trace-detail">
    {block(words.label.trace.arguments, argumentsLaidOut(item.arguments))}
    {item.error && block(words.label.trace.error, `${item.error.code}${item.error.reason ? `\n${item.error.reason}` : ''}`)}
    {item.result !== null && item.result.trim() !== (item.error?.reason ?? '').trim() && block(words.label.trace.result, item.result)}
  </div>;
}

/**
 * The trace of the task that is open: its steps side by side above, and below them, turn by
 * turn, what was said, what the model thought and answered, and every call, each with what it
 * took. Rows can be left out by their kind, looked through, and opened to read all that they
 * hold. It follows what comes for as long as the user stays at its end.
 */
export function TraceView({ workspace, locale }: { workspace: Workspace; locale: string }) {
  const words = useWords();
  const trace = useMemo(() => traceOf(workspace.events, workspace.projection), [workspace.events, workspace.projection]);
  const [shown, setShown] = useState<ReadonlySet<TraceKind>>(() => new Set(SHOWN_AT_FIRST));
  const [onlyWrong, setOnlyWrong] = useState(false);
  const [typed, setTyped] = useState('');
  const [sought, setSought] = useState('');
  const [opened, setOpened] = useState<ReadonlySet<string>>(() => new Set());
  const [folded, setFolded] = useState<ReadonlySet<number>>(() => new Set());
  const [marked, setMarked] = useState<string | null>(null);
  const [away, setAway] = useState(false);
  // The row to bring into sight once it is drawn, and whether the keyboard goes to it.
  const [reveal, setReveal] = useState<{ id: string; keyboard: boolean } | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const searchBox = useRef<HTMLInputElement>(null);
  const follows = useRef(true);

  // A search looks once the keys have stopped for a moment.
  useEffect(() => {
    const timer = setTimeout(() => setSought(typed), SEARCH_WAIT);
    return () => clearTimeout(timer);
  }, [typed]);

  const visible = useMemo(
    () => trace.rows.filter(row => shown.has(row.kind) && (!onlyWrong || wentWrong(row))),
    [trace.rows, shown, onlyWrong],
  );
  const found = useMemo(() => (sought.trim() === '' ? [] : visible.filter(row => holds(row, sought))), [visible, sought]);
  const foundAt = marked === null ? -1 : found.findIndex(row => row.id === marked);

  // What has no turn stands first, and then each turn with its rows.
  const groups = useMemo(() => {
    const outside = visible.filter(row => row.turn === null);
    const byTurn = trace.turns.map(turn => ({ turn, rows: visible.filter(row => row.turn === turn.number) }));
    return { outside, byTurn };
  }, [visible, trace.turns]);

  // Whatever makes the trace longer or its window smaller: stay at the end for as long as the user has not left it.
  useEffect(() => {
    const node = scroller.current;
    const inner = content.current;
    if (!node || !inner) return undefined;
    const stay = () => { if (follows.current) node.scrollTop = node.scrollHeight; };
    const observer = new ResizeObserver(stay);
    observer.observe(node);
    observer.observe(inner);
    return () => observer.disconnect();
  }, []);
  // Another task begins at its end, with nothing opened or marked.
  useEffect(() => {
    follows.current = true;
    setAway(false);
    setOpened(new Set());
    setFolded(new Set());
    setMarked(null);
    const node = scroller.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [workspace.sessionId]);

  /**
   * Bring a row into sight and mark it, unfolding its turn and showing its kind where they hide it.
   * The keyboard goes to the row, unless it is to stay where it is, as in the search.
   */
  const show = (row: TraceRow, keyboard = true) => {
    if (row.turn !== null && folded.has(row.turn)) setFolded(new Set([...folded].filter(each => each !== row.turn)));
    if (!shown.has(row.kind)) setShown(new Set([...shown, row.kind]));
    if (onlyWrong && !wentWrong(row)) setOnlyWrong(false);
    setMarked(row.id);
    setReveal({ id: row.id, keyboard });
  };
  useEffect(() => {
    if (reveal === null) return;
    setReveal(null);
    const node = [...(content.current?.querySelectorAll<HTMLElement>('[data-row]') ?? [])].find(each => each.dataset.row === reveal.id);
    if (!node) return;
    follows.current = false;
    setAway(true);
    node.scrollIntoView({ block: 'center' });
    if (reveal.keyboard) node.focus({ preventScroll: true });
  }, [reveal]);
  // A step leads to the first of its rows that is shown, and only where none is to its first row at all.
  const toStep = (index: number) => {
    const id = trace.steps[index]?.row;
    const row = visible.find(each => each.step === index)
      ?? (id === null || id === undefined ? undefined : trace.rows.find(each => each.id === id));
    if (row) show(row);
  };
  // From one find to the next, of what is written now: the search may not have looked yet.
  const toFound = (by: 1 | -1, query: string) => {
    const finds = query.trim() === '' ? [] : visible.filter(row => holds(row, query));
    if (finds.length === 0) return;
    const at = marked === null ? -1 : finds.findIndex(row => row.id === marked);
    const next = at < 0 ? (by === 1 ? 0 : finds.length - 1) : (at + by + finds.length) % finds.length;
    show(finds[next], false);
  };
  const toggle = (id: string) => {
    const next = new Set(opened);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setOpened(next);
  };
  const toLatest = () => {
    const node = scroller.current;
    if (!node) return;
    const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    node.scrollTo({ top: node.scrollHeight, behavior: still ? 'instant' : 'smooth' });
    follows.current = true;
    setAway(false);
  };

  // The arrows move from row to row, Enter opens or closes one, and a slash goes to the search.
  const onListKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === '/' && event.target !== searchBox.current) {
      event.preventDefault();
      searchBox.current?.focus();
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const rows = [...(content.current?.querySelectorAll<HTMLElement>('[data-row]') ?? [])];
    const at = rows.findIndex(node => node === document.activeElement);
    const next = rows[at < 0 ? 0 : Math.max(0, Math.min(rows.length - 1, at + (event.key === 'ArrowDown' ? 1 : -1)))];
    if (!next) return;
    event.preventDefault();
    next.focus();
    next.scrollIntoView({ block: 'nearest' });
    setMarked(next.dataset.row ?? null);
  };

  const stepOf = (row: TraceRow) => (row.step === null ? null : trace.steps[row.step] ?? null);
  const drawRow = (row: TraceRow) => {
    const open = opened.has(row.id);
    return <div key={row.id} className="trace-item">
      <button
        type="button"
        className="trace-row"
        data-row={row.id}
        data-kind={row.kind}
        data-state={row.kind === 'tool' ? row.state : undefined}
        data-tone={row.kind === 'notice' ? row.tone : undefined}
        data-wrong={wentWrong(row) || undefined}
        data-found={sought.trim() !== '' && holds(row, sought) ? '' : undefined}
        data-marked={row.id === marked || undefined}
        aria-expanded={open}
        onClick={() => { toggle(row.id); setMarked(row.id); }}
        data-test="trace-row"
      >
        <ChevronRight size={12} className="trace-chevron" aria-hidden="true" />
        <span className="trace-tag">{words.label.trace.kinds[row.kind]}</span>
        <Summary row={row} step={stepOf(row)} />
        <Meta row={row} locale={locale} />
      </button>
      {open && <Detail row={row} />}
    </div>;
  };

  if (trace.rows.length === 0) {
    return <div className="trace-wrap"><p className="trace-empty caption" data-test="trace-empty">{words.hint.trace.empty}</p></div>;
  }
  return <div className="trace-wrap" data-test="trace">
    <Overview trace={trace} onStep={toStep} />
    <div className="trace-bar" role="toolbar" aria-label={words.label.trace.filters}>
      {KINDS.map(kind => <button
        key={kind}
        type="button"
        className="chip"
        aria-pressed={shown.has(kind)}
        onClick={() => {
          const next = new Set(shown);
          if (next.has(kind)) next.delete(kind);
          else next.add(kind);
          setShown(next);
        }}
        data-test="trace-kind"
        data-kind={kind}
      >{words.label.trace.kinds[kind]}</button>)}
      <button
        type="button"
        className="chip"
        data-tone="bad"
        aria-pressed={onlyWrong}
        onClick={() => setOnlyWrong(!onlyWrong)}
        data-test="trace-wrong"
      >{words.label.trace.onlyWrong}</button>
      <span className="trace-gap" />
      <label className="trace-search">
        <Search size={13} aria-hidden="true" />
        <input
          ref={searchBox}
          className="field"
          value={typed}
          placeholder={words.label.trace.search}
          aria-label={words.label.trace.search}
          spellCheck={false}
          onChange={event => setTyped(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              event.preventDefault();
              setSought(typed);
              toFound(event.shiftKey ? -1 : 1, typed);
            } else if (event.key === 'Escape' && typed !== '') {
              event.preventDefault();
              event.stopPropagation();
              setTyped('');
              setSought('');
            }
          }}
          data-test="trace-search"
        />
        {typed !== '' && <button type="button" className="icon-btn" aria-label={words.label.trace.clear} title={words.label.trace.clear}
          onClick={() => { setTyped(''); setSought(''); searchBox.current?.focus(); }}>
          <X size={13} aria-hidden="true" />
        </button>}
      </label>
      {sought.trim() !== '' && <span className="caption trace-count" aria-live="polite" data-test="trace-count">
        {found.length === 0 ? words.label.trace.noneFound : words.label.trace.found(Math.max(1, foundAt + 1), found.length)}
      </span>}
    </div>
    <div className="trace-scroll-wrap">
      <div
        className="trace-scroll"
        ref={scroller}
        onScroll={() => {
          const node = scroller.current;
          if (!node) return;
          follows.current = node.scrollHeight - node.scrollTop - node.clientHeight < NEAR_END;
          setAway(!follows.current);
        }}
        onKeyDown={onListKey}
        data-test="trace-scroll"
      >
        <div className="trace-list" ref={content}>
          {groups.outside.map(drawRow)}
          {groups.byTurn.map(({ turn, rows }) => {
            const isFolded = folded.has(turn.number);
            const took = (turn.end?.time ?? (workspace.events.at(-1)?.time ?? turn.startedAt)) - turn.startedAt;
            return <section key={turn.number} className="trace-turn" data-test="trace-turn">
              <button
                type="button"
                className="trace-turn-head"
                aria-expanded={!isFolded}
                onClick={() => {
                  const next = new Set(folded);
                  if (isFolded) next.delete(turn.number);
                  else next.add(turn.number);
                  setFolded(next);
                }}
              >
                <ChevronRight size={13} className="trace-chevron" aria-hidden="true" />
                <span className="trace-turn-name">{words.label.trace.turn(turn.number)}</span>
                <span className="caption">{[
                  words.label.stats.steps(turn.steps),
                  words.duration(took / 1000),
                  words.label.stats.tokens(countOf(turn.tokens, locale)),
                  turn.end ? endedAs(words.label.trace.ends, turn.end.reason) : words.label.trace.ends.running,
                ].join(' · ')}</span>
              </button>
              {!isFolded && rows.map(drawRow)}
            </section>;
          })}
        </div>
      </div>
      {away && <button type="button" className="btn latest" onClick={toLatest} data-test="trace-latest">
        <ArrowDown size={14} aria-hidden="true" />{words.label.latest}
      </button>}
    </div>
  </div>;
}
