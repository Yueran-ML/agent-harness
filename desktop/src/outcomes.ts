/**
 * How things ended: a call of a tool, a request to the model, a turn.
 *
 * The service says it in codes and in sentences of its own. Here each is
 * sorted into a kind that the interface has words for. Nothing in this module
 * is shown to a person as it stands. The code and the sentence of the service
 * are passed on beside the kind, for a view of the details.
 *
 * What a code means was read in the sources of the service, not guessed from
 * its name. Where a comment names a file, it lies under `src/columbina`.
 */
import type { RequestFailure, ToolItem } from './state';
import type { Turn } from './turns';

/** What can be done next about something that went wrong. The interface makes a button of it. */
export type ActionKey = 'retry' | 'openModelSettings' | 'openPermissions' | 'compact' | 'none';

// A table is asked for a code that the service wrote. A code such as `toString`
// must not find what every object has.
function lookup<Value>(table: Record<string, Value>, key: string): Value | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined;
}

// ---------------------------------------------------------------------------
// A request to the model

export type ModelFailureKind =
  | 'noKey'
  | 'keyRefused'
  | 'keyProblem'
  | 'quota'
  | 'rateLimit'
  | 'contextTooLong'
  | 'contentFilter'
  | 'invalidRequest'
  | 'server'
  | 'timeout'
  | 'unreachable'
  | 'cutOff'
  | 'malformed'
  | 'empty'
  | 'internal'
  | 'unknown';

export interface ModelFailure {
  kind: ModelFailureKind;
  next: ActionKey;
  /** For a view of the details: the code, and the message as the service gave it. */
  code: string;
  detail: string | null;
}

/** What is known of a failure. Only the code is needed. */
export interface FailureGiven {
  code: string;
  message?: string | null;
  /**
   * The HTTP status that came with the failure. Null says that none came:
   * the service was not reached. Left out, it is not known.
   */
  status?: number | null;
}

const MODEL_FAILURES: Record<string, { kind: ModelFailureKind; next: ActionKey }> = {
  // HTTP 402, or the service names the failure `insufficient_quota`.
  QUOTA: { kind: 'quota', next: 'openModelSettings' },
  // HTTP 429.
  RATE_LIMIT: { kind: 'rateLimit', next: 'retry' },
  // HTTP 400 that speaks of the length of the context.
  CONTEXT_WINDOW_EXCEEDED: { kind: 'contextTooLong', next: 'compact' },
  // The reply ended because the provider withheld it.
  CONTENT_FILTER: { kind: 'contentFilter', next: 'none' },
  // HTTP 400, 404 or 422, or no adapter is there for the provider that the config names.
  INVALID_REQUEST: { kind: 'invalidRequest', next: 'openModelSettings' },
  // HTTP 500 and above, or an error in the middle of the reply.
  SERVER: { kind: 'server', next: 'retry' },
  // HTTP 408, no connection in time, or no piece of the reply for too long.
  TIMEOUT: { kind: 'timeout', next: 'retry' },
  // The service could not be reached.
  TRANSPORT: { kind: 'unreachable', next: 'retry' },
  // The reply stopped without having ended.
  STREAM_CLOSED: { kind: 'cutOff', next: 'retry' },
  // The service sent what cannot be read as a reply.
  MALFORMED_RESPONSE: { kind: 'malformed', next: 'retry' },
  // The reply ended and held nothing.
  EMPTY_RESPONSE: { kind: 'empty', next: 'retry' },
  // A fault on this side of the wire, in the adapter or in what reads its stream.
  INTERNAL: { kind: 'internal', next: 'retry' },
};

/**
 * Say why a request to the model failed.
 *
 * The codes are those of `llm/adapters/openai_chat.py` and `llm/runtime.py`.
 * A code that is not known here, such as `HTTP_418` for a status that the
 * adapter has no name for, is a failure of kind `unknown`.
 */
