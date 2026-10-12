import {
  Check, ChevronRight, CircleSlash, FilePen, FileText, FolderSearch, Hand, ListChecks, Search,
  Sparkles, SquareTerminal, TriangleAlert, Users, Wrench, type LucideIcon,
} from 'lucide-react';
import { classifyTool, type ToolOutcome } from '../outcomes';
import type { ToolItem } from '../state';
import { sayTool } from '../words';
import { useWords } from './locale';
import { argumentsOf, commandTools, objectOf, tail, toolKind, verbOf, type ToolKind } from './tell';

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

/** How many lines of what a tool gave back are shown. The rest is in the log. */
const LINES_SHOWN = 80;

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
  const seconds = item.durationMs !== null && item.durationMs >= 1000 ? item.durationMs / 1000 : null;
  const edit = item.name === 'edit' && typeof args?.old === 'string' && typeof args?.new === 'string'
    ? { before: args.old, after: args.new } : null;
  const written = item.name === 'write' && typeof args?.content === 'string' ? args.content : null;
  const result = item.result === null ? null : tail(item.result, LINES_SHOWN);

  return <details className="tool" data-tone={tone} data-test="tool">
    <summary>
      <Icon size={15} className="tool-icon" aria-hidden="true" />
      <span className="tool-what">
        <span className="tool-verb">{verbOf(words, item.name)}</span>
        {object && <code className="tool-object" title={object}>{object.split('\n')[0]}</code>}
      </span>
      <span className="tool-state">
        {seconds !== null && <span className="tool-time">{words.duration(seconds)}</span>}
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
        {result.shown
          ? <pre className="block" tabIndex={0}>{result.cut > 0 ? `…\n${result.shown}` : result.shown}</pre>
          : <p className="caption">{words.label.noOutput}</p>}
      </>}
      {outcome.code && <p className="tool-code mono">{outcome.code}{outcome.detail ? ` · ${outcome.detail}` : ''}</p>}
    </div>
  </details>;
}
