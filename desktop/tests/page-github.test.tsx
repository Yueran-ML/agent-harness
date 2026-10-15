// @vitest-environment jsdom
/**
 * Signing in to GitHub on the page: what the page holds of it, against a shell that the test
 * plays, and the card and the part of the settings that show it. No token comes this way.
 */
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { GitHubAccount, GitHubSignIn } from '../src/app/GitHubSignIn';
import type { Account, SignIn } from '../src/app/github';
import { useGitHub, useRefreshOnFocus, type GitHub } from '../src/useGitHub';
import { gitHubWith, INSTALL_URL, likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, 'clipboard');
});

const PICTURE = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const REVOKE = 'https://github.com/settings/apps/authorizations';
const DEVICE = 'https://github.com/login/device';
const octocat = { login: 'octocat', name: 'The Octocat', avatar: PICTURE };
const signedIn = (given: Partial<Extract<Account, { kind: 'signedIn' }>> = {}): Account => ({
  kind: 'signedIn', person: octocat, repositories: ['octocat/hello'], offline: null, installUrl: INSTALL_URL, ...given,
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
/** The button that says these words, and no other. */
const button = (says: string) => {
  const found = [...document.querySelectorAll('button')].filter(each => each.textContent === says);
  if (found.length !== 1) throw new Error(`There is not exactly one button that says "${says}".`);
  return found[0];
};
const buttons = () => [...document.querySelectorAll('button')].map(each => each.textContent);
/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  let refuse!: (problem: Error) => void;
  const promise = new Promise<T>((resolve, reject) => { release = resolve; refuse = reject; });
  return { promise, release, refuse };
}

/** The shell as the page meets it, as far as GitHub goes, played by the test. */
function shellOfGitHub() {
  const listeners = new Set<(said: Record<string, unknown>) => void>();
  const shell = {
    githubStatus: vi.fn(async (): Promise<unknown> => ({ signed_in: null, install_url: INSTALL_URL, skipped: false })),
    githubSignIn: vi.fn(async () => {}),
    githubCancel: vi.fn(async () => {}),
    githubSignOut: vi.fn(async (): Promise<unknown> => ({ signed_out: true, revoke: REVOKE })),
    skipGitHub: vi.fn(async (_skipped: boolean) => {}),
    onGitHub: vi.fn((listener: (said: Record<string, unknown>) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    }),
  };
  vi.stubGlobal('columbina', shell);
  return {
    shell,
    /** Say what a sign-in says, line by line. */
    tell: (...said: Record<string, unknown>[]) => act(() => {
      for (const document of said) for (const listener of [...listeners]) listener(document);
    }),
    listening: () => listeners.size,
  };
}

/** Draw what the page holds, and let what it asked for when it was drawn arrive. */
async function drawn() {
  const hook = renderHook(() => useGitHub());
  await act(async () => {});
  return hook;
}

describe('what the page holds of GitHub', () => {
  it('has nothing to sign in to where there is no shell, as in a replay', async () => {
    const { result } = renderHook(() => useGitHub());
    expect(result.current.account).toEqual({ kind: 'unavailable' });
    await act(async () => {
      await result.current.refresh();
      await result.current.begin();
      expect(await result.current.signOut()).toBe(false);
      result.current.cancel();
    });
    expect(result.current.account).toEqual({ kind: 'unavailable' });
    expect(result.current.signIn).toEqual({ kind: 'idle' });
  });

  it('asks the shell who is signed in when it is drawn, and holds the answer', async () => {
    const { shell } = shellOfGitHub();
    const answer = held<unknown>();
    shell.githubStatus.mockImplementationOnce(() => answer.promise);
    const { result } = renderHook(() => useGitHub());
    expect(result.current.account).toEqual({ kind: 'unknown' });
    expect(result.current.checking).toBe(true);
    await act(async () => {
      answer.release({ signed_in: octocat, install_url: INSTALL_URL, repositories: ['octocat/hello'], skipped: true });
    });
    expect(result.current.account).toEqual(signedIn());
    expect(result.current.skipped).toBe(true);
    expect(result.current.checking).toBe(false);
    expect(shell.githubStatus).toHaveBeenCalledTimes(1);
  });

  it('says that it could not find out, where the shell could not ask', async () => {
    const { shell } = shellOfGitHub();
    shell.githubStatus.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    const { result } = await drawn();
    expect(result.current.account).toEqual({ kind: 'failed', code: 'UNAVAILABLE', said: 'The setup program did not end in time.' });
    expect(result.current.checking).toBe(false);
    shell.githubStatus.mockRejectedValueOnce('gone');
    await act(() => result.current.refresh());
    expect(result.current.account).toEqual({ kind: 'failed', code: 'UNAVAILABLE', said: 'gone' });
  });

  it('holds the answer to the last question it asked, whichever answer comes first', async () => {
    const { shell } = shellOfGitHub();
    const first = held<unknown>();
    const second = held<unknown>();
    const third = held<unknown>();
    shell.githubStatus.mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise)
      .mockImplementationOnce(() => third.promise);
    const { result } = renderHook(() => useGitHub());
    act(() => { void result.current.refresh(); });
    await act(async () => { second.release({ signed_in: null, install_url: INSTALL_URL }); });
    expect(result.current.account).toEqual({ kind: 'nobody', installUrl: INSTALL_URL, ended: null });
    expect(result.current.checking).toBe(false);
    // An earlier answer, or an earlier failure, changes nothing once a later one has come.
    await act(async () => { first.release({ signed_in: octocat, install_url: INSTALL_URL, repositories: [] }); });
    expect(result.current.account.kind).toBe('nobody');
    act(() => { void result.current.refresh(); });
    expect(result.current.checking).toBe(true);
    shell.githubStatus.mockImplementationOnce(async () => ({ signed_in: octocat, install_url: INSTALL_URL, repositories: [] }));
    await act(() => result.current.refresh());
    await act(async () => { third.refuse(new Error('too late')); });
    expect(result.current.account).toEqual(signedIn({ repositories: [] }));
    expect(result.current.checking).toBe(false);
  });

  it('follows a sign-in from the code to who signed in, before any answer that was asked for earlier', async () => {
    const { shell, tell } = shellOfGitHub();
    const late = held<unknown>();
    shell.githubStatus.mockImplementationOnce(() => late.promise);
    const { result } = renderHook(() => useGitHub());
    await act(() => result.current.begin());
    expect(shell.githubSignIn).toHaveBeenCalledTimes(1);
    expect(result.current.signIn).toEqual({ kind: 'asking' });
    const before = Date.now();
    tell({ code: 'WDJB-MJHT', url: DEVICE, expires_in: 900 });
    const code = result.current.signIn as Extract<SignIn, { kind: 'code' }>;
    expect([code.kind, code.code, code.url]).toEqual(['code', 'WDJB-MJHT', DEVICE]);
    expect(code.expiresAt).toBeGreaterThanOrEqual(before + 900_000);
    expect(code.expiresAt).toBeLessThanOrEqual(Date.now() + 900_000);
    tell({ signed_in: octocat, install_url: INSTALL_URL, repositories: ['octocat/hello'] }, { ended: true });
    expect(result.current.signIn).toEqual({ kind: 'done', person: octocat });
    expect(result.current.account).toEqual(signedIn());
    expect(result.current.checking).toBe(false);
    await act(async () => { late.release({ signed_in: null, install_url: INSTALL_URL }); });
    expect(result.current.account).toEqual(signedIn());
  });

  it('says why a sign-in could not be started', async () => {
    const { shell } = shellOfGitHub();
    const { result } = await drawn();
    shell.githubSignIn.mockRejectedValueOnce(new Error('The sign-in could not be started: spawn ENOENT'));
    await act(() => result.current.begin());
    expect(result.current.signIn).toEqual({
      kind: 'failed', code: 'NOT_STARTED', said: 'The sign-in could not be started: spawn ENOENT',
    });
  });

  it('stops a sign-in at once, and hears nothing more of it', async () => {
    const { shell, tell } = shellOfGitHub();
    const { result } = await drawn();
    await act(() => result.current.begin());
    tell({ code: 'WDJB-MJHT', url: DEVICE, expires_in: 900 });
    act(() => result.current.cancel());
    expect(result.current.signIn).toEqual({ kind: 'idle' });
    expect(shell.githubCancel).toHaveBeenCalledTimes(1);
    // A shell that cannot stop it leaves the page as it is.
    shell.githubCancel.mockRejectedValueOnce(new Error('gone'));
    await act(async () => { result.current.cancel(); });
    expect(result.current.signIn).toEqual({ kind: 'idle' });
  });

  it('signs out, and says where the right is taken back', async () => {
    const { shell } = shellOfGitHub();
    shell.githubStatus.mockResolvedValueOnce({ signed_in: octocat, install_url: INSTALL_URL, repositories: [] });
    const { result } = await drawn();
    let went = false;
    await act(async () => { went = await result.current.signOut(); });
    expect(went).toBe(true);
    expect(shell.githubSignOut).toHaveBeenCalledTimes(1);
    expect(result.current.account).toEqual({ kind: 'nobody', installUrl: INSTALL_URL, ended: null });
    expect(result.current.signedOut).toEqual({ revoke: REVOKE });
    expect(result.current.signIn).toEqual({ kind: 'idle' });
    // The next sign-in puts that away.
    await act(() => result.current.begin());
    expect(result.current.signedOut).toBeNull();
  });

  it('is done with a sign-in once the user has signed out after it', async () => {
    const { tell } = shellOfGitHub();
    const { result } = await drawn();
    await act(() => result.current.begin());
    tell({ signed_in: octocat, install_url: INSTALL_URL, repositories: [] }, { ended: true });
    expect(result.current.signIn).toEqual({ kind: 'done', person: octocat });
    await act(async () => { await result.current.signOut(); });
    expect(result.current.signIn).toEqual({ kind: 'idle' });
  });

  it('says that it signed out also where it was told no page to take the right back on', async () => {
    const { shell } = shellOfGitHub();
    shell.githubSignOut.mockResolvedValueOnce({ signed_out: true, revoke: 'ftp://github.com/settings' });
    const { result } = await drawn();
    await act(async () => { await result.current.signOut(); });
    expect(result.current.signedOut).toEqual({ revoke: null });
  });

  it('has no page to choose repositories on after a sign-out, where it had none before', async () => {
    const { shell } = shellOfGitHub();
    shell.githubStatus.mockResolvedValueOnce({ error: 'the tokens of GitHub cannot be used', code: 'STORE' });
    const { result } = await drawn();
    await act(async () => { await result.current.signOut(); });
    expect(result.current.account).toEqual({ kind: 'nobody', installUrl: '', ended: null });
  });

  it('says why a sign-out did not go through, and stays signed in', async () => {
    const { shell } = shellOfGitHub();
    shell.githubStatus.mockResolvedValueOnce({ signed_in: octocat, install_url: INSTALL_URL, repositories: ['octocat/hello'] });
    const { result } = await drawn();
    shell.githubSignOut.mockResolvedValueOnce({ error: 'github.json cannot be written', code: 'STORE' });
    let went = true;
    await act(async () => { went = await result.current.signOut(); });
    expect(went).toBe(false);
    expect(result.current.signOutFailed).toEqual({ code: 'STORE', said: 'github.json cannot be written' });
    expect(result.current.account).toEqual(signedIn());
    shell.githubSignOut.mockRejectedValueOnce(new Error('No service program is set up.'));
    await act(async () => { went = await result.current.signOut(); });
    expect(result.current.signOutFailed).toEqual({ code: 'UNAVAILABLE', said: 'No service program is set up.' });
    // A sign-out that goes through, and a sign-in, put it away.
    await act(async () => { went = await result.current.signOut(); });
    expect(went).toBe(true);
    expect(result.current.signOutFailed).toBeNull();
    shell.githubSignOut.mockResolvedValueOnce({ error: 'it broke', code: 'STORE' });
    await act(async () => { await result.current.signOut(); });
    await act(() => result.current.begin());
    expect(result.current.signOutFailed).toBeNull();
  });

  it('keeps whether GitHub is to wait, also where the shell could not keep it', async () => {
    const { shell } = shellOfGitHub();
    const { result } = await drawn();
    await act(() => result.current.skip(true));
    expect(result.current.skipped).toBe(true);
    expect(shell.skipGitHub.mock.calls).toEqual([[true]]);
    shell.skipGitHub.mockRejectedValueOnce(new Error('gone'));
    await act(() => result.current.skip(false));
    expect(result.current.skipped).toBe(false);
  });

  it('stops listening when it is put away', async () => {
    const { listening } = shellOfGitHub();
    const { unmount } = await drawn();
    expect(listening()).toBe(1);
    unmount();
    expect(listening()).toBe(0);
  });
});

