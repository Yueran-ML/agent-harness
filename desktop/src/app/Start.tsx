import type { RefObject } from 'react';
import { FolderOpen } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { useNow } from './Sidebar';
import { useWords } from './locale';
import { agoOf, taskTitle } from './tell';

/** How many earlier tasks the first page offers. */
const RECENT = 4;

/**
 * What stands above the field on the page of a task in which nothing was said
 * yet. Before any project is open, it is the whole page.
 */
export function StartHead({ workspace, onOpenProject, openButton }: {
  workspace: Workspace;
  onOpenProject: () => void;
  openButton: RefObject<HTMLButtonElement | null>;
}) {
  const words = useWords();
  if (!workspace.hasProject) {
    return <div className="start-head" data-test="start">
      <span className="crescent" aria-hidden="true" />
      <h2 className="display">{words.label.emptyNoProjectTitle}</h2>
      <p className="start-lead">{words.hint.emptyNoProject}</p>
      <button type="button" ref={openButton} className="btn btn-main btn-large" onClick={onOpenProject} data-test="open-project-start">
        <FolderOpen size={16} aria-hidden="true" />{words.label.openProject}
      </button>
    </div>;
  }
  return <div className="start-head" data-test="start">
    <span className="crescent" aria-hidden="true" />
    <h2 className="display">{words.label.emptyTitle}</h2>
    {workspace.hello && <p className="start-lead mono" title={workspace.hello.cwd}>{workspace.hello.cwd}</p>}
  </div>;
}

/** The tasks that were worked on last, below the field of an empty task. */
export function Recent({ workspace, onOpenTask }: { workspace: Workspace; onOpenTask: (id: string) => void }) {
  const words = useWords();
  const now = useNow();
  const recent = workspace.sessions.filter(session => session.id !== workspace.sessionId).slice(0, RECENT);
  if (recent.length === 0) return null;
  return <div className="recent" data-test="recent">
    <div className="overline">{words.label.recentTasks}</div>
    {recent.map(session => <button type="button" className="task" key={session.id} onClick={() => onOpenTask(session.id)}>
      <span className="task-name">{taskTitle(words, session.first_input)}</span>
      <span className="task-when">{agoOf(words, session.modified_ms, now)}</span>
    </button>)}
  </div>;
}
