import { describe, expect, it } from 'vitest';
import {
  classifyModelFailure, classifyTool, classifyTurnEnd, turnOutcome,
  type ActionKey, type ModelFailureKind,
} from '../src/outcomes';
import type { SessionEvent } from '../src/protocol';
import { projectEvents, type NoticeItem, type ToolItem } from '../src/state';
import { groupTurns, type Turn } from '../src/turns';
import { modelFailureCodes, toolCodes, turnBlockedCodes, turnEndReasons } from './appendix';
import { eventsOf, logs } from './logs';

// A log is written here as a list of events. Each stands where it is listed.
type Written = [type: string, data?: Record<string, unknown>];
const log = (...written: Written[]): SessionEvent[] =>
  written.map(([type, data = {}], seq) => ({ seq, type, data, time: 1000 + seq * 100 }));

const text = (words: string) => [{ type: 'text', text: words }];
const asked = (id: string): Written => ['approval/asked', { call_id: id, tool: 'bash', summary: 'bash: make' }];
const decided = (id: string, outcome: string): Written => ['approval/decided', { call_id: id, outcome }];
const policy = (name: string): Written => ['approval/policy', { policy: name }];
const called = (id: string, name = 'bash'): Written => ['tool/call', { call_id: id, name, arguments: '{}' }];
const answered = (id: string, words: string, error: { code: string; reason: string } | null = null): Written =>
  ['tool/result', { message: { role: 'tool', call_id: id, content: text(words), is_error: error !== null }, error }];

const toolsOf = (events: SessionEvent[]): ToolItem[] =>
  projectEvents(events).items.filter((item): item is ToolItem => item.type === 'tool');
const noticesOf = (events: SessionEvent[]): NoticeItem[] =>
  projectEvents(events).items.filter((item): item is NoticeItem => item.type === 'notice');
const turnsOf = (events: SessionEvent[]): Turn[] =>
  groupTurns(projectEvents(events)).filter((entry): entry is Turn => entry.kind === 'turn');

// A call as the projection holds it. By default it has come back with a result.
const call = (given: Partial<ToolItem> = {}): ToolItem => ({
  id: 'event-1', seq: 1, time: 0, turn: 1, type: 'tool', callId: 'a', name: 'read', arguments: '{}',
  result: 'ok', error: null, beganAt: 0, durationMs: 10, trimmed: false, interrupted: false, approval: null,
  ...given,
});
const refusedWith = (code: string, given: Partial<ToolItem> = {}): ToolItem =>
  call({ result: 'What the model reads.', error: { code, reason: 'What the service says of it.' }, ...given });

describe('what the projection keeps of a question about a call', () => {
  it('is nothing for a call that nobody was asked about', () => {
    expect(toolsOf(log(called('a'), answered('a', 'ok')))[0].approval).toBeNull();
  });

  it('is the policy of the session when the question was put, and no answer while it waits', () => {
    const [waiting] = toolsOf(log(policy('ask'), called('a'), asked('a')));
    expect(waiting.approval).toEqual({ outcome: null, policy: 'ask' });
  });

  it('is the answer once the question has one', () => {
    const [allowed, rejected] = toolsOf(log(
      policy('ask'),
      called('a'), asked('a'), decided('a', 'allowed-once'), answered('a', 'ok'),
      called('b'), asked('b'), decided('b', 'rejected'),
    ));
    expect(allowed.approval).toEqual({ outcome: 'allowed-once', policy: 'ask' });
    expect(rejected.approval).toEqual({ outcome: 'rejected', policy: 'ask' });
  });

  it('tells a question that nobody was put from one that a person answered', () => {
    // With the policy `never` the question is written down and answered at once, by nobody.
    const [unasked] = toolsOf(log(policy('never'), called('a'), asked('a'), decided('a', 'rejected')));
    expect(unasked.approval).toEqual({ outcome: 'rejected', policy: 'never' });
  });

  it('names no policy where the log names none, and keeps the one that held for a call', () => {
    const [unnamed, earlier, later] = toolsOf(log(
      called('a'), asked('a'), decided('a', 'rejected'),
      policy('ask'), called('b'), asked('b'), decided('b', 'allowed-once'),
      policy('never'), called('c'), asked('c'), decided('c', 'rejected'),
    ));
    expect(unnamed.approval).toEqual({ outcome: 'rejected', policy: null });
    expect(earlier.approval).toEqual({ outcome: 'allowed-once', policy: 'ask' });
    expect(later.approval).toEqual({ outcome: 'rejected', policy: 'never' });
  });

  it('reads the questions of the sample', () => {
    const calls = new Map(toolsOf(eventsOf('tour')).map(item => [item.callId, item.approval]));
    expect(calls.get('call-1')).toBeNull();
    expect(calls.get('call-6')).toEqual({ outcome: 'allowed-once', policy: 'ask' });
    expect(calls.get('call-8')).toEqual({ outcome: 'rejected', policy: 'ask' });
  });
});

