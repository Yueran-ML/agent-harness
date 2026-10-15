import { describe, expect, it } from 'vitest';
import {
  accountOf, clockOf, failureOf, initialOf, NAMED_REPOSITORIES, personOf, repositoriesToShow, secondsLeft, signedOutOf,
  signInAfter, type SignIn,
} from '../src/app/github';

// A picture as the program gives it: a PNG of one pixel, as an address of data.
const picture = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const octocat = { login: 'octocat', name: 'The Octocat', avatar: picture };
const install = 'https://github.com/apps/columbina/installations/new';

describe('who signed in', () => {
  it('is read with the name and the picture', () => {
    expect(personOf(octocat)).toEqual(octocat);
    expect(personOf({ ...octocat, avatar: picture.replace('png', 'jpeg') })?.avatar).toBe(picture.replace('png', 'jpeg'));
  });

  it('is nobody without a login', () => {
    for (const said of [null, undefined, 'octocat', 7, [], [octocat], {}, { login: '' }, { login: 7 }, { name: 'The Octocat' }]) {
      expect(personOf(said)).toBeNull();
    }
  });

  it('has no name where none is given, and no picture that is anything but the data of one', () => {
    expect(personOf({ login: 'octocat', name: '', avatar: null })).toEqual({ login: 'octocat', name: null, avatar: null });
    expect(personOf({ login: 'octocat', name: 7 })).toEqual({ login: 'octocat', name: null, avatar: null });
    for (const avatar of [
      'https://avatars.githubusercontent.com/u/583231?v=4',
      'data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=',
      'data:text/html;base64,PGh0bWw+',
      `${picture}"><script>`,
      `${picture} `,
      'data:image/png;base64,',
      7,
    ]) {
      expect(personOf({ login: 'octocat', avatar })?.avatar).toBeNull();
    }
  });

  it('is stood for by the first letter of the name, or of the login', () => {
    expect(initialOf({ login: 'octocat', name: 'the Octocat', avatar: null })).toBe('T');
    expect(initialOf({ login: 'octocat', name: null, avatar: null })).toBe('O');
    expect(initialOf({ login: 'octocat', name: '  ', avatar: null })).toBe('O');
    // A letter that takes two units of UTF-16 is kept whole.
    expect(initialOf({ login: 'octocat', name: '𝒪cto', avatar: null })).toBe('𝒪');
  });
});