describe('asking again when the window comes back to the front', () => {
  it('happens only while it is asked for', () => {
    const refresh = vi.fn();
    const { rerender, unmount } = renderHook(({ when }) => useRefreshOnFocus(refresh, when), { initialProps: { when: false } });
    window.dispatchEvent(new Event('focus'));
    expect(refresh).not.toHaveBeenCalled();
    rerender({ when: true });
    window.dispatchEvent(new Event('focus'));
    expect(refresh).toHaveBeenCalledTimes(1);
    rerender({ when: false });
    window.dispatchEvent(new Event('focus'));
    rerender({ when: true });
    unmount();
    window.dispatchEvent(new Event('focus'));
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});

describe('the card of a sign-in', () => {
  const open = vi.fn();
  beforeEach(() => open.mockClear());
  const card = (github: GitHub, lead = true) => render(<GitHubSignIn github={github} onOpen={open} lead={lead} />);
  const codeIn = (seconds: number): SignIn => ({ kind: 'code', code: 'WDJB-MJHT', url: DEVICE, expiresAt: Date.now() + seconds * 1000 });

  it('says what signing in allows, and begins with one button', () => {
    const github = gitHubWith();
    card(github);
    expect(document.querySelector('.github-lead')?.textContent).toMatch(/^Once you are signed in, Columbina can read repositories/);
    expect(button('Sign in with GitHub').className).toBe('btn btn-main');
    fireEvent.click(button('Sign in with GitHub'));
    expect(github.begin).toHaveBeenCalledTimes(1);
    cleanup();
    card(gitHubWith(), false);
    expect(document.querySelector('.github-lead')).toBeNull();
  });

  it('is one choice among others where it is asked to be, with what stands beside it', () => {
    const aside = <button type="button" data-test="later">Later</button>;
    render(<GitHubSignIn github={gitHubWith()} onOpen={open} main={false} aside={aside} />);
    expect(button('Sign in with GitHub').className).toBe('btn');
    expect(one('later').parentElement).toBe(one('github-sign-in').parentElement);
    cleanup();
    // After a failure it stands beside the new try. While a sign-in runs, there is nothing to put off.
    render(<GitHubSignIn github={gitHubWith({ signIn: { kind: 'failed', code: 'DENIED', said: '' } })} onOpen={open} aside={aside} />);
    expect(buttons()).toEqual(['Try again', 'Later']);
    cleanup();
    render(<GitHubSignIn github={gitHubWith({ signIn: { kind: 'asking' } })} onOpen={open} aside={aside} />);
    expect(marked('later')).toEqual([]);
    cleanup();
    // In a build that cannot sign in, it stands alone.
    render(<GitHubSignIn github={gitHubWith({ signIn: { kind: 'failed', code: 'NOT_REGISTERED', said: '' } })} onOpen={open} aside={aside} />);
    expect(buttons()).toEqual(['Later']);
  });

  it('says that it asks GitHub for a code, which can be stopped, and gives the keyboard to the stop', () => {
    const github = gitHubWith({ signIn: { kind: 'asking' } });
    card(github);
    expect(one('github-asking').querySelector('[role="status"]')?.textContent).toBe('Asking GitHub for a code.');
    expect(document.activeElement?.textContent).toBe('Cancel');
    fireEvent.click(button('Cancel'));
    expect(github.cancel).toHaveBeenCalledTimes(1);
    expect(marked('github-sign-in')).toEqual([]);
  });

  it('shows the code large, the page to enter it on, and how long it holds', () => {
    vi.useFakeTimers();
    const github = gitHubWith({ signIn: codeIn(900) });
    card(github);
    expect(one('github-code-value').textContent).toBe('WDJB-MJHT');
    expect(one('github-url').textContent).toBe(DEVICE);
    expect(one('github-left').textContent).toBe('15:00 left');
    expect(one('github-code').textContent).toContain('Columbina waits until you have confirmed it in the browser.');
    // The keyboard goes on with the code.
    expect(document.activeElement).toBe(one('github-copy'));
    act(() => { vi.advanceTimersByTime(1_000); });
    expect(one('github-left').textContent).toBe('14:59 left');
    act(() => { vi.advanceTimersByTime(838_000); });
    expect(one('github-left').textContent).toBe('1:01 left');
    fireEvent.click(one('github-open'));
    expect(open.mock.calls).toEqual([[DEVICE]]);
    fireEvent.click(button('Cancel'));
    expect(github.cancel).toHaveBeenCalledTimes(1);
  });

  it('copies the code, and says so for a moment', async () => {
    vi.useFakeTimers();
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    card(gitHubWith({ signIn: codeIn(900) }));
    await act(async () => { fireEvent.click(one('github-copy')); });
    expect(writeText.mock.calls).toEqual([['WDJB-MJHT']]);
    expect(one('github-copy').textContent).toBe('Copied');
    act(() => { vi.advanceTimersByTime(1_599); });
    expect(one('github-copy').textContent).toBe('Copied');
    act(() => { vi.advanceTimersByTime(1); });
    expect(one('github-copy').textContent).toBe('Copy the code');
  });

  it('leaves the code to be marked by hand where there is no clipboard to take it', async () => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn(async () => { throw new Error('denied'); }) } });
    card(gitHubWith({ signIn: codeIn(900) }));
    await act(async () => { fireEvent.click(one('github-copy')); });
    expect(one('github-copy').textContent).toBe('Copy the code');
  });

  it('says when the code has run out, and offers a new one', () => {
    vi.useFakeTimers();
    const github = gitHubWith({ signIn: codeIn(2) });
    card(github);
    act(() => { vi.advanceTimersByTime(1_000); });
    expect(one('github-left').textContent).toBe('0:01 left');
    act(() => { vi.advanceTimersByTime(1_000); });
    expect(one('github-expired').textContent).toBe('The code has run out.');
    expect(one('github-code').hasAttribute('data-expired')).toBe(true);
    // Nothing is offered that leads to entering a code that is no longer good.
    expect(marked('github-copy')).toEqual([]);
    expect(marked('github-open')).toEqual([]);
    expect(marked('github-left')).toEqual([]);
    expect(document.activeElement).toBe(one('github-new-code'));
    fireEvent.click(one('github-new-code'));
    expect(github.begin).toHaveBeenCalledTimes(1);
    // The clock stands still once it has run out: once React has drawn the last of it, nothing waits.
    act(() => { vi.advanceTimersByTime(5_000); });
    expect(vi.getTimerCount()).toBe(0);
  });

  it('leaves the keyboard where the user took it meanwhile', () => {
    const elsewhere = document.createElement('input');
    document.body.append(elsewhere);
    try {
      elsewhere.focus();
      const { rerender } = card(gitHubWith({ signIn: { kind: 'asking' } }));
      rerender(<GitHubSignIn github={gitHubWith({ signIn: codeIn(900) })} onOpen={open} />);
      expect(document.activeElement).toBe(elsewhere);
      rerender(<GitHubSignIn github={gitHubWith({ signIn: { kind: 'failed', code: 'DENIED', said: '' } })} onOpen={open} />);
      expect(document.activeElement).toBe(elsewhere);
    } finally {
      elsewhere.remove();
    }
  });

  it('says what went wrong in words of its own, with what the program said folded away', () => {
    const said = 'GitHub could not be reached: [Errno 11001] getaddrinfo failed. If GitHub is reached through a proxy, see that the proxy is on';
    const github = gitHubWith({ signIn: { kind: 'failed', code: 'UNREACHABLE', said } });
    card(github);
    expect(one('github-failed').querySelector('p')?.textContent)
      .toBe('GitHub could not be reached. See whether this computer is online, and that the proxy is on if you use one.');
    expect(one('github-failed').querySelector('details')?.hasAttribute('open')).toBe(false);
    expect(one('github-failed').querySelector('details .tool-code')?.textContent).toBe(said);
    expect(document.querySelector('.github-lead')).toBeNull();
    expect(document.activeElement?.textContent).toBe('Try again');
    fireEvent.click(button('Try again'));
    expect(github.begin).toHaveBeenCalledTimes(1);
  });

  it.each([
    ['TIMEOUT', 'GitHub did not answer in time. Try again in a moment.', 'Try again'],
    ['BUSY', 'GitHub is busy. Try again in a moment.', 'Try again'],
    ['EXPIRED', 'The code ran out before it was confirmed. Ask for a new one.', 'Ask for a new code'],
    ['DENIED', 'The sign-in was turned down on GitHub.', 'Try again'],
    ['RATE_LIMITED', 'GitHub asks Columbina to wait before it asks again. The details say for how long.', 'Try again'],
    ['STORE', 'The sign-in could not be kept on this computer.', 'Try again'],
    ['ENDED', 'The sign-in stopped before it was done.', 'Try again'],
    ['NOT_STARTED', 'The sign-in could not be started.', 'Try again'],
    ['FORBIDDEN', 'The sign-in did not go through.', 'Try again'],
  ])('says of %s what it is', (code, words, again) => {
    card(gitHubWith({ signIn: { kind: 'failed', code, said: '' } }));
    expect(one('github-failed').querySelector('p')?.textContent).toBe(words);
    // Where the program said nothing, there is nothing to fold away.
    expect(one('github-failed').querySelector('details')).toBeNull();
    expect(buttons()).toEqual([again]);
  });

  it('offers no new try in a build that cannot sign in', () => {
    for (const code of ['NOT_REGISTERED', 'MISCONFIGURED']) {
      cleanup();
      card(gitHubWith({ signIn: { kind: 'failed', code, said: 'GitHub does not know the client ID of this Columbina' } }));
      expect(one('github-failed').querySelector('p')?.textContent).toBe('This build of Columbina cannot sign in to GitHub yet.');
      expect(buttons()).toEqual([]);
    }
  });
});

