/**
 * What the page makes of the sign-in to GitHub: who is signed in, as `columbina login --status`
 * says, and how a sign-in goes, as `columbina login github --json` tells it line by line.
 * Nothing here draws, so all of it can be tried with plain values.
 */

/** Who signed in: the name on GitHub, what the user is called there, and the picture as an address of data. */
export interface Person {
  login: string;
  name: string | null;
  avatar: string | null;
}

/** Who is signed in to GitHub, as far as the page knows. */
export type Account =
  /** Not asked yet, or the answer has not come. */
  | { kind: 'unknown' }
  /** There is no service program to ask, as in a replay. */
  | { kind: 'unavailable' }
  /** This build of Columbina has no GitHub App to sign in with. `why` is what the program said. */
  | { kind: 'unregistered'; why: string }
  /** Nobody is signed in. `ended` is what the program said where a sign-in ran out. */
  | { kind: 'nobody'; installUrl: string; ended: string | null }
  /**
   * Somebody is signed in. `repositories` are those that Columbina may reach, or null where
   * GitHub could not be asked, and `offline` says why then.
   */
  | { kind: 'signedIn'; person: Person; repositories: string[] | null; offline: string | null; installUrl: string }
  /** What is kept of the sign-in could not be read, or the program could not be run. */
  | { kind: 'failed'; code: string; said: string };

const text = (value: unknown): string => (typeof value === 'string' ? value : '');
// Only a page of the web that is encrypted is offered to be opened, or one on this machine,
// where the tests put GitHub. The program holds an address of GitHub to the same.
const page = (value: unknown): string | null =>
  (/^(https:\/\/\S+|http:\/\/(localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/\S*)?)$/.test(text(value)) ? text(value) : null);
const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});

/** Who signed in, from what the program says of it, or null where it says no such thing. */
export function personOf(said: unknown): Person | null {
  const { login, name, avatar } = fields(said);
  if (text(login) === '') return null;
  // A picture is shown only as an address of data of a picture: the page reaches nothing else.
  const picture = /^data:image\/(png|jpeg);base64,[A-Za-z0-9+/=]+$/.test(text(avatar)) ? text(avatar) : null;
  return { login: text(login), name: text(name) || null, avatar: picture };
}

const repositoriesOf = (value: unknown): string[] | null =>
  (Array.isArray(value) ? value.filter((name): name is string => typeof name === 'string' && name !== '') : null);

/**
 * Read what `columbina login --status --json` answered, with what the shell adds: whether the
 * user said on the first page that GitHub can wait. Null is the answer where there is no
 * service program.
 */
export function accountOf(answer: unknown): { account: Account; skipped: boolean } {
  if (answer === null || typeof answer !== 'object') return { account: { kind: 'unavailable' }, skipped: false };
  const given = fields(answer);
  const skipped = given.skipped === true;
  if (typeof given.error === 'string') {
    return { account: { kind: 'failed', code: text(given.code) || 'UNKNOWN', said: given.error }, skipped };
  }
  if (given.registered === false) return { account: { kind: 'unregistered', why: text(given.why) }, skipped };
  const installUrl = page(given.install_url) ?? '';
  const person = personOf(given.signed_in);
  if (person === null) {
    return { account: { kind: 'nobody', installUrl, ended: text(given.ended) || null }, skipped };
  }
  const repositories = repositoriesOf(given.repositories);
  const offline = repositories === null ? text(given.offline) : null;
  return { account: { kind: 'signedIn', person, repositories, offline, installUrl }, skipped };
}

/** How a sign-in goes. */
export type SignIn =
  | { kind: 'idle' }
  /** Begun, and waiting for GitHub to give a code. */
  | { kind: 'asking' }
  /** The code to enter, the page to enter it on, and when the code runs out, in milliseconds. */
  | { kind: 'code'; code: string; url: string; expiresAt: number }
  | { kind: 'done'; person: Person }
  | { kind: 'failed'; code: string; said: string };

/**
 * The sign-in after one more document of what the program said: the code, how it ended, or
 * that the program has ended. A program that ends before it said how is a failure. A sign-in
 * that was stopped is none any more.
 */
export function signInAfter(state: SignIn, said: Record<string, unknown>, now: number): SignIn {
  if (said.ended === true) {
    return state.kind === 'asking' || state.kind === 'code' ? { kind: 'failed', code: 'ENDED', said: '' } : state;
  }
  if (typeof said.error === 'string') {
    return said.code === 'STOPPED' ? { kind: 'idle' } : { kind: 'failed', code: text(said.code) || 'UNKNOWN', said: said.error };
  }
  const person = personOf(said.signed_in);
  if (person !== null) return { kind: 'done', person };
  const url = page(said.url);
  const seconds = said.expires_in;
  if (text(said.code) !== '' && url !== null && typeof seconds === 'number' && Number.isFinite(seconds) && seconds > 0) {
    return { kind: 'code', code: text(said.code), url, expiresAt: now + seconds * 1000 };
  }
  return state;
}

/**
 * What `columbina logout github --json` answered: where the right that was given to the app
 * is taken back on GitHub, or why nothing was done.
 */
export function signedOutOf(answer: unknown): { revoke: string | null } | { code: string; said: string } {
  const given = fields(answer);
  if (typeof given.error === 'string') return { code: text(given.code) || 'UNKNOWN', said: given.error };
  return { revoke: page(given.revoke) };
}

// Which words tell of a failure, by the code of the program. The words of a code that is not
// here are those of a failure of no known kind.
const failures = {
  UNREACHABLE: 'unreachable',
  TIMEOUT: 'timeout',
  BUSY: 'busy',
  EXPIRED: 'expired',
  DENIED: 'denied',
  RATE_LIMITED: 'rateLimited',
  // What is set up wrong is for whoever made this build. A user can only wait for another.
  NOT_REGISTERED: 'unregistered',
  MISCONFIGURED: 'unregistered',
  STORE: 'store',
  ENDED: 'ended',
  NOT_STARTED: 'notStarted',
} as const;

/** What went wrong with a sign-in, as the words of the page name it. */
export type Failure = (typeof failures)[keyof typeof failures] | 'unknown';

/** Which words tell of a failure with this code. */
export function failureOf(code: string): Failure {
  return Object.hasOwn(failures, code) ? failures[code as keyof typeof failures] : 'unknown';
}

/** How many whole seconds a code holds yet, and none once it has run out. */
export function secondsLeft(expiresAt: number, now: number): number {
  return Math.max(0, Math.ceil((expiresAt - now) / 1000));
}

/** A number of seconds as a clock shows it: `14:59`, `0:07`. */
export function clockOf(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

/** How many repositories are named before how many more there are. */
export const NAMED_REPOSITORIES = 5;

/** The repositories that are named, and how many more there are. */
export function repositoriesToShow(repositories: string[]): { shown: string[]; more: number } {
  return {
    shown: repositories.slice(0, NAMED_REPOSITORIES),
    more: Math.max(0, repositories.length - NAMED_REPOSITORIES),
  };
}

/** The letter that stands for a person who has no picture: the first of the name, or of the login. */
export function initialOf(person: Person): string {
  // A letter, and not half of one: some are written with two units of UTF-16.
  const [first] = Array.from(person.name?.trim() || person.login);
  return first.toUpperCase();
}