describe('what the program says of who is signed in', () => {
  it('is nothing to ask where there is no service program', () => {
    expect(accountOf(null)).toEqual({ account: { kind: 'unavailable' }, skipped: false });
    expect(accountOf(undefined)).toEqual({ account: { kind: 'unavailable' }, skipped: false });
  });

  it('names who is signed in, the repositories that Columbina may reach, and where others are chosen', () => {
    const answer = { signed_in: octocat, install_url: install, repositories: ['octocat/hello', '', 7, 'octocat/world'], skipped: false };
    expect(accountOf(answer)).toEqual({
      account: { kind: 'signedIn', person: octocat, repositories: ['octocat/hello', 'octocat/world'], offline: null, installUrl: install },
      skipped: false,
    });
  });

  it('says why the repositories are not known, where GitHub could not be asked', () => {
    const { account } = accountOf({ signed_in: octocat, install_url: install, offline: 'GitHub could not be reached' });
    expect(account).toEqual({
      kind: 'signedIn', person: octocat, repositories: null, offline: 'GitHub could not be reached', installUrl: install,
    });
    expect(accountOf({ signed_in: octocat, install_url: install }).account).toMatchObject({ repositories: null, offline: '' });
  });

  it('is nobody where nobody is signed in, and says so where a sign-in ran out', () => {
    expect(accountOf({ signed_in: null, install_url: install }).account).toEqual({ kind: 'nobody', installUrl: install, ended: null });
    expect(accountOf({ signed_in: null, install_url: install, ended: 'the sign-in to GitHub has run out. Sign in again' }).account)
      .toEqual({ kind: 'nobody', installUrl: install, ended: 'the sign-in to GitHub has run out. Sign in again' });
  });

  it('offers no page to choose repositories on that is not an encrypted page of the web, or one on this machine', () => {
    for (const odd of [
      'http://github.com/apps/x/installations/new', 'javascript:alert(1)', 'https://', 'https://a b', 7, null,
      'http://127.0.0.1.example.com/apps', 'http://localhost.example.com', 'http://127.0.0.1:80 /apps', 'http://[::2]/apps',
    ]) {
      expect(accountOf({ signed_in: null, install_url: odd }).account).toEqual({ kind: 'nobody', installUrl: '', ended: null });
    }
    // Where the tests put GitHub, on this machine, the page is offered as it is.
    for (const here of [
      'http://127.0.0.1:61319/apps/columbina-test/installations/new', 'http://localhost/apps/x/installations/new',
      'http://[::1]:8080/apps/x', 'http://127.0.0.1',
    ]) {
      expect(accountOf({ signed_in: null, install_url: here }).account).toEqual({ kind: 'nobody', installUrl: here, ended: null });
    }
  });

  it('knows a build that has no GitHub App', () => {
    const answer = { signed_in: null, registered: false, why: 'this build of Columbina has no GitHub App to sign in with' };
    expect(accountOf(answer).account).toEqual({ kind: 'unregistered', why: answer.why });
    expect(accountOf({ registered: false }).account).toEqual({ kind: 'unregistered', why: '' });
  });

  it('tells what could not be read', () => {
    const answer = { error: 'the tokens of GitHub cannot be used', code: 'STORE', skipped: true };
    expect(accountOf(answer)).toEqual({ account: { kind: 'failed', code: 'STORE', said: answer.error }, skipped: true });
    expect(accountOf({ error: 'it broke' }).account).toEqual({ kind: 'failed', code: 'UNKNOWN', said: 'it broke' });
  });

  it('says that GitHub is to wait only where the shell says yes', () => {
    expect(accountOf({ signed_in: null, skipped: true }).skipped).toBe(true);
    for (const skipped of [false, 'true', 1, undefined]) expect(accountOf({ signed_in: null, skipped }).skipped).toBe(false);
  });
});

describe('a sign-in', () => {
  const idle: SignIn = { kind: 'idle' };
  const asking: SignIn = { kind: 'asking' };
  const code: SignIn = { kind: 'code', code: 'WDJB-MJHT', url: 'https://github.com/login/device', expiresAt: 900_000 };
  const said = { code: 'WDJB-MJHT', url: 'https://github.com/login/device', expires_in: 899 };

  it('shows the code and the page, until the code runs out', () => {
    expect(signInAfter(asking, said, 1_000)).toEqual({
      kind: 'code', code: 'WDJB-MJHT', url: 'https://github.com/login/device', expiresAt: 900_000,
    });
  });

  it('takes no code that it could not show, or that holds for no time', () => {
    for (const odd of [
      { ...said, code: '' }, { ...said, code: 7 }, { ...said, url: 'http://github.com/login/device' }, { ...said, url: null },
      { ...said, expires_in: 0 }, { ...said, expires_in: -5 }, { ...said, expires_in: '899' }, { ...said, expires_in: Number.NaN },
      { ...said, expires_in: Number.POSITIVE_INFINITY }, {},
    ]) {
      expect(signInAfter(asking, odd, 1_000)).toBe(asking);
    }
  });

  it('is done when the program says who signed in', () => {
    const done = signInAfter(code, { signed_in: octocat, install_url: install, repositories: [] }, 2_000);
    expect(done).toEqual({ kind: 'done', person: octocat });
    // The end of the program changes nothing then.
    expect(signInAfter(done, { ended: true }, 3_000)).toBe(done);
  });

  it('fails as the program says it failed', () => {
    const failed = signInAfter(code, { error: 'the sign-in was turned down on GitHub', code: 'DENIED' }, 2_000);
    expect(failed).toEqual({ kind: 'failed', code: 'DENIED', said: 'the sign-in was turned down on GitHub' });
    expect(signInAfter(failed, { ended: true }, 3_000)).toBe(failed);
    expect(signInAfter(asking, { error: 'it broke', code: 7 }, 2_000)).toEqual({ kind: 'failed', code: 'UNKNOWN', said: 'it broke' });
  });

  it('is none any more where it was stopped', () => {
    expect(signInAfter(code, { error: 'stopped', code: 'STOPPED' }, 2_000)).toEqual(idle);
  });

  it('fails where the program ended before it said how the sign-in went', () => {
    expect(signInAfter(asking, { ended: true }, 2_000)).toEqual({ kind: 'failed', code: 'ENDED', said: '' });
    expect(signInAfter(code, { ended: true }, 2_000)).toEqual({ kind: 'failed', code: 'ENDED', said: '' });
    // A program that ends while none is waited for, such as one that was stopped, is no failure.
    expect(signInAfter(idle, { ended: true }, 2_000)).toBe(idle);
    expect(signInAfter(code, { ended: 'yes' }, 2_000)).toBe(code);
  });

  it('is done also where the page was not waiting for it, as after the page was loaded again', () => {
    expect(signInAfter(idle, { signed_in: octocat }, 2_000)).toEqual({ kind: 'done', person: octocat });
  });
});

