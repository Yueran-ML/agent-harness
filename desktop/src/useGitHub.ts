import { useCallback, useEffect, useRef, useState } from 'react';
import { accountOf, personOf, signedOutOf, signInAfter, type Account, type SignIn } from './app/github';

const said = (problem: unknown) => (problem instanceof Error ? problem.message : String(problem));

/**
 * Who is signed in to GitHub, and the sign-in that runs, as the shell tells of them. The page
 * holds one of these for the first page and the settings, so that both show the same sign-in.
 * No token comes this way: the program keeps them, and says only who signed in.
 */
export function useGitHub() {
  const [account, setAccount] = useState<Account>(window.columbina ? { kind: 'unknown' } : { kind: 'unavailable' });
  // Whether the user said on the first page that GitHub can wait.
  const [skipped, setSkipped] = useState(false);
  // Whether the program is being asked who is signed in.
  const [checking, setChecking] = useState(false);
  const [signIn, setSignIn] = useState<SignIn>({ kind: 'idle' });
  // That the user has signed out here, and where the right that was given to the app is taken back.
  const [signedOut, setSignedOut] = useState<{ revoke: string | null } | null>(null);
  // Why the sign-out did not go through.
  const [signOutFailed, setSignOutFailed] = useState<{ code: string; said: string } | null>(null);
  // The asks of who is signed in, by number. Only the answer to the last one counts, and none
  // to one that was asked before a sign-in or a sign-out changed it.
  const asked = useRef(0);

  const refresh = useCallback(async () => {
    const bridge = window.columbina;
    if (!bridge) return;
    const number = ++asked.current;
    setChecking(true);
    try {
      const answer = accountOf(await bridge.githubStatus());
      if (number !== asked.current) return;
      setAccount(answer.account);
      setSkipped(answer.skipped);
    } catch (problem) {
      if (number !== asked.current) return;
      setAccount({ kind: 'failed', code: 'UNAVAILABLE', said: said(problem) });
    } finally {
      if (number === asked.current) setChecking(false);
    }
  }, []);

  useEffect(() => {
    const bridge = window.columbina;
    if (!bridge) return undefined;
    const off = bridge.onGitHub((told) => {
      if (personOf(told.signed_in) !== null) {
        // What the sign-in says goes before any answer that was asked for earlier.
        asked.current += 1;
        setChecking(false);
        setAccount(accountOf(told).account);
      }
      setSignIn(state => signInAfter(state, told, Date.now()));
    });
    void refresh();
    return off;
  }, [refresh]);

  const begin = useCallback(async () => {
    const bridge = window.columbina;
    if (!bridge) return;
    setSignedOut(null);
    setSignOutFailed(null);
    setSignIn({ kind: 'asking' });
    try {
      await bridge.githubSignIn();
    } catch (problem) {
      setSignIn({ kind: 'failed', code: 'NOT_STARTED', said: said(problem) });
    }
  }, []);

  /** Stop the sign-in that runs. The program says nothing more of it. */
  const cancel = useCallback(() => {
    setSignIn({ kind: 'idle' });
    void window.columbina?.githubCancel().catch(() => {});
  }, []);

  /** Sign out on this computer. True where it went through. */
  const signOut = useCallback(async (): Promise<boolean> => {
    const bridge = window.columbina;
    if (!bridge) return false;
    setSignOutFailed(null);
    let answer: ReturnType<typeof signedOutOf>;
    try {
      answer = signedOutOf(await bridge.githubSignOut());
    } catch (problem) {
      answer = { code: 'UNAVAILABLE', said: said(problem) };
    }
    if ('code' in answer) {
      setSignOutFailed(answer);
      return false;
    }
    asked.current += 1;
    setChecking(false);
    setAccount(current => ({
      kind: 'nobody', installUrl: current.kind === 'signedIn' || current.kind === 'nobody' ? current.installUrl : '', ended: null,
    }));
    setSignIn({ kind: 'idle' });
    setSignedOut({ revoke: answer.revoke });
    return true;
  }, []);

  /** Say whether GitHub is to wait. The shell keeps it for the next start. */
  const skip = useCallback(async (wait: boolean) => {
    setSkipped(wait);
    try {
      await window.columbina?.skipGitHub(wait);
    } catch {
      // It holds for this window, and the first page offers GitHub again at the next start.
    }
  }, []);

  return { account, skipped, checking, signIn, signedOut, signOutFailed, refresh, begin, cancel, signOut, skip };
}

export type GitHub = ReturnType<typeof useGitHub>;

/** Ask again who is signed in whenever the window comes back to the front, while `when` holds. */
export function useRefreshOnFocus(refresh: () => unknown, when: boolean) {
  useEffect(() => {
    if (!when) return undefined;
    const again = () => { void refresh(); };
    window.addEventListener('focus', again);
    return () => window.removeEventListener('focus', again);
  }, [refresh, when]);
}
