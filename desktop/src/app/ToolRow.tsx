import { useEffect, useRef, useState } from 'react';
import {
  Check, ChevronRight, CircleSlash, FilePen, FileText, FolderSearch, Hand, ListChecks, Search,
  Sparkles, SquareTerminal, TriangleAlert, Users, Wrench, type LucideIcon,
} from 'lucide-react';
import { classifyTool, type ToolOutcome } from '../outcomes';
import type { ToolItem } from '../state';
import { sayTool } from '../words';
import { useWords } from './locale';
import {
  argumentsOf, callSeconds, commandTools, objectOf, subagentKind, tail, toolKind, verbOf, type ToolKind,
} from './tell';

/** The picture that goes with each kind of tool. */
const icons: Record<ToolKind, LucideIcon> = {
  read: FileText,
  write: FilePen,
  edit: FilePen,
  list: FolderSearch,
  search: Search,
  run: SquareTerminal,
  todo: ListChecks,
  skill: Sparkles,
  subagent: Users,
  other: Wrench,
};

type Tone = 'live' | 'ok' | 'held' | 'bad';

function toneOf(outcome: ToolOutcome): Tone {
  switch (outcome.kind) {
    case 'running':
    case 'waitingForApproval':
      return 'live';
    case 'done':
      return 'ok';
    // Somebody or something decided against it, or it was stopped: nothing broke.
    case 'declined':
    case 'refused':
    case 'stopped':
    case 'notRun':
    case 'withheld':
      return 'held';
    default:
      return 'bad';
  }
}

function Mark({ tone }: { tone: Tone }) {
  if (tone === 'live') return <span className="moon" aria-hidden="true" />;
  if (tone === 'ok') return <Check size={14} aria-hidden="true" />;
  if (tone === 'held') return <CircleSlash size={13} aria-hidden="true" />;
  return <TriangleAlert size={13} aria-hidden="true" />;
}

/**
 * How many lines of what a tool gave back are shown at first: the last ones, because what
 * went wrong and how a command ended stand at the end. The rest is shown when it is asked for.
 */
const LINES_SHOWN = 80;

/** This moment, and while `moving` holds the moment of a look that is taken twice a second. */
function useNow(moving: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!moving) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [moving]);
  return now;
}

/**
 * One call of a tool, as a line: what it does, to what, and how it stands.
 * Opened, it shows what it was given and what came back.
 */
export function ToolRow({ item }: { item: ToolItem }) {
  const words = useWords();
  const args = argumentsOf(item);
  const Icon = icons[toolKind(item.name)];
  const object = objectOf(args);
  const outcome = classifyTool(item);
  const saying = sayTool(words, outcome);
  const tone = toneOf(outcome);
  const running = outcome.kind === 'running';
  const seconds = callSeconds(item, running, useNow(running));
  const kind = subagentKind(item.name, args);
  const edit = item.name === 'edit' && typeof args?.old === 'string' && typeof args?.new === 'string'
    ? { before: args.old, after: args.new } : null;
  const written = item.name === 'write' && typeof args?.content === 'string' ? args.content : null;
  // Whether the user has asked for all that came back.
  const [whole, setWhole] = useState(false);
  // What came back is taller than its place. The last lines are what is looked for first, so the
  // place is opened at its end. All of it, once that is asked for, is read from the beginning.
  const output = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (whole && output.current) output.current.scrollTop = 0;
  }, [whole]);
  const result = item.result === null ? null : tail(item.result, whole ? null : LINES_SHOWN);

  return <details
    className="tool"
    data-tone={tone}
    data-test="tool"
    onToggle={event => {
      const node = output.current;
      if (event.currentTarget.open && node && !whole) node.scrollTop = node.scrollHeight;
    }}
  >
    <summary>
      <Icon size={15} className="tool-icon" aria-hidden="true" />
      <span className="tool-what">
        <span className="tool-verb">{verbOf(words, item.name)}</span>
        {object && <code className="tool-object" title={object}>{object.split('\n')[0]}</code>}
        {kind && <span className="tool-kind" title={words.label.subagentKind} data-test="tool-kind">{kind}</span>}
      </span>
      <span className="tool-state">
        {seconds !== null && <span className="tool-time" data-test="tool-time">{words.duration(seconds)}</span>}
        {outcome.allowedByYou && <span className="tool-note"><Hand size={12} aria-hidden="true" />{words.tool.allowedByYou}</span>}
        {tone === 'ok'
          ? <span className="sr-only">{saying.what}</span>
          : <span className="tool-said">{saying.what}</span>}
        <Mark tone={tone} />
      </span>
      <ChevronRight size={14} className="tool-chevron" aria-hidden="true" />
    </summary>
    <div className="tool-more">
      {saying.why && <p className="tool-why">{saying.why}</p>}
      {edit ? <>
        <div className="overline">{words.label.before}</div>
        <pre className="block block-before" tabIndex={0}>{edit.before}</pre>
        <div className="overline">{words.label.after}</div>
        <pre className="block block-after" tabIndex={0}>{edit.after}</pre>
      </> : <>
        <div className="overline">{words.label.input}</div>
        <pre className="block" tabIndex={0}>{written ?? (commandTools.has(item.name) && object ? object
          : args ? JSON.stringify(args, null, 2) : item.arguments)}</pre>
      </>}
      {result !== null && <>
        <div className="overline">{words.label.output}</div>
        {result.cut > 0 && <button type="button" className="btn btn-quiet tool-all" onClick={() => setWhole(true)} data-test="tool-all">
          {words.label.showAllLines(result.cut + LINES_SHOWN)}
        </button>}
        {result.shown
          ? <pre className="block" ref={output} tabIndex={0} data-test="tool-output">{result.cut > 0 ? `…\n${result.shown}` : result.shown}</pre>
          : <p className="caption">{words.label.noOutput}</p>}
      </>}
      {outcome.code && <p className="tool-code mono">{outcome.code}{outcome.detail ? ` · ${outcome.detail}` : ''}</p>}
    </div>
  </details>;
}