describe('a sign-out', () => {
  it('says where the right is taken back', () => {
    const revoke = 'https://github.com/settings/apps/authorizations';
    expect(signedOutOf({ signed_out: true, revoke })).toEqual({ revoke });
    expect(signedOutOf({ signed_out: false, revoke })).toEqual({ revoke });
    expect(signedOutOf({ signed_out: true, revoke: 'javascript:alert(1)' })).toEqual({ revoke: null });
    expect(signedOutOf(null)).toEqual({ revoke: null });
  });

  it('says why nothing was done', () => {
    expect(signedOutOf({ error: 'the file cannot be written', code: 'STORE' })).toEqual({ code: 'STORE', said: 'the file cannot be written' });
    expect(signedOutOf({ error: 'it broke' })).toEqual({ code: 'UNKNOWN', said: 'it broke' });
  });
});

describe('the words for a failure', () => {
  it('are chosen by its code', () => {
    expect(failureOf('UNREACHABLE')).toBe('unreachable');
    expect(failureOf('TIMEOUT')).toBe('timeout');
    expect(failureOf('BUSY')).toBe('busy');
    expect(failureOf('EXPIRED')).toBe('expired');
    expect(failureOf('DENIED')).toBe('denied');
    expect(failureOf('RATE_LIMITED')).toBe('rateLimited');
    expect(failureOf('STORE')).toBe('store');
    expect(failureOf('ENDED')).toBe('ended');
    expect(failureOf('NOT_STARTED')).toBe('notStarted');
  });

  it('say to a user that this build cannot sign in, where it is set up wrong', () => {
    expect(failureOf('NOT_REGISTERED')).toBe('unregistered');
    expect(failureOf('MISCONFIGURED')).toBe('unregistered');
  });

  it('are those of no known kind for any other code', () => {
    for (const code of ['UNKNOWN', 'FORBIDDEN', '', 'toString', '__proto__', 'constructor']) expect(failureOf(code)).toBe('unknown');
  });
});

describe('the time that a code holds', () => {
  it('is counted in whole seconds, and is none once it has run out', () => {
    expect(secondsLeft(900_000, 1_000)).toBe(899);
    expect(secondsLeft(900_000, 1_001)).toBe(899);
    expect(secondsLeft(900_000, 899_999)).toBe(1);
    expect(secondsLeft(900_000, 900_000)).toBe(0);
    expect(secondsLeft(900_000, 960_000)).toBe(0);
  });

  it('is shown as a clock shows it', () => {
    expect(clockOf(899)).toBe('14:59');
    expect(clockOf(600)).toBe('10:00');
    expect(clockOf(7)).toBe('0:07');
    expect(clockOf(0)).toBe('0:00');
    expect(clockOf(-3)).toBe('0:00');
    expect(clockOf(61.9)).toBe('1:01');
  });
});

describe('the repositories that are named', () => {
  const names = (count: number) => Array.from({ length: count }, (_, at) => `octocat/r${at}`);

  it('are all of them, where there are few', () => {
    expect(repositoriesToShow([])).toEqual({ shown: [], more: 0 });
    expect(repositoriesToShow(names(NAMED_REPOSITORIES))).toEqual({ shown: names(NAMED_REPOSITORIES), more: 0 });
  });

  it('are the first few, with how many more there are', () => {
    expect(repositoriesToShow(names(NAMED_REPOSITORIES + 3))).toEqual({ shown: names(NAMED_REPOSITORIES), more: 3 });
  });
});