export function classifyModelFailure(given: FailureGiven): ModelFailure {
  const told = { code: given.code, detail: given.message || null };
  if (given.code === 'AUTH') {
    // The code covers two things that a person has to tell apart. A key that is
    // missing, or that cannot be sent, fails before any request: no status. A key
    // that the service turns down comes back with HTTP 401 or 403.
    const kind = given.status === undefined ? 'keyProblem' : given.status === null ? 'noKey' : 'keyRefused';
    return { kind, next: 'openModelSettings', ...told };
  }
  const known = lookup(MODEL_FAILURES, given.code);
  return known ? { ...known, ...told } : { kind: 'unknown', next: 'retry', ...told };
}

// ---------------------------------------------------------------------------
// A call of a tool

/** Who or what refused a call. */
export type RefusedBy =
  | 'rule'
  | 'hook'
  | 'policy'
  | 'nobody'
  | 'approval'
  | 'readOnly'
  | 'protected'
  | 'outside'
  | 'notWritable'
  | 'rulesUnreadable'
  | 'rulesNotRead';

/** Why a call that ran, or was about to, did not do what it was asked. */
export type ToolFailureCause =
  | 'error'
  | 'checkFailed'
  | 'sandboxUnavailable'
  | 'commandTooLong'
  | 'projectMoved'
  | 'notFound'
  | 'notAFile'
  | 'notADirectory'
  | 'notText'
  | 'tooLarge'
  | 'notRead'
  | 'changedSinceRead'
  | 'noMatch'
  | 'ambiguous'
  | 'noChange'
  | 'invalidPath'
  | 'invalidPattern'
  | 'moved'
  | 'searchFailed';

export type ToolEnding =
  | { kind: 'running' }
  /** A question about the call was put to the user and has no answer yet. */
  | { kind: 'waitingForApproval' }
  | { kind: 'done' }
  /** A command ran to its end and gave another exit code than 0. */
  | { kind: 'exited'; exitCode: number }
  | { kind: 'timedOut'; of: 'call' | 'command' | 'search'; seconds: number | null }
  /** The user was asked and said no. */
  | { kind: 'declined' }
  | { kind: 'refused'; by: RefusedBy }
  /** The call was cut short while it ran. It may have taken effect in part. */
  | { kind: 'stopped'; because: 'turnStopped' | 'programEnded' }
  /** The call never began. */
  | { kind: 'notRun'; because: 'turnStopped' | 'programEnded' | 'questionWithdrawn' }
  | { kind: 'unknownTool' }
  | { kind: 'wrongArguments' }
  | { kind: 'failed'; cause: ToolFailureCause }
  /** The call ran, and a hook kept its result from the model. */
  | { kind: 'withheld' }
  /** The turn ended, and the log holds no result of the call. */
  | { kind: 'noResult' };

export type ToolOutcome = ToolEnding & {
  next: ActionKey;
  /** Whether the user was asked about the call and said yes. */
  allowedByYou: boolean;
  /** For a view of the details: the code, and the reason as the service gave it. */
  code: string | null;
  detail: string | null;
};

interface ToolRule {
  ending: ToolEnding;
  next: ActionKey;
}

const refused = (by: RefusedBy, next: ActionKey = 'none'): ToolRule => ({ ending: { kind: 'refused', by }, next });
const failed = (cause: ToolFailureCause): ToolRule => ({ ending: { kind: 'failed', cause }, next: 'none' });
const ended = (ending: ToolEnding): ToolRule => ({ ending, next: 'none' });

