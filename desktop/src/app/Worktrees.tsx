import { useEffect, useState } from 'react';
import { ChevronRight, GitBranch, GitMerge, Plus, Trash2 } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { useWords } from './locale';
import { failureOf, worktreesOf, type Worktree } from './work';

/** Whether two paths of folders are the same one, as the shell and git may write them. */
const same = (one: string, other: string) => {
  const plain = (path: string) => path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  return plain(one) === plain(other);
};

/**
 * The worktrees of the tasks of the project that is open, in the list of tasks: a task can be
 * begun in a worktree of its own, and the one that is done is merged into the project or removed.
 * The worktree that is open cannot be merged or removed from inside it. Without the shell, as in
 * a replay, and in a conversation of no project, there is nothing of it.
 */
export function Worktrees({ workspace }: { workspace: Workspace }) {
  const words = useWords();
  const said = words.label.work.worktrees;
  const told = words.hint.work.worktrees;
  const bridge = window.columbina?.worktrees ? window.columbina : undefined;
  const folder = workspace.chat ? null : workspace.folder;
  const [found, setFound] = useState<Worktree[]>([]);
  const [name, setName] = useState('');
  const [asking, setAsking] = useState<{ what: 'merge' | 'remove'; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  const asked = async (what: 'list' | 'add' | 'merge' | 'remove', named?: string): Promise<unknown> => {
    if (!bridge || folder === null) return null;
    setBusy(true);
    setFailed(null);
    try {
      const answer = await bridge.worktrees({ what, cwd: folder, name: named, force: what === 'remove' });
      if (what !== 'list') setFound(worktreesOf(await bridge.worktrees({ what: 'list', cwd: folder })));
      else setFound(worktreesOf(answer));
      return answer;
    } catch (problem) {
      // A folder that is in no repository of git has no worktrees, and that is no trouble to tell of.
      if (what !== 'list') setFailed(`${told.failed} ${failureOf(problem)}`);
      return null;
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => { void asked('list'); }, [folder]);
  if (!bridge || folder === null) return null;

  const make = async () => {
    const task = name.trim();
    if (task === '') return;
    const made = await asked('add', task) as { path?: unknown } | null;
    if (made && typeof made.path === 'string') {
      setName('');
      await workspace.openFolder(made.path);
    }
  };
  // Another folder is opened by starting the service again there, which a turn that runs would not survive.
  const busyAtAll = busy || workspace.loading || !workspace.connected || workspace.status?.status === 'running';

  return <details className="worktrees" data-test="worktrees">
    <summary className="overline sidebar-title"><ChevronRight size={12} aria-hidden="true" />{said.title}</summary>
    <p className="caption">{told.about}</p>
    <form className="worktree-new" onSubmit={event => { event.preventDefault(); void make(); }}>
      <input
        className="field"
        value={name}
        maxLength={100}
        placeholder={said.name}
        aria-label={said.name}
        onChange={event => setName(event.target.value)}
        data-test="worktree-name"
      />
      <button type="submit" className="icon-btn" disabled={busyAtAll || name.trim() === ''} aria-label={said.new} title={said.new} data-test="worktree-make">
        <Plus size={15} />
      </button>
    </form>
    {found.map(one => {
      const open = same(one.path, folder);
      return <div className="worktree" key={one.path} aria-current={open || undefined} data-test="worktree">
        <button type="button" className="task" disabled={open || busyAtAll} title={one.branch} onClick={() => void workspace.openFolder(one.path)} data-test="worktree-open">
          <GitBranch size={14} aria-hidden="true" /><span className="task-name">{one.name}</span>
        </button>
        {!open && <>
          <button type="button" className="icon-btn" disabled={busyAtAll} aria-label={said.merge} title={said.merge} onClick={() => setAsking({ what: 'merge', name: one.name })} data-test="worktree-merge">
            <GitMerge size={14} />
          </button>
          <button type="button" className="icon-btn" disabled={busyAtAll} aria-label={said.remove} title={said.remove} onClick={() => setAsking({ what: 'remove', name: one.name })} data-test="worktree-remove">
            <Trash2 size={14} />
          </button>
        </>}
      </div>;
    })}
    {asking && <div className="turn-ask" role="group" data-test="worktree-ask">
      <p className="caption">{asking.what === 'merge' ? told.mergeAsk(asking.name) : told.removeAsk(asking.name)}</p>
      <button type="button" className="btn" disabled={busyAtAll} onClick={() => { const { what, name: named } = asking; setAsking(null); void asked(what, named); }} data-test="worktree-yes">
        {asking.what === 'merge' ? said.merge : said.remove}
      </button>
      <button type="button" className="btn btn-quiet" onClick={() => setAsking(null)}>{words.label.work.keep}</button>
    </div>}
    {failed && <p className="caption" data-tone="warn" role="alert" data-test="worktrees-failed">{failed}</p>}
  </details>;
}