describe('the account in the settings', () => {
  const open = vi.fn();
  beforeEach(() => open.mockClear());
  const account = (github: GitHub) => render(<GitHubAccount github={github} onOpen={open} />);

  it('says that it asks who is signed in, and nothing before it asks', () => {
    account(gitHubWith({ account: { kind: 'unknown' }, checking: true }));
    expect(document.querySelector('[role="status"]')?.textContent).toBe('Asking who is signed in to GitHub.');
    cleanup();
    const { container } = account(gitHubWith({ account: { kind: 'unknown' } }));
    expect(container.textContent).toBe('');
  });

  it('says where signing in works, in a replay', () => {
    const { container } = account(gitHubWith({ account: { kind: 'unavailable' } }));
    expect(container.textContent).toBe('Signing in to GitHub works where the application runs the service itself, and not in a replay.');
    expect(buttons()).toEqual([]);
  });

  it('says that this build cannot sign in, with what the program said folded away', () => {
    const why = 'this build of Columbina has no GitHub App to sign in with';
    const { container } = account(gitHubWith({ account: { kind: 'unregistered', why } }));
    expect(container.querySelector('p')?.textContent).toBe('This build of Columbina cannot sign in to GitHub yet.');
    expect(container.querySelector('details .tool-code')?.textContent).toBe(why);
    expect(buttons()).toEqual([]);
  });

  it('says what could not be read, and asks again', () => {
    const github = gitHubWith({ account: { kind: 'failed', code: 'STORE', said: 'the tokens of GitHub cannot be used' } });
    account(github);
    expect(one('github-status-failed').querySelector('p')?.textContent)
      .toBe('What this computer keeps of the sign-in to GitHub cannot be read.');
    fireEvent.click(one('github-check-again'));
    expect(github.refresh).toHaveBeenCalledTimes(1);
    cleanup();
    account(gitHubWith({ account: { kind: 'failed', code: 'UNAVAILABLE', said: 'gone' }, checking: true }));
    expect(one('github-status-failed').querySelector('p')?.textContent).toBe('Columbina could not find out who is signed in to GitHub.');
    // While it asks, it is not asked again.
    expect((one('github-check-again') as HTMLButtonElement).disabled).toBe(true);
  });

  it('offers to sign in where nobody is, and says so where a sign-in ran out', () => {
    account(gitHubWith());
    expect(marked('github-sign-in')).toHaveLength(1);
    expect(marked('github-ended')).toEqual([]);
    cleanup();
    account(gitHubWith({ account: { kind: 'nobody', installUrl: INSTALL_URL, ended: 'the sign-in to GitHub has run out' } }));
    expect(one('github-ended').textContent).toBe('The sign-in to GitHub has run out. Sign in again.');
    cleanup();
    // While a new one runs, that is past.
    account(gitHubWith({
      account: { kind: 'nobody', installUrl: INSTALL_URL, ended: 'the sign-in to GitHub has run out' }, signIn: { kind: 'asking' },
    }));
    expect(marked('github-ended')).toEqual([]);
  });

  it('says after a sign-out where the right that was given is taken back', () => {
    account(gitHubWith({ signedOut: { revoke: REVOKE } }));
    expect(one('github-signed-out').querySelector('p')?.textContent)
      .toBe('Signed out on this computer. The app keeps the right that you gave it until you take it back on GitHub.');
    fireEvent.click(one('github-revoke'));
    expect(open.mock.calls).toEqual([[REVOKE]]);
    expect(marked('github-sign-in')).toHaveLength(1);
    cleanup();
    // Without a page to go to, it still says that the sign-out went through.
    account(gitHubWith({ signedOut: { revoke: null } }));
    expect(marked('github-signed-out')).toHaveLength(1);
    expect(marked('github-revoke')).toEqual([]);
  });

  it('shows who is signed in, with the picture, and the repositories that Columbina may reach', () => {
    const repositories = Array.from({ length: 8 }, (_, at) => `octocat/r${at}`);
    account(gitHubWith({ account: signedIn({ repositories }) }));
    const person = one('github-person');
    expect(person.querySelector('img')?.getAttribute('src')).toBe(PICTURE);
    expect(person.querySelector('img')?.getAttribute('alt')).toBe('');
    expect(person.querySelector('.github-name')?.textContent).toBe('The Octocat');
    expect(person.querySelector('.github-login')?.textContent).toBe('@octocat');
    expect([...one('github-repositories').querySelectorAll('li')].map(row => row.textContent))
      .toEqual(['octocat/r0', 'octocat/r1', 'octocat/r2', 'octocat/r3', 'octocat/r4']);
    expect(document.body.textContent).toContain('And 3 more.');
    expect(marked('github-no-repositories')).toEqual([]);
    expect(one('github-repositories-choose').textContent).toBe('Manage repositories');
    fireEvent.click(one('github-repositories-choose'));
    expect(open.mock.calls).toEqual([[INSTALL_URL]]);
    // Nobody is told that someone signed in who did not sign in just now.
    expect(document.querySelector('.sr-only[role="status"]')).toBeNull();
  });

  it('stands a letter for a person with no picture and no name', () => {
    account(gitHubWith({ account: signedIn({ person: { login: 'octocat', name: null, avatar: null } }) }));
    const person = one('github-person');
    expect(person.querySelector('img')).toBeNull();
    expect(person.querySelector('.avatar-letter')?.textContent).toBe('O');
    expect(person.querySelector('.github-name')).toBeNull();
    expect(person.querySelector('.github-login')?.textContent).toBe('@octocat');
  });

  it('tells whoever cannot see the page who has just signed in', () => {
    account(gitHubWith({ account: signedIn(), signIn: { kind: 'done', person: octocat } }));
    expect(document.querySelector('.sr-only[role="status"]')?.textContent).toBe('Signed in to GitHub as @octocat.');
  });

  it('offers to choose repositories where none is chosen, and asks again when the user comes back', () => {
    const github = gitHubWith({ account: signedIn({ repositories: [] }) });
    account(github);
    expect(one('github-no-repositories').textContent)
      .toBe('Columbina reaches only the repositories that you choose for it on GitHub, and none is chosen yet.');
    expect(marked('github-repositories')).toEqual([]);
    const choose = one('github-repositories-choose');
    expect([choose.textContent, choose.className]).toEqual(['Choose repositories', 'btn btn-main']);
    fireEvent.click(choose);
    expect(open.mock.calls).toEqual([[INSTALL_URL]]);
    window.dispatchEvent(new Event('focus'));
    expect(github.refresh).toHaveBeenCalledTimes(1);
  });

  it('asks nothing when the user comes back while the repositories are known', () => {
    const github = gitHubWith({ account: signedIn() });
    account(github);
    window.dispatchEvent(new Event('focus'));
    expect(github.refresh).not.toHaveBeenCalled();
  });

  it('says where GitHub could not be asked which repositories it may reach, and asks again', () => {
    const github = gitHubWith({ account: signedIn({ repositories: null, offline: 'GitHub could not be reached: timed out' }) });
    account(github);
    expect(one('github-offline').querySelector('p')?.textContent).toBe('GitHub could not be asked which repositories Columbina may reach.');
    expect(one('github-offline').querySelector('details .tool-code')?.textContent).toBe('GitHub could not be reached: timed out');
    expect(one('github-repositories-choose').textContent).toBe('Manage repositories');
    fireEvent.click(button('Check again'));
    window.dispatchEvent(new Event('focus'));
    expect(github.refresh).toHaveBeenCalledTimes(2);
    cleanup();
    account(gitHubWith({ account: signedIn({ repositories: null, offline: '', installUrl: '' }), checking: true }));
    expect(one('github-offline').querySelector('details')).toBeNull();
    expect(marked('github-repositories-choose')).toEqual([]);
    expect((button('Check again') as HTMLButtonElement).disabled).toBe(true);
  });

  it('asks before it signs out, and gives the keyboard to the answer that changes nothing', async () => {
    const github = gitHubWith({ account: signedIn() });
    account(github);
    fireEvent.click(one('github-sign-out'));
    expect(one('github-sign-out-ask').querySelector('p')?.textContent)
      .toBe('Sign out of GitHub on this computer? Columbina can then no longer act in the name of @octocat.');
    expect(marked('github-sign-out')).toEqual([]);
    expect(document.activeElement).toBe(one('github-stay'));
    fireEvent.click(one('github-stay'));
    expect(marked('github-sign-out-ask')).toEqual([]);
    expect(github.signOut).not.toHaveBeenCalled();
    fireEvent.click(one('github-sign-out'));
    await act(async () => { fireEvent.click(one('github-sign-out-sure')); });
    expect(github.signOut).toHaveBeenCalledTimes(1);
    expect(marked('github-sign-out-ask')).toEqual([]);
  });

  it('holds the question while the sign-out is under way', async () => {
    const going = held<boolean>();
    const github = gitHubWith({ account: signedIn(), signOut: vi.fn(() => going.promise) });
    account(github);
    fireEvent.click(one('github-sign-out'));
    act(() => { fireEvent.click(one('github-sign-out-sure')); });
    expect((one('github-sign-out-sure') as HTMLButtonElement).disabled).toBe(true);
    expect((one('github-stay') as HTMLButtonElement).disabled).toBe(true);
    await act(async () => { going.release(false); });
    expect(marked('github-sign-out-ask')).toEqual([]);
    expect(marked('github-sign-out')).toHaveLength(1);
  });

  it('says why a sign-out did not go through', () => {
    account(gitHubWith({ account: signedIn(), signOutFailed: { code: 'STORE', said: 'github.json cannot be written' } }));
    expect(one('github-sign-out-failed').querySelector('p')?.textContent).toBe('The sign-out did not go through.');
    expect(one('github-sign-out-failed').querySelector('details .tool-code')?.textContent).toBe('github.json cannot be written');
  });
});
