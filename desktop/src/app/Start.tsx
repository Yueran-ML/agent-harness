import type { RefObject } from 'react';
import { FolderOpen, ShieldAlert, ShieldCheck, Sparkles } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { sayPermissions } from '../words';
import { useNow } from './Sidebar';
import { useWords } from './locale';
import { agoOf, skillsToShow, taskTitle } from './tell';

/** How many earlier tasks the first page offers. */
const RECENT = 4;

/**
 * What stands above the field on the page of a task in which nothing was said
 * yet. Before any project is open, it is the whole page.
 */
export function StartHead({ workspace, onOpenProject, onOpenRecent, onPermissions, openButton }: {
  workspace: Workspace;
  onOpenProject: () => void;
  onOpenRecent: (folder: string) => void;
  /** Open the settings where what a task may do is chosen. */
  onPermissions: () => void;
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
      {workspace.recent.length > 0 && <div className="recent recent-projects" data-test="recent-projects">
        <div className="overline">{words.label.recentProjects}</div>
        {workspace.recent.map(project => <button
          type="button"
          className="task"
          key={project.path}
          title={project.path}
          onClick={() => onOpenRecent(project.path)}
        >
          <span className="task-name">{project.name}</span>
          <span className="task-when mono">{project.path}</span>
        </button>)}
      </div>}
    </div>;
  }
  const status = workspace.status;
  const full = status?.sandbox?.mode === 'danger-full-access';
  return <div className="start-head" data-test="start">
    <span className="crescent" aria-hidden="true" />
    <h2 className="display">{words.label.emptyTitle}</h2>
    {workspace.hello && <p className="start-lead mono" title={workspace.hello.cwd}>{workspace.hello.cwd}</p>}
    {/* What the task that is about to begin may do, where it is decided whether to begin it. */}
    {status && <button type="button" className="start-may" data-tone={full ? 'warn' : undefined} onClick={onPermissions} data-test="start-may">
      {full ? <ShieldAlert size={13} aria-hidden="true" /> : <ShieldCheck size={13} aria-hidden="true" />}
      <span>{sayPermissions(words, status.sandbox?.mode ?? null, status.approval)}</span>
    </button>}
  </div>;
}

/**
 * The skills that there are for this task, below the field of an empty task. A click on one
 * that a person can hand over writes the command for it into the field.
 */
export function StartSkills({ workspace, onUse }: { workspace: Workspace; onUse: (name: string) => void }) {
  const words = useWords();
  const { shown, more } = skillsToShow(workspace.skills);
  if (shown.length === 0) return null;
  return <div className="recent start-skills" data-test="start-skills">
    <div className="overline">{words.label.skills}</div>
    {shown.map(skill => (skill.user_invocable
      ? <button type="button" className="task skill" key={skill.name} title={skill.description} onClick={() => onUse(skill.name)} data-test="skill">
          <Sparkles size={13} aria-hidden="true" />
          <span className="skill-name mono">{skill.name}</span>
          <span className="skill-for">{skill.description}</span>
        </button>
      : <div className="task skill" key={skill.name} title={skill.description} data-own="agent" data-test="skill">
          <Sparkles size={13} aria-hidden="true" />
          <span className="skill-name mono">{skill.name}</span>
          <span className="skill-for">{skill.description}</span>
          <span className="task-when">{words.label.skillOfTheAgent}</span>
        </div>))}
    {more > 0 && <p className="caption skills-more">{words.hint.moreSkills(more)}</p>}
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
