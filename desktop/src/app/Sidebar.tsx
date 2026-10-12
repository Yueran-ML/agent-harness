import { useEffect, useState } from 'react';
import { Folder, PanelLeftClose, Search, Settings2, SquarePen } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { useWords } from './locale';
import { agoOf, folderName, taskTitle } from './tell';

/** The time, told anew every minute. */
export function useNow(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

/** The tasks of the project, and the ways to a new task, another project and the settings. */
export function Sidebar({ workspace, title, onNewTask, onOpenTask, onOpenProject, onSettings, onHide }: {
  workspace: Workspace;
  /** What the task that is open is called. */
  title: string;
  onNewTask: () => void;
  onOpenTask: (id: string) => void;
  onOpenProject: () => void;
  onSettings: () => void;
  onHide: () => void;
}) {
  const words = useWords();
  const now = useNow();
  const [search, setSearch] = useState('');
  const { sessions, sessionId, hello } = workspace;
  const running = workspace.status?.status === 'running';
  // While the log of a long task is read, another task can be opened in its place.
  const busy = (workspace.loading && workspace.reading === null) || !workspace.connected;

  const wanted = search.trim().toLowerCase();
  const shown = sessions.filter(session => (session.first_input ?? '').toLowerCase().includes(wanted));
  // A task that nothing was said in yet is in no list of the service.
  const fresh = sessionId !== null && !sessions.some(session => session.id === sessionId);

  return <aside className="sidebar" aria-label={words.label.tasks} data-test="sidebar">
    <div className="sidebar-head">
      <button type="button" className="btn new-task" disabled={busy} onClick={onNewTask} data-test="new-task">
        <SquarePen size={15} aria-hidden="true" />{words.label.newTask}
      </button>
      <button type="button" className="icon-btn" onClick={onHide} aria-label={words.label.hideTasks} title={`${words.label.hideTasks} (Ctrl+B)`}>
        <PanelLeftClose size={16} />
      </button>
    </div>

    <label className="search">
      <Search size={14} aria-hidden="true" />
      <input
        type="search"
        value={search}
        onChange={event => setSearch(event.target.value)}
        placeholder={words.label.searchTasks}
        aria-label={words.label.searchTasks}
        spellCheck={false}
      />
    </label>

    <div className="overline sidebar-title">{words.label.tasks}</div>
    <nav className="tasks" aria-label={words.label.tasks}>
      {fresh && !wanted && <div className="task" aria-current="true" title={title}>
        <span className="task-name">{title}</span>
        {running && <span className="moon" role="img" aria-label={words.label.status.working} />}
      </div>}
      {shown.map(session => {
        const current = session.id === sessionId;
        const title = taskTitle(words, session.first_input);
        return <button
          type="button"
          className="task"
          key={session.id}
          aria-current={current || undefined}
          disabled={busy}
          title={title}
          onClick={() => { if (!current) onOpenTask(session.id); }}
          data-test="task"
        >
          <span className="task-name">{title}</span>
          {current && running
            ? <span className="moon" role="img" aria-label={words.label.status.working} />
            : <span className="task-when">{agoOf(words, session.modified_ms, now)}</span>}
        </button>;
      })}
      {shown.length === 0 && !(fresh && !wanted) && <p className="caption tasks-none">
        {wanted ? words.hint.noMatch : words.hint.noTasks}
      </p>}
    </nav>

    <div className="sidebar-foot">
      <button type="button" className="foot-row" onClick={onOpenProject} title={hello?.cwd ?? words.label.otherProject} aria-label={words.label.otherProject} data-test="open-project">
        <Folder size={15} aria-hidden="true" />
        <span>{hello ? folderName(hello.cwd) : words.label.openProject}</span>
      </button>
      <button type="button" className="foot-row" onClick={onSettings} data-test="open-settings">
        <Settings2 size={15} aria-hidden="true" />
        <span>{words.label.settings}</span>
      </button>
    </div>
  </aside>;
}