const TOOL_CODES: Record<string, ToolRule> = {
  // The pipeline that every call goes through: `tools/pipeline.py`, `tools/tool.py`.
  // No tool of that name, or the tool left between the check and the run.
  UNKNOWN_TOOL: ended({ kind: 'unknownTool' }),
  // The arguments are no JSON object, or do not fit the parameters of the tool.
  INVALID_ARGS: ended({ kind: 'wrongArguments' }),
  // The tool raised something, or reported a failure without a code of its own.
  TOOL_ERROR: failed('error'),
  // The whole call took longer than its tool may take.
  TOOL_TIMEOUT: ended({ kind: 'timedOut', of: 'call', seconds: null }),
  // A check of the call or of its result failed. What cannot be checked does not
  // run, and a result that cannot be checked is withheld. `fs/__init__.py` says the
  // same of an access to a file.
  CHECK_FAILED: failed('checkFailed'),
  // The tool ran. A check of its result, which is a hook of the user, withheld it.
  RESULT_BLOCKED: ended({ kind: 'withheld' }),

  // The loop, when a turn is cancelled: `agent_loop/loop.py`.
  // The call was running. It may have taken effect in part.
  TOOL_CANCELLED: ended({ kind: 'stopped', because: 'turnStopped' }),
  // The call had not begun.
  ABORTED_BEFORE_DISPATCH: ended({ kind: 'notRun', because: 'turnStopped' }),

  // A log that a run left open and that was closed afterwards: `session/repair.py`.
  // The call had been dispatched. Whether it took effect is not known.
  TOOL_OUTCOME_UNKNOWN: ended({ kind: 'stopped', because: 'programEnded' }),
  // The call had not been dispatched.
  TOOL_NOT_STARTED: ended({ kind: 'notRun', because: 'programEnded' }),

  // The rules and who is asked: `permissions/__init__.py`. APPROVAL_REJECTED is not
  // here: what it means depends on whether anybody was asked. See `rejected` below.
  // A rule of the user or of the project is against the call, or against the file.
  DENIED: refused('rule'),
  // The question was withdrawn before it was answered.
  APPROVAL_CANCELLED: ended({ kind: 'notRun', because: 'questionWithdrawn' }),
  // Nobody was there who could be asked.
  APPROVAL_UNAVAILABLE: refused('nobody'),
  // The rules of the project cannot be read. Nothing runs until that is put right.
  RULES_UNREADABLE: refused('rulesUnreadable'),
  // The rules of the session have not been read yet.
  RULES_NOT_READ: refused('rulesNotRead'),

  // A hook of the user stopped the call, or wanted somebody asked and nobody
  // said yes: `hooks/__init__.py`.
  HOOK_BLOCKED: refused('hook'),

  // The mode of the sandbox: `sandbox/__init__.py`.
  // The session is read-only, so nothing is written.
  READ_ONLY: refused('readOnly', 'openPermissions'),
  // The file lies in `.git` or `.columbina`, which this mode does not write.
  PROTECTED: refused('protected'),

  // The seam through which tools reach files: `fs/__init__.py`, `fs/paths.py`.
  // Only files inside the working directory are read or written.
  OUTSIDE_WORKING_DIRECTORY: refused('outside', 'openPermissions'),
  // Nothing in this session allows writing files.
  NOT_WRITABLE: refused('notWritable', 'openPermissions'),
  // The working directory leads somewhere else than when the session began.
  WORKING_DIRECTORY_MOVED: failed('projectMoved'),
  // The file exists and the model has not read it, so it is not written over.
  NOT_READ: failed('notRead'),
  // The file has changed since the model read it.
  CHANGED_SINCE_READ: failed('changedSinceRead'),
  // The file or a directory on the way was not where it had been a moment before.
  MOVED: failed('moved'),
  // The path names a directory, or something else that is no regular file.
  NOT_A_FILE: failed('notAFile'),
  // The path is empty, or spelled in a way that is not used here.
  INVALID_PATH: failed('invalidPath'),

  // The tools themselves: `builtin_tools/`.
  // No such file, or no such directory to search.
  NOT_FOUND: failed('notFound'),
  // A directory was to be searched, and the path names a file.
  NOT_A_DIRECTORY: failed('notADirectory'),
  // The file does not hold text in UTF-8.
  NOT_TEXT: failed('notText'),
  // The file is larger than an edit takes.
  TOO_LARGE: failed('tooLarge'),
  // The text that an edit is to replace does not occur in the file.
  NO_MATCH: failed('noMatch'),
  // It occurs more than once, and the call does not say to replace every one.
  AMBIGUOUS: failed('ambiguous'),
  // The old text and the new text of an edit are the same.
  NO_CHANGE: failed('noChange'),
  // A pattern of paths that leaves the directory, or text that is no regular expression.
  INVALID_PATTERN: failed('invalidPattern'),
  // The process that matches the lines of a search broke down.
  SEARCH_FAILED: failed('searchFailed'),
  // A search took longer than it may.
  SEARCH_TIMEOUT: ended({ kind: 'timedOut', of: 'search', seconds: null }),
  // The command is longer than can be handed to the shell.
  COMMAND_TOO_LONG: failed('commandTooLong'),
  // The sandbox that is to hold commands is not there, so the command was not run.
  SANDBOX_UNAVAILABLE: failed('sandboxUnavailable'),
};

