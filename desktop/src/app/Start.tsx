import { useCallback, useEffect, useState, type RefObject } from 'react';
import { Check, ExternalLink, FolderOpen, MessageCircle, ShieldAlert, ShieldCheck, Sparkles } from 'lucide-react';
import { useRefreshOnFocus, type GitHub } from '../useGitHub';
import type { Workspace } from '../useWorkspace';
import { sayPermissions } from '../words';
import { Avatar, GitHubSignIn } from './GitHubSignIn';
import { useNow } from './Sidebar';
import { firstStepsOf, gitHubStepOf, modelStepOf } from './firstSteps';
import { useWords } from './locale';
import { agoOf, skillsToShow, taskTitle } from './tell';

/** How many earlier tasks the first page offers. */
const RECENT = 4;

/** Give the keyboard to what is done next, where the button that had it is gone. */
function focusIfLost(element: HTMLElement | null | undefined) {
  if (element && (document.activeElement === null || document.activeElement === document.body)) element.focus();
}

/**
 * The page before any project is open: the steps that come before the work, a model service
 * with its key and GitHub if the user likes, then the project. Once all that is offered is
 * done, the steps stand in one line under the button that opens a project, and that line opens
 * them again. Whatever was set up elsewhere meanwhile, such as a key in a terminal or
 * repositories on GitHub, is asked for when the user comes back to the window.
 */