describe('what the projection keeps of a notice', () => {
  it('is the failure of a request that did not get through', () => {
    const [notice] = noticesOf(log(['assistant/attempt', {
      message: { role: 'assistant', content: [] },
      failure: {
        message: 'Too many requests.', code: 'RATE_LIMIT', status: 429, retry_after_ms: 2000, request_id: 'r-1',
      },
    }]));
    expect(notice.cause).toEqual({
      kind: 'attempt',
      failure: {
        code: 'RATE_LIMIT', message: 'Too many requests.', status: 429, retryAfterMs: 2000, requestId: 'r-1',
      },
      // The log named no service before the request.
      provider: null,
    });
    // The line of the earlier interface is as it was.
    expect(notice.text).toBe('Model request failed: Too many requests.');
  });

  it('says that no status came with a failure that had none', () => {
    const [notice] = noticesOf(log(['assistant/attempt', {
      failure: { message: 'the environment variable X is not set', code: 'AUTH', status: null },
    }]));
    expect(notice.cause).toMatchObject({
      kind: 'attempt', failure: { code: 'AUTH', status: null, retryAfterMs: null, requestId: null },
    });
  });

  it('is of the service that the log named last before the request', () => {
    const refused: Written = ['assistant/attempt', { failure: { message: 'refused', code: 'AUTH', status: 401 } }];
    const notices = noticesOf(log(
      ['request/header', { config: { provider: 'deepseek', model: 'deepseek-flash' }, tools: [] }],
      refused,
      refused,
      ['request/header', { config: { provider: 'my-own', model: 'echo-1' }, tools: [] }],
      refused,
      // A header that names no service: of the requests after it, the service is not known.
      ['request/header', { tools: [] }],
      refused,
      ['request/header', { config: { provider: 7 }, tools: [] }],
      refused,
    ));
    expect(notices.map(notice => (notice.cause.kind === 'attempt' ? notice.cause.provider : 'NO ATTEMPT')))
      .toEqual(['deepseek', 'deepseek', 'my-own', null, null]);
  });

  it('is the wait before a request is made again', () => {
    const [notice] = noticesOf(log(['llm/retry', { turn: 1, step: 2, attempt: 3, code: 'SERVER', delay_ms: 4000 }]));
    expect(notice.cause).toEqual({ kind: 'retry', attempt: 3, code: 'SERVER', delayMs: 4000 });
    expect(notice.text).toBe('Retrying request (SERVER) in 4s');
  });

  it('is the end of a turn, and a summary that was not written', () => {
    const notices = noticesOf(log(
      ['turn/start', { turn: 1 }],
      ['turn/end', { turn: 1, reason: 'blocked', detail: 'the turn has taken its 3 steps' }],
      ['compaction/start', { why: 'manual' }],
      ['compaction/end', { why: 'manual', outcome: { error: 'the model wrote no summary' } }],
      ['compaction/start', { why: 'pressure' }],
      ['session/end-seed', { inherited: false }],
    ));
    expect(notices.map(notice => notice.cause)).toEqual([
      { kind: 'turn-end', reason: 'blocked', detail: 'the turn has taken its 3 steps' },
      { kind: 'compaction-failed', detail: 'the model wrote no summary' },
      { kind: 'compaction-interrupted' },
    ]);
  });

  it('reads the notices of the sample', () => {
    expect(noticesOf(eventsOf('tour')).map(notice => notice.cause)).toEqual([
      {
        kind: 'attempt',
        failure: {
          code: 'RATE_LIMIT', message: 'Too many requests. Try again in a moment.',
          status: 429, retryAfterMs: null, requestId: 'req_7f3a',
        },
        provider: 'deepseek',
      },
      { kind: 'retry', attempt: 1, code: 'RATE_LIMIT', delayMs: 20 },
      { kind: 'turn-end', reason: 'aborted', detail: 'user' },
    ]);
  });
});