/**
 * Say what APPROVAL_REJECTED means for one call.
 *
 * The service gives this code in two cases (`permissions/__init__.py`). Under
 * the policy `ask` a person was asked and said no. Under `never` nobody is
 * asked, and every call that would need a yes is turned down. Only the first
 * is a decision of the user, so only the first may say so.
 */
function rejected(item: ToolItem): ToolRule {
  const policy = item.approval?.policy ?? null;
  if (policy === 'ask') return ended({ kind: 'declined' });
  if (policy === 'never') return refused('policy', 'openPermissions');
  // The log does not say who was asked.
  return refused('approval');
}

/** The tools that run a command: `bash`, and `powershell` on Windows (`builtin_tools/shell.py`). */
const SHELLS = new Set(['bash', 'powershell']);

/** What a hook that ran after a call adds to its result begins with this (`hooks/__init__.py`). */
const HOOK_ADDS = '\n\n[A hook of the user adds:]\n';

/**
 * Read from the result of a command how the command ended.
 *
 * A command that gives another exit code than 0 has not failed as a call: its
 * result holds no error. The tool writes a last line that names the exit code,
 * or says that the command was stopped after its time. Notes about where the
 * whole output is may follow that line, each in parentheses on a line of its
 * own, and after those what a hook adds.
 */
function commandEnd(result: string): { exitCode: number } | { stoppedAfter: number } | null {
  const written = result.split(HOOK_ADDS)[0];
  const lines = written.trimEnd().split(/\r?\n/);
  let last = lines.length - 1;
  while (last > 0 && /^\(.*\)$/.test(lines[last])) last -= 1;

  const exit = /^\[exit code (-?\d+)\]$/.exec(lines[last]);
  if (exit) return { exitCode: Number(exit[1]) };
  const stopped = /^\[the command was stopped after ([\d.]+) seconds\]$/.exec(lines[last]);
  if (stopped) return { stoppedAfter: Number(stopped[1]) };
  return null;
}

/** Say how a call of a tool ended, or where it stands. */
export function classifyTool(item: ToolItem): ToolOutcome {
  // The only yes that a call can be given is that of whoever was asked.
  const allowedByYou = item.approval?.outcome === 'allowed-once';
  const tell = (ending: ToolEnding, next: ActionKey = 'none'): ToolOutcome => ({
    ...ending,
    next,
    allowedByYou,
    code: item.error?.code ?? null,
    detail: item.error?.reason ?? null,
  });

  if (item.result === null) {
    if (item.interrupted) return tell({ kind: 'noResult' });
    const asked = item.approval !== null && item.approval.outcome === null;
    return tell({ kind: asked ? 'waitingForApproval' : 'running' });
  }

  if (item.error === null) {
    const end = SHELLS.has(item.name) ? commandEnd(item.result) : null;
    if (end && 'stoppedAfter' in end) {
      return tell({ kind: 'timedOut', of: 'command', seconds: end.stoppedAfter });
    }
    if (end && end.exitCode !== 0) return tell({ kind: 'exited', exitCode: end.exitCode });
    return tell({ kind: 'done' });
  }

  const rule = item.error.code === 'APPROVAL_REJECTED'
    ? rejected(item)
    // A code that is not known here is a failure all the same: the result holds an error.
    : lookup(TOOL_CODES, item.error.code) ?? failed('error');
  return tell(rule.ending, rule.next);
}

