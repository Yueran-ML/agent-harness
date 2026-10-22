import { useMemo } from 'react';
import { Activity, Coins, ShieldAlert, ShieldCheck, TriangleAlert } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { sayConfinement, sayPermissions } from '../words';
import { useWords } from './locale';
import { contextPercent, countOf, standingOf } from './tell';
import { speedOf } from './trace';

/** A ring that fills with the context. */
function Ring({ percent }: { percent: number }) {
  const round = 2 * Math.PI * 6;
  return <svg className="ring" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
    <circle cx="8" cy="8" r="6" className="ring-track" />
    <circle
      cx="8" cy="8" r="6" className="ring-fill"
      strokeDasharray={`${round * percent / 100} ${round}`}
      transform="rotate(-90 8 8)"
    />
  </svg>;
}

/**
 * How far the task has come and what it took: its turns and steps, how fast the model wrote,
 * and the tokens of all its requests, with how many of those that were read came from the cache.
 * The whole of it is said when it is pointed at, and it leads to the details.
 */
function Stats({ workspace, locale, onDetails }: { workspace: Workspace; locale: string; onDetails: () => void }) {
  const words = useWords();
  const { projection } = workspace;
  const speed = useMemo(() => speedOf(workspace.events), [workspace.events]);
  const turns = projection.turns.length;
  if (turns === 0) return null;
  const steps = projection.turns.reduce((sum, turn) => sum + turn.steps, 0);
  const { usage } = projection;
  const read = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
  // To a tenth, and never more than it is: what was nearly all read from the cache is not said to be all.
  const cached = read > 0 ? Math.floor(usage.cache_read_tokens / read * 1000) / 10 : null;
  const whole = (count: number) => Intl.NumberFormat(locale, { maximumFractionDigits: 0 }).format(count);
  const tenths = (count: number) => Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(count);
  const run = [
    words.label.stats.turns(turns), words.label.stats.steps(steps),
    ...(speed === null ? [] : [words.label.stats.speed(whole(speed))]),
  ].join(' · ');
  const spent = [
    words.label.stats.tokens(countOf(read + usage.output_tokens, locale)),
    ...(cached === null ? [] : [words.label.stats.cached(tenths(cached))]),
  ].join(' · ');
  const told = words.hint.stats({
    turns, steps, speed: speed === null ? null : whole(speed), tokens: whole(read + usage.output_tokens),
    read: whole(read), cached: read > 0 ? whole(usage.cache_read_tokens) : null, written: whole(usage.output_tokens),
  });
  return <button type="button" className="status-stats" onClick={onDetails} title={told} aria-label={told} data-test="status-stats">
    <span className="status-stats-run"><Activity size={13} aria-hidden="true" />{run}</span>
    <span className="status-stats-spent"><Coins size={13} aria-hidden="true" />{spent}</span>
  </button>;
}

/**
 * One line under the field: what the task may do, in a sentence, where
 * the agent stands, and how far the task has come.
 */
export function StatusLine({ workspace, locale, onPermissions, onDetails }: {
  workspace: Workspace;
  /** The language of the page, for numbers. */
  locale: string;
  onPermissions: () => void;
  onDetails: () => void;
}) {
  const words = useWords();
  const { status } = workspace;
  // A chat changes no file and runs no command, whatever its preset says, and nothing is to be confined there.
  const chat = workspace.chat;
  const full = !chat && status?.sandbox?.mode === 'danger-full-access';
  const confinement = status?.sandbox ? sayConfinement(words, status.sandbox.strength) : null;
  const loose = !chat && (status?.sandbox?.strength === 'none' || status?.sandbox?.strength === 'partial');
  const percent = contextPercent(workspace.projection.lastInputTokens, status?.model.context_window);

  const { words: stands, tone } = standingOf(words, {
    status: status?.status, cancelling: workspace.cancelling, loading: workspace.loading,
    greeted: workspace.hello !== null, connected: workspace.connected,
  });

  // Under full access the whole line is in the colour of a warning, not its sentence alone.
  return <footer className="status" data-tone={full ? 'warn' : undefined} data-test="status">
    {status && <button type="button" className="status-may" onClick={onPermissions} data-test="status-may">
      {full ? <ShieldAlert size={13} aria-hidden="true" /> : <ShieldCheck size={13} aria-hidden="true" />}
      <span>{chat ? words.permissions.chat : sayPermissions(words, status.sandbox?.mode ?? null, status.approval)}</span>
    </button>}
    {confinement && loose && !full && <button type="button" className="status-loose" onClick={onPermissions} title={confinement} aria-label={confinement}>
      <TriangleAlert size={13} />
    </button>}
    <span className="status-gap" />
    {workspace.mode === 'mock' && workspace.connected && <span className="status-replay" title={words.hint.replay}>{words.label.replay}</span>}
    {workspace.hasProject && <span className="status-stands" data-tone={tone} data-test="status-stands" role="status">
      <span className="dot" aria-hidden="true" />{stands}
    </span>}
    {workspace.hasProject && <Stats workspace={workspace} locale={locale} onDetails={onDetails} />}
    {percent !== null && <button
      type="button"
      className="status-context"
      data-tone={percent >= 95 ? 'bad' : percent >= 80 ? 'warn' : undefined}
      onClick={onDetails}
      data-test="status-context"
    >
      <Ring percent={percent} />{words.label.context(percent)}
    </button>}
  </footer>;
}