describe('how a call ended', () => {
  it('runs until its result is there', () => {
    expect(classifyTool(call({ result: null, durationMs: null }))).toMatchObject({
      kind: 'running', next: 'none', code: null, detail: null, allowedByYou: false,
    });
  });

  it('waits for the user while a question about it has no answer', () => {
    const waiting = call({ result: null, approval: { outcome: null, policy: 'ask' } });
    expect(classifyTool(waiting).kind).toBe('waitingForApproval');
    // Once the user has said yes, the call runs.
    const allowed = call({ result: null, approval: { outcome: 'allowed-once', policy: 'ask' } });
    expect(classifyTool(allowed)).toMatchObject({ kind: 'running', allowedByYou: true });
  });

  it('has no result if the turn ended before one was written down', () => {
    expect(classifyTool(call({ result: null, interrupted: true })).kind).toBe('noResult');
  });

  it('is done when its result holds no error', () => {
    expect(classifyTool(call())).toMatchObject({ kind: 'done', allowedByYou: false, next: 'none' });
    const allowed = call({ approval: { outcome: 'allowed-once', policy: 'ask' } });
    expect(classifyTool(allowed)).toMatchObject({ kind: 'done', allowedByYou: true });
  });

  describe('of a command', () => {
    const ran = (output: string, name = 'bash') => classifyTool(call({ name, result: output }));

    it('is done with the exit code 0', () => {
      expect(ran('ok\n[exit code 0]').kind).toBe('done');
      expect(ran('[exit code 0]', 'powershell').kind).toBe('done');
    });

    it('is another exit code where the command gave one', () => {
      expect(ran('1 failed\n[exit code 1]')).toMatchObject({ kind: 'exited', exitCode: 1, code: null });
      expect(ran('[exit code 3221225786]', 'powershell')).toMatchObject({ kind: 'exited', exitCode: 3221225786 });
      expect(ran('killed\n[exit code -9]')).toMatchObject({ kind: 'exited', exitCode: -9 });
    });

    it('reads the line of the exit code where notes about the output follow it', () => {
      const saved = '(The whole output has 70000 bytes and is in /tmp/out.txt. Read a part of it with the read tool.)';
      expect(ran(`tail\n[exit code 2]\n${saved}`)).toMatchObject({ kind: 'exited', exitCode: 2 });
      // The tool may say one thing of the output and the limit on results another.
      const partly = '(Only the first 4096 bytes of the output are saved, in /tmp/out.txt. Read it with the read tool.)';
      expect(ran(`tail\n[exit code 2]\n${partly}\n${saved}`)).toMatchObject({ kind: 'exited', exitCode: 2 });
      expect(ran('tail\n[exit code 2]\n(The whole output could not be saved.)\n')).toMatchObject({ exitCode: 2 });
    });

    it('reads it where a hook has added words of its own', () => {
      const added = '\n\n[A hook of the user adds:]\nformatted 1 file\n[exit code 0]';
      expect(ran(`boom\n[exit code 1]${added}`)).toMatchObject({ kind: 'exited', exitCode: 1 });
    });

    it('goes by the last such line that the tool wrote, not by one in the output', () => {
      expect(ran('the script printed:\n[exit code 7]\nand went on\n[exit code 0]').kind).toBe('done');
    });

    it('timed out where the command was stopped after its time', () => {
      expect(ran('still going\n[the command was stopped after 120 seconds]')).toMatchObject({
        kind: 'timedOut', of: 'command', seconds: 120,
      });
      expect(ran('[the command was stopped after 0.5 seconds]')).toMatchObject({ seconds: 0.5 });
    });

    it('is done where the result does not say how the command ended', () => {
      expect(ran('whatever a plugin made of it').kind).toBe('done');
      expect(ran('').kind).toBe('done');
    });

    it('is read from the result of a tool that runs commands, and of no other', () => {
      expect(classifyTool(call({ name: 'read', result: '   1 [exit code 1]\n[exit code 1]' })).kind).toBe('done');
    });
  });

  // What each code means was read in the sources of the service. See the report of this work.
  const expected: Record<typeof toolCodes[number], Record<string, unknown>> = {
    UNKNOWN_TOOL: { kind: 'unknownTool' },
    INVALID_ARGS: { kind: 'wrongArguments' },
    // Nothing on the call says who was asked. See the cases below.
    APPROVAL_REJECTED: { kind: 'refused', by: 'approval' },
    DENIED: { kind: 'refused', by: 'rule' },
    TOOL_CANCELLED: { kind: 'stopped', because: 'turnStopped' },
    TOOL_TIMEOUT: { kind: 'timedOut', of: 'call', seconds: null },
    TOOL_ERROR: { kind: 'failed', cause: 'error' },
    ABORTED_BEFORE_DISPATCH: { kind: 'notRun', because: 'turnStopped' },
    RESULT_BLOCKED: { kind: 'withheld' },
    SANDBOX_UNAVAILABLE: { kind: 'failed', cause: 'sandboxUnavailable' },
    COMMAND_TOO_LONG: { kind: 'failed', cause: 'commandTooLong' },
    NOT_FOUND: { kind: 'failed', cause: 'notFound' },
    NOT_A_FILE: { kind: 'failed', cause: 'notAFile' },
    NOT_A_DIRECTORY: { kind: 'failed', cause: 'notADirectory' },
    NOT_TEXT: { kind: 'failed', cause: 'notText' },
    TOO_LARGE: { kind: 'failed', cause: 'tooLarge' },
    NOT_READ: { kind: 'failed', cause: 'notRead' },
    CHANGED_SINCE_READ: { kind: 'failed', cause: 'changedSinceRead' },
    NO_MATCH: { kind: 'failed', cause: 'noMatch' },
    AMBIGUOUS: { kind: 'failed', cause: 'ambiguous' },
    NO_CHANGE: { kind: 'failed', cause: 'noChange' },
    READ_ONLY: { kind: 'refused', by: 'readOnly' },
    PROTECTED: { kind: 'refused', by: 'protected' },
    INVALID_PATH: { kind: 'failed', cause: 'invalidPath' },
    INVALID_PATTERN: { kind: 'failed', cause: 'invalidPattern' },
    CHECK_FAILED: { kind: 'failed', cause: 'checkFailed' },
    MOVED: { kind: 'failed', cause: 'moved' },
    SEARCH_FAILED: { kind: 'failed', cause: 'searchFailed' },
    SEARCH_TIMEOUT: { kind: 'timedOut', of: 'search', seconds: null },
  };

  it.each(toolCodes)('knows the code %s of the design', (code) => {
    const outcome = classifyTool(refusedWith(code));
    expect(outcome).toMatchObject(expected[code]);
    // The code and the words of the service stay at hand for a view of the details.
    expect(outcome).toMatchObject({ code, detail: 'What the service says of it.' });
  });

  it.each([
    // The service gives these as well. The design does not list them.
    ['HOOK_BLOCKED', { kind: 'refused', by: 'hook' }],
    ['APPROVAL_UNAVAILABLE', { kind: 'refused', by: 'nobody' }],
    ['APPROVAL_CANCELLED', { kind: 'notRun', because: 'questionWithdrawn' }],
    ['OUTSIDE_WORKING_DIRECTORY', { kind: 'refused', by: 'outside' }],
    ['NOT_WRITABLE', { kind: 'refused', by: 'notWritable' }],
    ['RULES_UNREADABLE', { kind: 'refused', by: 'rulesUnreadable' }],
    ['RULES_NOT_READ', { kind: 'refused', by: 'rulesNotRead' }],
    ['WORKING_DIRECTORY_MOVED', { kind: 'failed', cause: 'projectMoved' }],
    ['TOOL_NOT_STARTED', { kind: 'notRun', because: 'programEnded' }],
    ['TOOL_OUTCOME_UNKNOWN', { kind: 'stopped', because: 'programEnded' }],
  ])('knows the code %s, which the service gives as well', (code, outcome) => {
    expect(classifyTool(refusedWith(code))).toMatchObject({ ...outcome, code });
  });

  it('takes a code that it does not know for a failure, and keeps what the service said', () => {
    expect(classifyTool(refusedWith('SUBAGENT_FAILED'))).toMatchObject({
      kind: 'failed', cause: 'error', code: 'SUBAGENT_FAILED', detail: 'What the service says of it.', next: 'none',
    });
  });

  describe('that was not allowed', () => {
    const rejected = (policyThen: string | null) =>
      classifyTool(refusedWith('APPROVAL_REJECTED', { approval: { outcome: 'rejected', policy: policyThen } }));

    it('was declined by the user where the session asks', () => {
      expect(rejected('ask')).toMatchObject({ kind: 'declined', next: 'none', allowedByYou: false });
    });

    it('was refused without a question where the session asks nobody', () => {
      // The code is the same. Nobody said no: the policy of the session did.
      expect(rejected('never')).toMatchObject({ kind: 'refused', by: 'policy', next: 'openPermissions' });
    });

    it('is not laid at the door of the user where the log does not say who was asked', () => {
      expect(rejected(null)).toMatchObject({ kind: 'refused', by: 'approval' });
    });
  });

  it('names the permissions as what can be changed where they stood in the way', () => {
    const next = (code: string): ActionKey => classifyTool(refusedWith(code)).next;
    expect(['READ_ONLY', 'NOT_WRITABLE', 'OUTSIDE_WORKING_DIRECTORY'].map(next)).toEqual([
      'openPermissions', 'openPermissions', 'openPermissions',
    ]);
    // Nothing in the settings lets a session write into its repository or leave a rule aside.
    expect(['PROTECTED', 'DENIED', 'NO_MATCH', 'TOOL_ERROR'].map(next)).toEqual(['none', 'none', 'none', 'none']);
  });

  it.each(logs)('reads every call of %s', (name) => {
    const outcomes = toolsOf(eventsOf(name)).map(item => [item.callId, classifyTool(item).kind]);
    const notDone = outcomes.filter(([, kind]) => kind !== 'done');
    // In the sample the user did not allow one commit. Every other call of the three logs is done.
    expect(notDone).toEqual(name === 'tour' ? [['call-8', 'declined']] : []);
  });

  it('says of the calls of the sample which ones the user allowed', () => {
    const allowed = toolsOf(eventsOf('tour')).filter(item => classifyTool(item).allowedByYou);
    expect(allowed.map(item => item.callId)).toEqual(['call-6', 'call-10', 'call-11']);
  });
});