// ---------------------------------------------------------------------------
// A turn

/** What a turn has a budget of. */
type Limited = 'steps' | 'tokens' | 'time';

export type TurnEnding =
  | { kind: 'completed' }
  | { kind: 'stopped'; by: 'you' | 'parent' | 'hook' | 'budget' | 'closing' | 'unknown' }
  /**
   * The turn reached what it may take at most. `limit` is steps, tokens or
   * seconds, or null where the log names the limit and does not say how much it was.
   */
  | { kind: 'limit'; of: Limited; limit: number | null }
  /** A hook of the user stopped what the user said. `said` is the reason that the hook gave. */
  | { kind: 'inputStopped'; said: string }
  | { kind: 'blocked' }
  | { kind: 'modelFailed'; failure: ModelFailure }
  /** Something other than a request to the model failed in the service. */
  | { kind: 'crashed' }
  /** A reply was cut off because the model had written as much as it may. */
  | { kind: 'outputLimit' }
  /** The program ended while the turn was under way. */
  | { kind: 'interrupted' }
  /** The session was copied at a point in the middle of the turn. */
  | { kind: 'forked' }
  | { kind: 'unknown' };

export type TurnOutcome = TurnEnding & {
  next: ActionKey;
  /** For a view of the details: the reason and the detail as the log has them. */
  reason: string;
  detail: string | null;
};

/** Who cancelled a turn: the detail of `aborted` (`agent/agent.py`). */
const STOPPED_BY: Record<string, 'you' | 'parent' | 'hook' | 'budget' | 'closing'> = {
  user: 'you',
  // The agent that handed a task to a subagent stopped waiting for it.
  parent: 'parent',
  hook: 'hook',
  budget: 'budget',
  // The agent was ended while its turn ran: the session was closed, or the program left.
  disposed: 'closing',
};

/** The limits of the budget of a turn, by the codes that it ends a turn with (`guard/budget.py`). */
const LIMIT_CODES: Record<string, Limited> = {
  STEP_BUDGET: 'steps',
  TOKEN_BUDGET: 'tokens',
  TIME_BUDGET: 'time',
};

/** The code of a turn that a hook of the user kept from beginning (`hooks/__init__.py`). */
const STOPPED_BY_A_HOOK = 'HOOK';

/** The code of a turn that ended with an error of the program, and not of a model (`agent/events.py`). */
const EXCEPTION = 'EXCEPTION';

// The sentences of the budget of a turn, word for word. Each says how much the limit was.
const LIMIT_SENTENCES: [of: Limited, said: RegExp][] = [
  ['steps', /^the turn has taken its (\d+) steps?$/],
  ['tokens', /^the turn has used \d+ tokens, and it has (\d+)$/],
  ['time', /^the turn has run for \d+ seconds, and it has ([\d.e+]+)$/],
];

// The words that the program puts before what a hook said.
const HOOK_SENTENCE = /^a hook stopped the input: ([\s\S]*)$/;

/** Read out of a sentence of the budget which limit it speaks of and how much that was. */
function limitSaid(sentence: string): { of: Limited; limit: number } | null {
  for (const [of, said] of LIMIT_SENTENCES) {
    const limit = Number(said.exec(sentence)?.[1]);
    if (Number.isFinite(limit)) return { of, limit };
  }
  return null;
}

/**
 * Say why a turn was not let go on.
 *
 * A turn ends as `blocked` when something refuses its next step. Whoever
 * refused gives a code and a sentence. The code says who it was. Out of the
 * sentence comes only what the code cannot say: how much a limit was, and
 * what a hook gave as its reason. So a sentence that is worded anew one day
 * costs a number, and not what is said of the end. A code that is not known
 * here, of a plugin for instance, is `blocked` and no more, whatever its
 * sentence looks like.
 *
 * A log that was written before ends had a code has only the sentence. There
 * the sentences of the budget and of the hooks are known word for word, and
 * any other sentence is `blocked` and no more.
 */
