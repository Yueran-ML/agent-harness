// @vitest-environment jsdom
/**
 * The first steps on the page before any project is open: a model service with its key, GitHub
 * if the user likes, and the project, with a workspace and a sign-in that the test holds in its hand.
 */
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StartHead } from '../src/app/Start';
import type { Account } from '../src/app/github';
import type { Listed } from '../src/key/form';
import type { GitHub } from '../src/useGitHub';
import type { Workspace } from '../src/useWorkspace';
import { gitHubWith, INSTALL_URL, likeABrowser, service, workspaceWith } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const title = () => one('start').querySelector('h2')?.textContent;
const marks = () => [...document.querySelectorAll('.step-mark')].map(mark => (mark.querySelector('svg') ? '✓' : mark.textContent));

const deepseek = (key: string) => ({ ...service('deepseek', 'DeepSeek', { key, key_env: 'DEEPSEEK_API_KEY' }), model: 'deepseek-flash' });
const listed = (key = 'none'): Listed => ({ use: null, services: [deepseek(key), service('openai', 'OpenAI')] });
const nobody: Account = { kind: 'nobody', installUrl: INSTALL_URL, ended: null };
const signedIn = (repositories: string[] | null = ['octocat/hello']): Account => ({
  kind: 'signedIn', person: { login: 'octocat', name: 'The Octocat', avatar: null }, repositories, offline: null, installUrl: INSTALL_URL,
});
/** The page before any project is open, with whatever the test says of the services. */
const before = (given: Partial<Workspace> = {}) =>
  workspaceWith({ hasProject: false, hello: null, status: null, sessionId: null, ...given });

function page(workspace: Workspace, github: GitHub = gitHubWith()) {
  const calls = { onOpenProject: vi.fn(), onOpenRecent: vi.fn(), onPermissions: vi.fn(), onSetUp: vi.fn(), onOpenPage: vi.fn() };
  const firstButton = { current: null as HTMLButtonElement | null };
  const drawn = render(<StartHead workspace={workspace} github={github} {...calls} firstButton={firstButton} />);
  return {
    calls,
    firstButton,
    again: (next: Workspace, nextGitHub: GitHub = github) =>
      drawn.rerender(<StartHead workspace={next} github={nextGitHub} {...calls} firstButton={firstButton} />),
  };
}