describe('why a request to the model failed', () => {
  const expected: Record<typeof modelFailureCodes[number], [ModelFailureKind, ActionKey]> = {
    // The code alone does not say whether a key is missing or was refused. See below.
    AUTH: ['keyProblem', 'openModelSettings'],
    QUOTA: ['quota', 'openModelSettings'],
    RATE_LIMIT: ['rateLimit', 'retry'],
    CONTEXT_WINDOW_EXCEEDED: ['contextTooLong', 'compact'],
    CONTENT_FILTER: ['contentFilter', 'none'],
    INVALID_REQUEST: ['invalidRequest', 'openModelSettings'],
    // Asking again or choosing another model changes nothing: a new conversation does.
    THINKING_REFUSED: ['thinkingRefused', 'none'],
    SERVER: ['server', 'retry'],
    TIMEOUT: ['timeout', 'retry'],
    TRANSPORT: ['unreachable', 'retry'],
    STREAM_CLOSED: ['cutOff', 'retry'],
    MALFORMED_RESPONSE: ['malformed', 'retry'],
    EMPTY_RESPONSE: ['empty', 'retry'],
    INTERNAL: ['internal', 'retry'],
  };

  it.each(modelFailureCodes)('knows the code %s of the design', (code) => {
    const [kind, next] = expected[code];
    expect(classifyModelFailure({ code, message: 'What the service says of it.' })).toEqual({
      kind, next, code, detail: 'What the service says of it.',
    });
  });

  it('tells a key that is missing from a key that the service refused', () => {
    // No status came with the failure: the service was never asked.
    expect(classifyModelFailure({ code: 'AUTH', status: null }).kind).toBe('noKey');
    expect(classifyModelFailure({ code: 'AUTH', status: 401 }).kind).toBe('keyRefused');
    expect(classifyModelFailure({ code: 'AUTH', status: 403 }).kind).toBe('keyRefused');
    // Where it is not known whether a status came, neither is said.
    expect(classifyModelFailure({ code: 'AUTH' }).kind).toBe('keyProblem');
  });

  it('takes a code that it does not know for a failure that can be tried again', () => {
    expect(classifyModelFailure({ code: 'HTTP_418', message: "I'm a teapot" })).toEqual({
      kind: 'unknown', next: 'retry', code: 'HTTP_418', detail: "I'm a teapot",
    });
    expect(classifyModelFailure({ code: '' })).toMatchObject({ kind: 'unknown', detail: null });
  });
});