function blockedBy(detail: string | null, code: string | null): TurnEnding {
  const sentence = detail ?? '';
  const limit = limitSaid(sentence);
  const hook = HOOK_SENTENCE.exec(sentence);
  if (code === null) {
    if (limit) return { kind: 'limit', ...limit };
    if (hook) return { kind: 'inputStopped', said: hook[1] };
    return { kind: 'blocked' };
  }
  const of = lookup(LIMIT_CODES, code);
  if (of) return { kind: 'limit', of, limit: limit?.of === of ? limit.limit : null };
  if (code === STOPPED_BY_A_HOOK) return { kind: 'inputStopped', said: hook ? hook[1] : sentence };
  return { kind: 'blocked' };
}

/**
 * Say how a turn ended.
 *
 * Args:
 *   reason, detail: As `turn/end` has them.
 *   failure: The request that failed last in the turn, if the caller has it.
 *     A turn that ends with an error names the code and the message of that
 *     request in its detail. The failure itself says more.
 *   code: As `turn/end` has it: null where the end has none, and in a log
 *     that was written before ends had a code.
 */
export function classifyTurnEnd(
  reason: string,
  detail: string | null,
  failure: FailureGiven | null = null,
  code: string | null = null,
): TurnOutcome {
  const tell = (ending: TurnEnding, next: ActionKey = 'none'): TurnOutcome => ({ ...ending, next, reason, detail });
  const modelFailed = (of: string, message: string): TurnOutcome => {
    const classified = classifyModelFailure(failure?.code === of ? failure : { code: of, message });
    return tell({ kind: 'modelFailed', failure: classified }, classified.next);
  };

  switch (reason) {
    case 'completed':
      return tell({ kind: 'completed' });
    case 'aborted':
      return tell({ kind: 'stopped', by: lookup(STOPPED_BY, detail ?? '') ?? 'unknown' });
    case 'blocked':
      return tell(blockedBy(detail, code));
    case 'error': {
      const said = detail ?? '';
      if (code === null) {
        // Without a code the sentence has to tell the two apart. A request that failed
        // for good ends the turn with "CODE: message". Anything else that fails ends it
        // with the name of a Python exception, which is not written in capitals
        // throughout (`agent_loop/loop.py`).
        const request = /^([A-Z][A-Z0-9_]*): ([\s\S]*)$/.exec(said);
        return request ? modelFailed(request[1], request[2]) : tell({ kind: 'crashed' }, 'retry');
      }
      if (code === EXCEPTION) return tell({ kind: 'crashed' }, 'retry');
      // The code is that of the request. The message is the detail without the code before it.
      const before = `${code}: `;
      return modelFailed(code, said.startsWith(before) ? said.slice(before.length) : said);
    }
    case 'max-tokens':
      return tell({ kind: 'outputLimit' });
    case 'interrupted':
      return tell({ kind: 'interrupted' });
    case 'forked':
      return tell({ kind: 'forked' });
    default:
      return tell({ kind: 'unknown' });
  }
}

/** Say how a turn ended, or null while it runs or where the log does not hold its end. */
export function turnOutcome(turn: Turn): TurnOutcome | null {
  if (turn.end === null) return null;
  return classifyTurnEnd(turn.end.reason, turn.end.detail, lastFailure(turn), turn.end.code);
}

// The request that failed last in a turn. A turn that ends with an error ends right after it.
function lastFailure(turn: Turn): RequestFailure | null {
  for (let index = turn.steps.length - 1; index >= 0; index -= 1) {
    const item = turn.steps[index];
    if (item.type === 'notice' && item.cause.kind === 'attempt') return item.cause.failure;
  }
  return null;
}