function FirstPage({ workspace, github, onOpenProject, onOpenRecent, onOpenChat, onSetUp, onOpenPage, firstButton }: {
  workspace: Workspace;
  github: GitHub;
  onOpenProject: () => void;
  onOpenRecent: (folder: string) => void;
  onOpenChat: () => void;
  onSetUp: (service: string) => void;
  onOpenPage: (url: string) => void;
  firstButton: RefObject<HTMLButtonElement | null>;
}) {
  const words = useWords();
  const model = modelStepOf(workspace.services, workspace.checkingServices);
  const signedIn = gitHubStepOf(github.account, github.skipped);
  const steps = firstStepsOf(model, signedIn);
  // Steps that were open stay open while the page is there: what was just done is still to be seen.
  const [open, setOpen] = useState(false);
  useEffect(() => { if (steps.kind === 'open') setOpen(true); }, [steps.kind]);
  const shown = steps.kind === 'open' || (steps.kind === 'done' && open);
  // The keyboard begins at the key where the service lacks one, and at the project otherwise.
  const keyFirst = shown && model.kind === 'missing';
  useEffect(() => { focusIfLost(firstButton.current); }, [keyFirst, shown, firstButton]);

  const { refreshServices, checkingServices } = workspace;
  const { refresh, checking } = github;
  const waitsForGitHub = signedIn.kind !== 'ready' || signedIn.repositories === null || signedIn.repositories.length === 0;
  const askAgain = useCallback(() => {
    if (!checkingServices) void refreshServices();
    if (waitsForGitHub && !checking) void refresh();
  }, [checkingServices, refreshServices, waitsForGitHub, checking, refresh]);
  useRefreshOnFocus(askAgain, steps.kind !== 'none');

  const modelReady = model.kind === 'ready';
  const openProject = (large: boolean) => <button
    type="button"
    ref={keyFirst ? undefined : firstButton}
    className={['btn', large || modelReady ? 'btn-main' : '', large ? 'btn-large' : ''].filter(Boolean).join(' ')}
    onClick={onOpenProject}
    data-test="open-project-start"
  >
    <FolderOpen size={large ? 16 : 14} aria-hidden="true" />{words.label.openProject}
  </button>;
  // A conversation that belongs to no project, for whoever only wants to talk.
  const justChat = <button
    type="button"
    className="btn btn-quiet start-chat"
    title={words.hint.justChat}
    onClick={onOpenChat}
    data-test="open-chat-start"
  >
    <MessageCircle size={14} aria-hidden="true" />{words.label.justChat}
  </button>;
  return <div className="start-head" data-test="start">
    <span className="crescent" aria-hidden="true" />
    <h2 className="display">{shown ? words.label.start.title : words.label.emptyNoProjectTitle}</h2>
    <p className="start-lead">{shown ? words.hint.start.lead : words.hint.emptyNoProject}</p>
    {!shown && openProject(true)}
    {!shown && justChat}
    {!shown && steps.kind === 'done' && <button
      type="button"
      className="start-may first-steps-line"
      aria-expanded="false"
      title={words.label.start.show}
      onClick={() => setOpen(true)}
      data-test="first-steps-line"
    >
      <Check size={13} aria-hidden="true" /><span>{steps.line}</span>
    </button>}
    {shown && (model.kind === 'ready' || model.kind === 'missing') && <ol className="first-steps" data-test="first-steps">
      <li className="step" data-done={modelReady ? '' : undefined} data-test="step-model">
        <span className="step-mark" aria-hidden="true">{modelReady ? <Check size={13} /> : 1}</span>
        <div className="step-body">
          <h3 className="step-title">{words.label.start.model}</h3>
          <p className="caption">
            {modelReady ? [model.title, model.model].filter(Boolean).join(' · ') : words.hint.start.noKey(model.title)}
          </p>
        </div>
        <button
          type="button"
          ref={keyFirst ? firstButton : undefined}
          className={modelReady ? 'btn btn-quiet' : 'btn btn-main'}
          onClick={() => onSetUp(model.service)}
          data-test="step-model-set-up"
        >
          {modelReady ? words.label.start.change : words.label.start.connect}
        </button>
      </li>
      {(signedIn.kind === 'offered' || signedIn.kind === 'ready') && <li
        className="step"
        data-done={signedIn.kind === 'ready' ? '' : undefined}
        data-test="step-github"
      >
        <span className="step-mark" aria-hidden="true">{signedIn.kind === 'ready' ? <Check size={13} /> : 2}</span>
        <div className="step-body">
          <h3 className="step-title">
            {words.label.account}<span className="step-optional">{words.label.start.optional}</span>
          </h3>
          {signedIn.kind === 'offered' && <>
            <p className="caption">{words.hint.start.github}</p>
            <GitHubSignIn
              github={github}
              onOpen={onOpenPage}
              lead={false}
              main={false}
              aside={<button type="button" className="btn btn-quiet" onClick={() => void github.skip(true)} data-test="step-github-skip">
                {words.label.start.skip}
              </button>}
            />
          </>}
          {signedIn.kind === 'ready' && <>
            <p className="step-person">
              <Avatar person={signedIn.person} size={20} />
              <span className="caption mono">@{signedIn.person.login}</span>
            </p>
            {signedIn.repositories !== null && signedIn.repositories.length === 0 && <>
              <p className="caption">{words.hint.github.noRepositories}</p>
              {signedIn.installUrl !== '' && <div className="setting-row">
                <button type="button" className="btn" onClick={() => onOpenPage(signedIn.installUrl)} data-test="step-github-choose">
                  <ExternalLink size={14} aria-hidden="true" />{words.label.github.chooseRepositories}
                </button>
              </div>}
            </>}
          </>}
        </div>
      </li>}
      <li className="step" data-test="step-project">
        <span className="step-mark" aria-hidden="true">{signedIn.kind === 'offered' || signedIn.kind === 'ready' ? 3 : 2}</span>
        <div className="step-body">
          <h3 className="step-title">{words.label.start.project}</h3>
          <p className="caption">{words.hint.emptyNoProject}</p>
        </div>
        {openProject(false)}
      </li>
    </ol>}
    {shown && justChat}
    {shown && steps.kind === 'done' && <button
      type="button"
      className="btn btn-quiet first-steps-hide"
      aria-expanded="true"
      onClick={() => setOpen(false)}
      data-test="first-steps-hide"
    >
      {words.label.start.hide}
    </button>}
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

/**
 * What stands above the field on the page of a task in which nothing was said
 * yet. Before any project is open, it is the whole page.
 */
export function StartHead({ workspace, github, onOpenProject, onOpenRecent, onOpenChat, onPermissions, onSetUp, onOpenPage, firstButton }: {
  workspace: Workspace;
  github: GitHub;
  onOpenProject: () => void;
  onOpenRecent: (folder: string) => void;
  /** Start a conversation that belongs to no project. */
  onOpenChat: () => void;
  /** Open the settings where what a task may do is chosen. */
  onPermissions: () => void;
  /** Open the window in which a service is set up, at this service. */
  onSetUp: (service: string) => void;
  /** Open a page of the web in the browser of the system. */
  onOpenPage: (url: string) => void;
  /**
   * The button that the keyboard begins at on this page: the one that opens a project, or the
   * one of the first step that is still to be done.
   */
  firstButton: RefObject<HTMLButtonElement | null>;
}) {
  const words = useWords();
  if (!workspace.hasProject) {
    return <FirstPage
      workspace={workspace}
      github={github}
      onOpenProject={onOpenProject}
      onOpenRecent={onOpenRecent}
      onOpenChat={onOpenChat}
      onSetUp={onSetUp}
      onOpenPage={onOpenPage}
      firstButton={firstButton}
    />;
  }
  const status = workspace.status;
  const full = status?.sandbox?.mode === 'danger-full-access';
  return <div className="start-head" data-test="start">
    <span className="crescent" aria-hidden="true" />
    <h2 className="display">{words.label.emptyTitle}</h2>
    {workspace.hello && (workspace.chat
      ? <p className="start-lead" data-test="start-chat-lead">{words.hint.justChat}</p>
      : <p className="start-lead mono" title={workspace.hello.cwd}>{workspace.hello.cwd}</p>)}
    {/* What the task that is about to begin may do, where it is decided whether to begin it. */}
    {status && !workspace.chat && <button type="button" className="start-may" data-tone={full ? 'warn' : undefined} onClick={onPermissions} data-test="start-may">
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