describe('how a turn ended', () => {
  it.each(turnEndReasons)('knows the reason %s of the design', (reason) => {
    const expected: Record<typeof turnEndReasons[number], Record<string, unknown>> = {
      completed: { kind: 'completed' },
      aborted: { kind: 'stopped', by: 'unknown' },
      blocked: { kind: 'blocked' },
      error: { kind: 'crashed' },
      'max-tokens': { kind: 'outputLimit' },
      interrupted: { kind: 'interrupted' },
      forked: { kind: 'forked' },
    };
    expect(classifyTurnEnd(reason, null)).toMatchObject({ ...expected[reason], reason, detail: null });
  });

  it.each([
    ['user', 'you'], ['parent', 'parent'], ['hook', 'hook'], ['budget', 'budget'], ['disposed', 'closing'],
    // The replay of the first interface writes a sentence here.
    ['You stopped the replay.', 'unknown'],
  ])('says who stopped a turn that was stopped: %s', (detail, by) => {
    expect(classifyTurnEnd('aborted', detail)).toMatchObject({ kind: 'stopped', by, next: 'none', detail });
  });

  it('says which limit a turn reached', () => {
    // These are the sentences of the budget of a turn, word for word.
    expect(classifyTurnEnd('blocked', 'the turn has taken its 30 steps')).toMatchObject({
      kind: 'limit', of: 'steps', limit: 30, next: 'none',
    });
    expect(classifyTurnEnd('blocked', 'the turn has taken its 1 step')).toMatchObject({ of: 'steps', limit: 1 });
    expect(classifyTurnEnd('blocked', 'the turn has used 200341 tokens, and it has 200000')).toMatchObject({
      kind: 'limit', of: 'tokens', limit: 200000,
    });
    expect(classifyTurnEnd('blocked', 'the turn has run for 601 seconds, and it has 600')).toMatchObject({
      kind: 'limit', of: 'time', limit: 600,
    });
    expect(classifyTurnEnd('blocked', 'the turn has run for 2 seconds, and it has 1.5')).toMatchObject({ limit: 1.5 });
  });

  it('says that a hook stopped what the user said, with the reason of the hook', () => {
    expect(classifyTurnEnd('blocked', 'a hook stopped the input: no deploys\non a Friday')).toMatchObject({
      kind: 'inputStopped', said: 'no deploys\non a Friday', next: 'none',
    });
  });

  it('says no more than that a turn was not let go on where it does not know the sentence', () => {
    expect(classifyTurnEnd('blocked', 'a listener of agent/pre-step gave no decision')).toMatchObject({
      kind: 'blocked', detail: 'a listener of agent/pre-step gave no decision',
    });
    expect(classifyTurnEnd('blocked', 'the turn has taken its many steps').kind).toBe('blocked');
  });

  it('reads the failure of the model out of a turn that ended with one', () => {
    const outcome = classifyTurnEnd('error', 'RATE_LIMIT: Too many requests.\nTry again in a moment.');
    expect(outcome).toMatchObject({
      kind: 'modelFailed',
      next: 'retry',
      failure: { kind: 'rateLimit', code: 'RATE_LIMIT', detail: 'Too many requests.\nTry again in a moment.' },
    });
    expect(classifyTurnEnd('error', 'CONTEXT_WINDOW_EXCEEDED: too long')).toMatchObject({ next: 'compact' });
    expect(classifyTurnEnd('error', 'HTTP_418: teapot')).toMatchObject({ failure: { kind: 'unknown' } });
  });

  it('goes by the failure itself where the caller has it', () => {
    const failure = { code: 'AUTH', message: 'Incorrect API key provided.', status: 401 };
    expect(classifyTurnEnd('error', 'AUTH: Incorrect API key provided.', failure)).toMatchObject({
      kind: 'modelFailed', failure: { kind: 'keyRefused' }, next: 'openModelSettings',
    });
    // A failure that is not the one the turn ended with is left aside.
    expect(classifyTurnEnd('error', 'SERVER: down', failure)).toMatchObject({ failure: { kind: 'server' } });
  });

  it('takes any other error for a fault of the program', () => {
    expect(classifyTurnEnd('error', 'TypeError: the source of context gave 3, which is no text')).toMatchObject({
      kind: 'crashed', next: 'retry',
    });
    expect(classifyTurnEnd('error', 'RuntimeError').kind).toBe('crashed');
  });

  it('does not make up an end for a reason that it does not know', () => {
    expect(classifyTurnEnd('paused', 'until tomorrow')).toEqual({
      kind: 'unknown', next: 'none', reason: 'paused', detail: 'until tomorrow',
    });
    // A code does not make a reason known.
    expect(classifyTurnEnd('paused', 'until tomorrow', null, 'STEP_BUDGET').kind).toBe('unknown');
  });

  describe('by the code that the log has with it', () => {
    it.each(turnBlockedCodes)('knows the code %s of a turn that was not let go on', (code) => {
      const expected: Record<typeof turnBlockedCodes[number], Record<string, unknown>> = {
        STEP_BUDGET: { kind: 'limit', of: 'steps', limit: null },
        TOKEN_BUDGET: { kind: 'limit', of: 'tokens', limit: null },
        TIME_BUDGET: { kind: 'limit', of: 'time', limit: null },
        HOOK: { kind: 'inputStopped', said: '' },
        NO_DECISION: { kind: 'blocked' },
      };
      // The sentence is not needed for it, and may be worded anew one day.
      expect(classifyTurnEnd('blocked', null, null, code)).toEqual({
        ...expected[code], next: 'none', reason: 'blocked', detail: null,
      });
    });

    it('reads out of the sentence how much the limit was, and no more', () => {
      expect(classifyTurnEnd('blocked', 'the turn has taken its 30 steps', null, 'STEP_BUDGET')).toMatchObject({
        kind: 'limit', of: 'steps', limit: 30,
      });
      expect(classifyTurnEnd('blocked', 'the turn has run for 601 seconds, and it has 600', null, 'TIME_BUDGET'))
        .toMatchObject({ kind: 'limit', of: 'time', limit: 600 });
      expect(classifyTurnEnd('blocked', 'no step is left of the 30', null, 'STEP_BUDGET')).toMatchObject({
        kind: 'limit', of: 'steps', limit: null,
      });
      // The number of one limit is not that of another.
      expect(classifyTurnEnd('blocked', 'the turn has taken its 30 steps', null, 'TOKEN_BUDGET')).toMatchObject({
        kind: 'limit', of: 'tokens', limit: null,
      });
    });

    it('gives what a hook said, without the words that the program puts before it', () => {
      expect(classifyTurnEnd('blocked', 'a hook stopped the input: no deploys\non a Friday', null, 'HOOK'))
        .toMatchObject({ kind: 'inputStopped', said: 'no deploys\non a Friday' });
      // Where those words are others one day, nothing is cut away.
      expect(classifyTurnEnd('blocked', 'stopped by a hook, which said: no', null, 'HOOK')).toMatchObject({
        kind: 'inputStopped', said: 'stopped by a hook, which said: no',
      });
    });

    it('goes by the code and not by the sentence where the two say different things', () => {
      // A plugin refuses in the words of the budget, and with a code of its own.
      expect(classifyTurnEnd('blocked', 'the turn has taken its 30 steps', null, 'ONE_OF_MINE').kind).toBe('blocked');
      expect(classifyTurnEnd('blocked', 'a hook stopped the input: no', null, 'NO_DECISION').kind).toBe('blocked');
      // A code such as this one must not find what every object has.
      expect(classifyTurnEnd('blocked', 'the turn has taken its 30 steps', null, 'toString').kind).toBe('blocked');
    });

    it('tells a fault of the program from a failure of the model', () => {
      // The name of an error may be written in capitals throughout. Its code says what it was.
      expect(classifyTurnEnd('error', 'EOF: the pipe is closed', null, 'EXCEPTION')).toMatchObject({
        kind: 'crashed', next: 'retry',
      });
      expect(classifyTurnEnd('error', 'RATE_LIMIT: Too many requests.', null, 'RATE_LIMIT')).toMatchObject({
        kind: 'modelFailed', next: 'retry', failure: { kind: 'rateLimit', code: 'RATE_LIMIT', detail: 'Too many requests.' },
      });
      // A message that is not written like an error of this program is one of the model all the same.
      expect(classifyTurnEnd('error', 'the service is down', null, 'SERVER')).toMatchObject({
        kind: 'modelFailed', failure: { kind: 'server', code: 'SERVER', detail: 'the service is down' },
      });
      expect(classifyTurnEnd('error', null, null, 'TIMEOUT')).toMatchObject({
        kind: 'modelFailed', failure: { kind: 'timeout', detail: null },
      });
      expect(classifyTurnEnd('error', 'HTTP_418: teapot', null, 'HTTP_418')).toMatchObject({
        failure: { kind: 'unknown', detail: 'teapot' },
      });
    });

    it('goes by the failure itself where the caller has the one of that code', () => {
      const failure = { code: 'AUTH', message: 'Incorrect API key provided.', status: 401 };
      expect(classifyTurnEnd('error', 'AUTH: Incorrect API key provided.', failure, 'AUTH')).toMatchObject({
        kind: 'modelFailed', failure: { kind: 'keyRefused' }, next: 'openModelSettings',
      });
      expect(classifyTurnEnd('error', 'SERVER: down', failure, 'SERVER')).toMatchObject({ failure: { kind: 'server' } });
    });
  });
});

