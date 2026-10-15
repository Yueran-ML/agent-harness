/**
 * The first steps on the first page: a model service with its key, GitHub if the user likes,
 * and a project. What each step stands at is worked out here, out of what the shell said.
 * Nothing here draws, so all of it can be tried with plain values.
 */
import { called, type Listed } from '../key/form';
import type { Account, Person } from './github';
import { keyAtHand } from './tell';

/**
 * The service of the profile that the application starts, which talks to it where no service
 * was chosen. The tests hold this to the profile.
 */
export const PROFILE_SERVICE = 'deepseek';

/** Where the step of the model service stands. */
export type ModelStep =
  /** The program is being asked for the first time. */
  | { kind: 'checking' }
  /** The service in use has what it needs: its key, or none where it asks for none. */
  | { kind: 'ready'; service: string; title: string; model: string | null }
  /** The service in use has no key that it can use. */
  | { kind: 'missing'; service: string; title: string }
  /** What is set up is not known: there is no program to ask, or it could not be asked. */
  | { kind: 'unknown' };

/**
 * The step of the model service, out of the services as the program lists them.
 *
 * Args:
 *   listed: The services, or null where the program has not answered or could not be asked.
 *   checking: Whether the program is being asked. A list that is there is shown meanwhile.
 */
export function modelStepOf(listed: Listed | null, checking: boolean): ModelStep {
  if (listed === null) return checking ? { kind: 'checking' } : { kind: 'unknown' };
  const name = listed.use ?? PROFILE_SERVICE;
  const service = listed.services.find(each => each.name === name);
  if (service === undefined) return { kind: 'unknown' };
  const title = called(service);
  return keyAtHand(name, listed) ? { kind: 'ready', service: name, title, model: service.model } : { kind: 'missing', service: name, title };
}

/** Where the step of GitHub stands. */
export type GitHubStep =
  /** Not known yet. */
  | { kind: 'checking' }
  /** Not offered: this build cannot sign in, the program could not say, or the user said it can wait. */
  | { kind: 'hidden' }
  /** Nobody is signed in. */
  | { kind: 'offered' }
  | { kind: 'ready'; person: Person; repositories: string[] | null; installUrl: string };

/** The step of GitHub, out of who is signed in and whether the user said that GitHub can wait. */
export function gitHubStepOf(account: Account, skipped: boolean): GitHubStep {
  if (account.kind === 'unknown') return { kind: 'checking' };
  if (account.kind === 'signedIn') {
    return { kind: 'ready', person: account.person, repositories: account.repositories, installUrl: account.installUrl };
  }
  return account.kind === 'nobody' && !skipped ? { kind: 'offered' } : { kind: 'hidden' };
}

/** What the first page shows of the first steps. */
export type FirstSteps =
  /** Nothing: there is nothing to set up from here, or it is not known yet whether there is. */
  | { kind: 'none' }
  /** All that is offered is done. The line says what is set up. */
  | { kind: 'done'; line: string }
  /** Something is still to be done. */
  | { kind: 'open' };

/** What the first page shows of the first steps, out of where each stands. */
export function firstStepsOf(model: ModelStep, github: GitHubStep): FirstSteps {
  if (model.kind === 'checking' || model.kind === 'unknown' || github.kind === 'checking') return { kind: 'none' };
  if (model.kind === 'missing' || github.kind === 'offered') return { kind: 'open' };
  const parts = [model.title, ...(model.model ? [model.model] : []), ...(github.kind === 'ready' ? [`@${github.person.login}`] : [])];
  return { kind: 'done', line: parts.join(' · ') };
}
