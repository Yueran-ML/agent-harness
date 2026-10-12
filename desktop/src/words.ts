/**
 * The words for how things ended and for what a task may do.
 *
 * `outcomes.ts` sorts what the service says into kinds. Here each kind is
 * looked up in a catalogue. A kind without words does not compile: every
 * lookup below is checked against the English catalogue, and every other
 * catalogue against that.
 */
import type { Messages } from './messages/en';
import type { ModelFailure, ToolOutcome, TurnOutcome } from './outcomes';

/** What the interface says of one thing: what happened, and why if that is known. */
export interface Saying {
  what: string;
  why?: string;
}

/**
 * Say why a request to the model failed.
 *
 * Args:
 *   messages: The catalogue.
 *   failure: The failure.
 *   service: The name of the service that the model comes from, where the window knows
 *     it. A key that is missing is then said to be missing for that service.
 */
export function sayModelFailure(messages: Messages, failure: ModelFailure, service: string | null = null): Saying {
  if (failure.kind === 'noKey' && service) return messages.model.noKeyFor(service);
  return messages.model[failure.kind];
}

/**
 * Say how a call of a tool stands or ended.
 *
 * `what` is a few words for the row of the call. Where the user allowed the
 * call, `messages.tool.allowedByYou` can stand beside them.
 */
export function sayTool(messages: Messages, outcome: ToolOutcome): Saying {
  const tool = messages.tool;
  switch (outcome.kind) {
    case 'running':
      return tool.running;
    case 'waitingForApproval':
      return tool.waitingForApproval;
    case 'done':
      return tool.done;
    case 'exited':
      return tool.exited(outcome.exitCode);
    case 'timedOut':
      // Only a command says after how long it was stopped.
      if (outcome.of !== 'command') return tool.timedOut[outcome.of];
      return outcome.seconds === null ? tool.timedOut.call : tool.timedOut.command(outcome.seconds);
    case 'declined':
      return tool.declined;
    case 'refused':
      return tool.refused[outcome.by];
    case 'stopped':
      return tool.stopped[outcome.because];
    case 'notRun':
      return tool.notRun[outcome.because];
    case 'unknownTool':
      return tool.unknownTool;
    case 'wrongArguments':
      return tool.wrongArguments;
    case 'failed':
      return tool.failed[outcome.cause];
    case 'withheld':
      return tool.withheld;
    case 'noResult':
      return tool.noResult;
  }
}

/**
 * Say how a turn ended.
 *
 * A turn that completed did not go wrong, so there is nothing to say of its
 * end: null. The line for it is `messages.turn.took`. Of a turn that a hook
 * stopped, the reason of the hook is in the outcome, as the hook gave it.
 */
export function sayTurn(messages: Messages, outcome: TurnOutcome, service: string | null = null): Saying | null {
  const turn = messages.turn;
  switch (outcome.kind) {
    case 'completed':
      return null;
    case 'stopped':
      return turn.stopped[outcome.by];
    case 'limit':
      // Only the limit of steps is a number that says something to a reader.
      return outcome.of === 'steps' ? turn.limit.steps(outcome.limit) : turn.limit[outcome.of];
    case 'inputStopped':
      return turn.inputStopped;
    case 'blocked':
      return turn.blocked;
    case 'modelFailed':
      return sayModelFailure(messages, outcome.failure, service);
    case 'crashed':
      return turn.crashed;
    case 'outputLimit':
      return turn.outputLimit;
    case 'interrupted':
      return turn.interrupted;
    case 'forked':
      return turn.forked;
    case 'unknown':
      return turn.unknown;
  }
}

/**
 * Say in a sentence what a task may do.
 *
 * Args:
 *   mode: The mode of the sandbox, as `session.status` has it.
 *   policy: The policy of approval, as `session.status` has it.
 *
 * What a task does with a command depends on both. With full access a
 * command that no rule speaks about runs without a question. Otherwise it
 * needs a yes, which the policy `ask` asks for and the policy `never` never
 * gets (`permissions/__init__.py` in the sources of the service).
 */
export function sayPermissions(messages: Messages, mode: string | null, policy: string | null): string {
  const permissions = messages.permissions;
  if (mode === 'danger-full-access') return permissions.fullAccess;

  const files = mode === 'read-only' ? 'readOnly' : mode === 'workspace-write' ? 'write' : null;
  const commands = policy === 'ask' ? 'Asks' : policy === 'never' ? 'Strict' : null;
  if (files === null || commands === null) return permissions.unknown;
  return permissions[`${files}${commands}`];
}

/**
 * Say how well this computer holds a command to what the task may do, or
 * null for a strength that is not known.
 */
export function sayConfinement(messages: Messages, strength: string | null): string | null {
  if (strength === 'none' || strength === 'partial' || strength === 'full') {
    return messages.confinement[strength];
  }
  return null;
}