describe('the outcome of a turn', () => {
  it('is none while the turn runs', () => {
    const [turn] = turnsOf(log(['turn/start', { turn: 1 }]));
    expect(turnOutcome(turn)).toBeNull();
  });

  it('knows the failure that a turn ended with from the request that failed last', () => {
    const attempt = (failure: Record<string, unknown>): Written => ['assistant/attempt', { failure }];
    const [refused, missing] = turnsOf(log(
      ['turn/start', { turn: 1 }],
      attempt({ code: 'SERVER', message: 'down', status: 503 }),
      attempt({ code: 'AUTH', message: 'Incorrect API key provided.', status: 401 }),
      ['turn/end', { turn: 1, reason: 'error', detail: 'AUTH: Incorrect API key provided.' }],
      ['turn/start', { turn: 2 }],
      attempt({ code: 'AUTH', message: 'the environment variable X is not set', status: null }),
      ['turn/end', { turn: 2, reason: 'error', detail: 'AUTH: the environment variable X is not set' }],
    ));
    expect(turnOutcome(refused)).toMatchObject({ kind: 'modelFailed', failure: { kind: 'keyRefused' } });
    expect(turnOutcome(missing)).toMatchObject({ kind: 'modelFailed', failure: { kind: 'noKey' } });
  });

  it('goes by the code that the log has with the end of a turn', () => {
    const [limited, crashed, old] = turnsOf(log(
      ['turn/start', { turn: 1 }],
      ['turn/end', { turn: 1, reason: 'blocked', detail: 'no step is left', code: 'STEP_BUDGET' }],
      ['turn/start', { turn: 2 }],
      ['turn/end', { turn: 2, reason: 'error', detail: 'EOF: the pipe is closed', code: 'EXCEPTION' }],
      // A log from before an end had a code: the sentence is all there is.
      ['turn/start', { turn: 3 }],
      ['turn/end', { turn: 3, reason: 'blocked', detail: 'the turn has taken its 30 steps' }],
    ));
    expect(turnOutcome(limited)).toMatchObject({ kind: 'limit', of: 'steps', limit: null });
    expect(turnOutcome(crashed)).toMatchObject({ kind: 'crashed' });
    expect(turnOutcome(old)).toMatchObject({ kind: 'limit', of: 'steps', limit: 30 });
  });

  it('reads the turns of the sample', () => {
    const outcomes = turnsOf(eventsOf('tour')).map(turn => turnOutcome(turn));
    expect(outcomes.map(outcome => outcome?.kind)).toEqual(['completed', 'stopped', 'completed', 'completed']);
    expect(outcomes[1]).toMatchObject({ kind: 'stopped', by: 'you' });
  });
});
