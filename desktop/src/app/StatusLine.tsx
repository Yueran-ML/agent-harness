import { ShieldAlert, ShieldCheck, TriangleAlert } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { sayConfinement, sayPermissions } from '../words';
import { useWords } from './locale';
import { contextPercent, standingOf } from './tell';

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
 * One line under the field: what the task may do, in a sentence, and where
 * the agent stands.
 */
export function StatusLine({ workspace, onPermissions, onDetails }: {
  workspace: Workspace;
  onPermissions: () => void;
  onDetails: () => void;
}) {
  const words = useWords();
  const { status } = workspace;
  const full = status?.sandbox?.mode === 'danger-full-access';
  const confinement = status?.sandbox ? sayConfinement(words, status.sandbox.strength) : null;
  const loose = status?.sandbox?.strength === 'none' || status?.sandbox?.strength === 'partial';
  const percent = contextPercent(workspace.projection.lastInputTokens, status?.model.context_window);

  const { words: stands, tone } = standingOf(words, {
    status: status?.status, cancelling: workspace.cancelling, loading: workspace.loading,
    greeted: workspace.hello !== null, connected: workspace.connected,
  });

  // Under full access the whole line is in the colour of a warning, not its sentence alone.
  return <footer className="status" data-tone={full ? 'warn' : undefined} data-test="status">
    {status && <button type="button" className="status-may" onClick={onPermissions} data-test="status-may">
      {full ? <ShieldAlert size={13} aria-hidden="true" /> : <ShieldCheck size={13} aria-hidden="true" />}
      <span>{sayPermissions(words, status.sandbox?.mode ?? null, status.approval)}</span>
    </button>}
    {confinement && loose && !full && <button type="button" className="status-loose" onClick={onPermissions} title={confinement} aria-label={confinement}>
      <TriangleAlert size={13} />
    </button>}
    <span className="status-gap" />
    {workspace.mode === 'mock' && workspace.connected && <span className="status-replay" title={words.hint.replay}>{words.label.replay}</span>}
    {workspace.hasProject && <span className="status-stands" data-tone={tone} data-test="status-stands" role="status">
      <span className="dot" aria-hidden="true" />{stands}
    </span>}
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