describe('the first steps', () => {
  it('are not shown while it is not known where they stand: the page is the way to a project then', () => {
    for (const [workspace, github] of [
      [before({ services: null, checkingServices: true }), gitHubWith()],
      [before({ services: listed() }), gitHubWith({ account: { kind: 'unknown' }, checking: true })],
    ] as const) {
      cleanup();
      page(workspace, github);
      expect(title()).toBe('Open a project to begin');
      expect(one('open-project-start').className).toBe('btn btn-main btn-large');
      expect(marked('first-steps')).toEqual([]);
      expect(marked('first-steps-line')).toEqual([]);
    }
  });

  it('are not shown where nothing can be set up from here, and ask nothing when the user comes back', () => {
    const workspace = before({ services: null, checkingServices: false });
    const github = gitHubWith({ account: { kind: 'unavailable' } });
    page(workspace, github);
    expect(marked('first-steps')).toEqual([]);
    expect(marked('first-steps-line')).toEqual([]);
    window.dispatchEvent(new Event('focus'));
    expect(workspace.refreshServices).not.toHaveBeenCalled();
    expect(github.refresh).not.toHaveBeenCalled();
  });

  it('lead to a key for the service, to GitHub, and to a project, while the service lacks its key', () => {
    const github = gitHubWith();
    const { calls, firstButton } = page(before({ services: listed() }), github);
    expect(title()).toBe('Getting started');
    expect(one('start').querySelector('.start-lead')?.textContent).toBe('A few steps, and Columbina is ready to work.');
    expect(marks()).toEqual(['1', '2', '3']);
    expect(one('step-model').querySelector('p')?.textContent).toBe('No key is set up for DeepSeek yet.');
    const connect = one('step-model-set-up');
    expect([connect.textContent, connect.className]).toEqual(['Connect', 'btn btn-main']);
    // The key is what is done first, so the keyboard begins there, also when the page gives it again.
    expect(document.activeElement).toBe(connect);
    expect(firstButton.current).toBe(connect);
    fireEvent.click(connect);
    expect(calls.onSetUp.mock.calls).toEqual([['deepseek']]);
    // GitHub is offered, and can wait.
    expect(one('step-github').querySelector('.step-title')?.textContent).toBe('GitHub accountOptional');
    // Signing in is one choice among others here, and not the main thing to do.
    expect(one('github-sign-in').className).toBe('btn');
    fireEvent.click(one('github-sign-in'));
    expect(github.begin).toHaveBeenCalledTimes(1);
    fireEvent.click(one('step-github-skip'));
    expect(vi.mocked(github.skip).mock.calls).toEqual([[true]]);
    // A project can be opened all the same; it is not the main thing to do before there is a key.
    const open = one('open-project-start');
    expect(open.className).toBe('btn');
    fireEvent.click(open);
    expect(calls.onOpenProject).toHaveBeenCalledTimes(1);
    expect(marked('first-steps-hide')).toEqual([]);
  });

  it('show the code of a sign-in in the step of GitHub, without the way to put it off', () => {
    page(before({ services: listed('stored') }), gitHubWith({
      signIn: { kind: 'code', code: 'WDJB-MJHT', url: 'https://github.com/login/device', expiresAt: Date.now() + 900_000 },
    }));
    expect(one('step-github').querySelector('[data-test="github-code-value"]')?.textContent).toBe('WDJB-MJHT');
    expect(marked('step-github-skip')).toEqual([]);
  });

  it('stand in one line under the button that opens a project, once all that is offered is done', () => {
    const { firstButton } = page(before({ services: listed('stored') }), gitHubWith({ account: signedIn() }));
    expect(title()).toBe('Open a project to begin');
    expect(firstButton.current).toBe(one('open-project-start'));
    expect(one('open-project-start').className).toBe('btn btn-main btn-large');
    const line = one('first-steps-line');
    expect(line.textContent).toBe('DeepSeek · deepseek-flash · @octocat');
    expect([line.getAttribute('aria-expanded'), line.title]).toEqual(['false', 'Show the first steps']);
    expect(marked('first-steps')).toEqual([]);
  });

  it('open from that line, and go back into it', () => {
    const { calls, firstButton } = page(before({ services: listed('stored') }), gitHubWith({ account: { kind: 'unregistered', why: '' } }));
    expect(one('first-steps-line').textContent).toBe('DeepSeek · deepseek-flash');
    fireEvent.click(one('first-steps-line'));
    expect(title()).toBe('Getting started');
    // GitHub cannot be signed in to with this build, so it is not among the steps.
    expect(marks()).toEqual(['✓', '2']);
    expect(marked('step-github')).toEqual([]);
    expect(one('step-model').querySelector('p')?.textContent).toBe('DeepSeek · deepseek-flash');
    const change = one('step-model-set-up');
    expect([change.textContent, change.className]).toEqual(['Change', 'btn btn-quiet']);
    fireEvent.click(change);
    expect(calls.onSetUp.mock.calls).toEqual([['deepseek']]);
    expect(one('open-project-start').className).toBe('btn btn-main');
    expect(firstButton.current).toBe(one('open-project-start'));
    expect(one('first-steps-hide').getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(one('first-steps-hide'));
    expect(marked('first-steps')).toEqual([]);
    expect(one('first-steps-line').textContent).toBe('DeepSeek · deepseek-flash');
  });

  it('stay open once they were open, as the user does what they ask', () => {
    const github = gitHubWith();
    const { again } = page(before({ services: listed() }), github);
    expect(marked('first-steps')).toHaveLength(1);
    again(before({ services: listed('stored') }), gitHubWith({ account: signedIn() }));
    expect(marks()).toEqual(['✓', '✓', '3']);
    expect(one('step-github').querySelector('.caption.mono')?.textContent).toBe('@octocat');
    // Now that all of it is done, the steps can be put away.
    expect(marked('first-steps-hide')).toHaveLength(1);
  });

  it('come back where the key was lost meanwhile', () => {
    const github = gitHubWith({ account: signedIn() });
    const { again } = page(before({ services: listed('stored') }), github);
    expect(marked('first-steps')).toEqual([]);
    again(before({ services: listed('none') }));
    expect(marked('first-steps')).toHaveLength(1);
    expect(one('step-model-set-up').textContent).toBe('Connect');
  });

  it('lead to choosing repositories where the user is signed in and has chosen none', () => {
    const { calls } = page(before({ services: listed() }), gitHubWith({ account: signedIn([]) }));
    expect(one('step-github').textContent).toContain('Columbina reaches only the repositories that you choose for it on GitHub');
    fireEvent.click(one('step-github-choose'));
    expect(calls.onOpenPage.mock.calls).toEqual([[INSTALL_URL]]);
    cleanup();
    page(before({ services: listed() }), gitHubWith({ account: { ...signedIn([]), installUrl: '' } as Account }));
    expect(marked('step-github-choose')).toEqual([]);
  });

  it('leave GitHub out where the user said that it can wait', () => {
    page(before({ services: listed() }), gitHubWith({ skipped: true }));
    expect(marks()).toEqual(['1', '2']);
    expect(marked('step-github')).toEqual([]);
    cleanup();
    page(before({ services: listed('stored') }), gitHubWith({ skipped: true }));
    expect(one('first-steps-line').textContent).toBe('DeepSeek · deepseek-flash');
  });

  it('ask again what is set up when the user comes back to the window', () => {
    const workspace = before({ services: listed() });
    const github = gitHubWith();
    page(workspace, github);
    window.dispatchEvent(new Event('focus'));
    expect(workspace.refreshServices).toHaveBeenCalledTimes(1);
    expect(github.refresh).toHaveBeenCalledTimes(1);
  });

  it('ask GitHub nothing when the user comes back, where its step is done, and nothing that is being asked', () => {
    const workspace = before({ services: listed('stored'), checkingServices: true });
    const github = gitHubWith({ account: signedIn() });
    page(workspace, github);
    window.dispatchEvent(new Event('focus'));
    expect(workspace.refreshServices).not.toHaveBeenCalled();
    expect(github.refresh).not.toHaveBeenCalled();
    cleanup();
    const asking = gitHubWith({ checking: true });
    page(before({ services: listed() }), asking);
    window.dispatchEvent(new Event('focus'));
    expect(asking.refresh).not.toHaveBeenCalled();
  });

  it('give the keyboard to the button that opens a project where the key is there, and leave it where the user took it', () => {
    page(before({ services: listed('stored') }), gitHubWith());
    expect(document.activeElement).toBe(one('open-project-start'));
    cleanup();
    const elsewhere = document.createElement('input');
    document.body.append(elsewhere);
    try {
      elsewhere.focus();
      page(before({ services: listed() }), gitHubWith());
      expect(document.activeElement).toBe(elsewhere);
    } finally {
      elsewhere.remove();
    }
  });
});
