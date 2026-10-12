import { describe, expect, it } from 'vitest';
import { messagesFor } from '../src/i18n';
import { splitDuration } from '../src/messages/duration';
import { en, type Messages } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import {
  classifyModelFailure, classifyTool, classifyTurnEnd, turnOutcome, type ActionKey,
} from '../src/outcomes';
import { projectEvents, type ToolItem } from '../src/state';
import { groupTurns, type Turn } from '../src/turns';
import {
  sayConfinement, sayModelFailure, sayPermissions, sayTool, sayTurn, type Saying,
} from '../src/words';
import {
  approvalPolicies, modelFailureCodes, sandboxModes, sandboxStrengths, toolCodes, turnEndReasons,
} from './appendix';
import { eventsOf, logs } from './logs';

const catalogues: [name: string, messages: Messages][] = [['English', en], ['Chinese', zhCN]];

// A sentence that takes numbers is a function. These are the numbers it is tried with:
// one and many, short and long.
const samples: Record<string, unknown[][]> = {
  'duration': [[0.5], [1], [42], [60], [125], [3600], [3725]],
  'tool.exited': [[1], [137]],
  'tool.timedOut.command': [[0.5], [120]],
  'turn.took': [[1, 1], [42, 3], [125, 12]],
  'turn.limit.steps': [[1], [30]],
  'wait.retry': [[0, 3], [1, 1], [4, 2]],
  'wait.retried': [[0, 3], [1, 1], [4, 2]],
  'label.ago.minutes': [[1], [45]],
  'label.ago.hours': [[1], [23]],
  'label.ago.days': [[1], [30]],
  'label.context': [[0], [87]],
  'label.verb.other': [['fetch']],
  'hint.starting': [['F:\\work\\project']],
  'hint.consentOther': [['fetch']],
  'hint.scope.anyCall': [['fetch']],
  'hint.scope.onTarget': [['peek', 'notes.txt']],
  'hint.contextOf': [['12.4K', '128K']],
  'hint.contextUnknown': [['950']],
  'hint.about': [['0.1.0', 1]],
};

type Said = { path: string; text: string };

// Every string that a catalogue can say, with the place where it stands.
function everythingSaid(value: unknown, path = ''): Said[] {
  if (typeof value === 'string') return [{ path, text: value }];
  if (typeof value === 'function') {
    const tried = samples[path];
    if (!tried) throw new Error(`The tests have no numbers to try ${path} with.`);
    return tried.flatMap(given => everythingSaid((value as (...given: unknown[]) => unknown)(...given), path));
  }
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).flatMap(([key, inner]) => everythingSaid(inner, path ? `${path}.${key}` : key));
  }
  throw new Error(`${path} is neither words nor a place for words.`);
}

// What a catalogue is made of: its keys, and at each key words, a place for words, or a function.
function shapeOf(value: unknown, path = ''): unknown {
  if (typeof value === 'string') return 'words';
  if (typeof value === 'function') {
    const [given] = samples[path] ?? [[]];
    const made = (value as (...given: unknown[]) => unknown)(...given);
    return { takes: value.length, gives: shapeOf(made, path) };
  }
  return Object.fromEntries(
    Object.entries(value as object).map(([key, inner]) => [key, shapeOf(inner, path ? `${path}.${key}` : key)]),
  );
}

const call = (given: Partial<ToolItem> = {}): ToolItem => ({
  id: 'event-1', seq: 1, time: 0, turn: 1, type: 'tool', callId: 'a', name: 'bash', arguments: '{}',
  result: 'ok', error: null, durationMs: 10, trimmed: false, interrupted: false, approval: null,
  ...given,
});
const failedWith = (code: string, given: Partial<ToolItem> = {}): ToolItem =>
  call({ result: 'What the model reads.', error: { code, reason: 'What the service says of it.' }, ...given });

const hasWords = (saying: Saying) => {
  expect(saying.what.trim()).not.toBe('');
  if ('why' in saying) expect(saying.why?.trim()).not.toBe('');
};

describe('the two catalogues', () => {
  it('have the same shape', () => {
    expect(shapeOf(zhCN)).toEqual(shapeOf(en));
  });

  it('are tried with numbers wherever a sentence takes some', () => {
    const functions = (value: unknown, path = ''): string[] => {
      if (typeof value === 'function') return [path];
      if (typeof value !== 'object' || value === null) return [];
      return Object.entries(value).flatMap(([key, inner]) => functions(inner, path ? `${path}.${key}` : key));
    };
    expect(functions(en).sort()).toEqual(Object.keys(samples).sort());
  });

  describe.each(catalogues)('in %s', (_name, messages) => {
    const said = everythingSaid(messages);

    it('say something at every place', () => {
      expect(said.filter(({ text }) => text.trim() === '')).toEqual([]);
      expect(said.length).toBeGreaterThan(80);
    });

    it('hold no code of the service', () => {
      // A code is written in capitals. Most have an underscore, and the others are listed.
      const withUnderscore = /[A-Z][A-Z0-9]*_[A-Z0-9_]+/;
      const listed = new RegExp(`\\b(${[...modelFailureCodes, ...toolCodes].join('|')})\\b`);
      expect(said.filter(({ text }) => withUnderscore.test(text) || listed.test(text))).toEqual([]);
    });

    it('hold no value of the protocol and no name of an event', () => {
      const values = [
        ...sandboxModes, 'full-access', 'allowed-once', 'max-tokens', 'tool-calls', 'next-turn', 'next-step',
        'in_progress', 'aborted', 'disposed',
      ];
      // An event is named with a slash, as in `turn/end`.
      const event = /[a-z]+\/[a-z]+/;
      const found = said.filter(({ text }) => event.test(text) || values.some(value => text.includes(value)));
      expect(found).toEqual([]);
    });

    it('stay calm', () => {
      const loud = /[!！]|oops|successfully|成功/i;
      expect(said.filter(({ text }) => loud.test(text))).toEqual([]);
    });

    it('have no blanks at their ends and none doubled', () => {
      expect(said.filter(({ text }) => text !== text.trim() || /\s{2,}/.test(text))).toEqual([]);
    });
  });

  it('end a sentence with a full stop, and a line that is no sentence without one', () => {
    // In the row of a call, `what` is a few words. The name of an action is the
    // face of a button, and so is a label. A duration and the time that a turn took
    // are parts of a line.
    const isSentence = (path: string) => path.endsWith('.why') || !(
      path.startsWith('tool.') || path.startsWith('actions.') || path.startsWith('label.')
      || path === 'duration' || path === 'turn.took'
    );
    for (const [, messages] of catalogues) {
      const stop = messages === en ? '.' : '。';
      const said = everythingSaid(messages);
      expect(said.filter(({ path, text }) => isSentence(path) && !text.endsWith(stop))).toEqual([]);
      expect(said.filter(({ path, text }) => !isSentence(path) && /[.。]$/.test(text))).toEqual([]);
    }
  });

  it('keep the short line of a call short', () => {
    const lines = (messages: Messages): string[] => {
      const found: string[] = [];
      const visit = (value: unknown, path: string) => {
        if (typeof value === 'function') {
          for (const given of samples[path]) visit((value as (...given: unknown[]) => unknown)(...given), path);
        } else if (typeof value === 'object' && value !== null) {
          if ('what' in value) found.push((value as Saying).what);
          else for (const [key, inner] of Object.entries(value)) visit(inner, `${path}.${key}`);
        }
      };
      visit(messages.tool, 'tool');
      return found;
    };
    expect(lines(en).filter(line => line.length > 30)).toEqual([]);
    expect(lines(zhCN).filter(line => line.length > 10)).toEqual([]);
  });

  it('are written as Chinese is written, in the Chinese one', () => {
    const said = everythingSaid(zhCN);
    const chinese = '[\\u4e00-\\u9fff]';
    // No blank between two Chinese characters, and no punctuation of half width beside one.
    const blank = new RegExp(`${chinese}\\s+${chinese}`);
    const halfWidth = new RegExp(`${chinese}\\s*[,;:?()]|[,;:?()]\\s*${chinese}|${chinese}\\.(\\s|$)`);
    expect(said.filter(({ text }) => blank.test(text))).toEqual([]);
    expect(said.filter(({ text }) => halfWidth.test(text))).toEqual([]);
    // The user is addressed as 你.
    expect(said.filter(({ text }) => text.includes('您'))).toEqual([]);
    // Every sentence is in Chinese: none was left as it is in English.
    const english = new Set(everythingSaid(en).map(({ text }) => text));
    expect(said.filter(({ text }) => english.has(text))).toEqual([]);
    expect(said.filter(({ text }) => !new RegExp(chinese).test(text))).toEqual([]);
  });
});

describe('picking a catalogue', () => {
  it.each(['zh-CN', 'zh', 'zh_CN', 'ZH-cn', 'zh-Hans', 'zh-Hans-CN', 'zh-SG', 'zh-TW'])(
    'gives the Chinese one for %s', (locale) => {
      expect(messagesFor(locale)).toBe(zhCN);
    },
  );

  it.each(['en', 'en-US', 'en_GB', 'ja', 'ja-JP', 'fr-FR', 'zhuang', '', 'C'])(
    'gives the English one for %s', (locale) => {
      expect(messagesFor(locale)).toBe(en);
    },
  );
});

describe('the parts of a duration', () => {
  it('are hours, minutes and seconds', () => {
    expect(splitDuration(42)).toEqual({ hours: 0, minutes: 0, seconds: 42 });
    expect(splitDuration(125)).toEqual({ hours: 0, minutes: 2, seconds: 5 });
    expect(splitDuration(3725)).toEqual({ hours: 1, minutes: 2, seconds: 5 });
  });

  it('keep a tenth of a second below ten seconds, and whole seconds above', () => {
    expect(splitDuration(0.52).seconds).toBe(0.5);
    expect(splitDuration(9.96).seconds).toBe(10);
    expect(splitDuration(41.6).seconds).toBe(42);
    // Rounding does not make sixty seconds of fifty-nine and a half.
    expect(splitDuration(59.6)).toEqual({ hours: 0, minutes: 1, seconds: 0 });
  });

  it('are nothing for a time that is no time', () => {
    expect(splitDuration(-3)).toEqual({ hours: 0, minutes: 0, seconds: 0 });
    expect(splitDuration(Number.NaN)).toEqual({ hours: 0, minutes: 0, seconds: 0 });
  });

  it('are said in words', () => {
    expect([0.5, 1, 42, 60, 125, 3600, 3725].map(en.duration)).toEqual([
      '0.5 s', '1 s', '42 s', '1 min', '2 min 5 s', '1 h', '1 h 2 min',
    ]);
    expect([0.5, 1, 42, 60, 125, 3600, 3725].map(zhCN.duration)).toEqual([
      '0.5 秒', '1 秒', '42 秒', '1 分钟', '2 分 5 秒', '1 小时', '1 小时 2 分',
    ]);
  });
});

describe.each(catalogues)('what is said in %s', (_name, messages) => {
  it.each(modelFailureCodes)('has words for the failure %s of a request', (code) => {
    for (const status of [undefined, null, 401]) {
      hasWords(sayModelFailure(messages, classifyModelFailure({ code, status })));
    }
  });

  it('has words for a failure that it does not know', () => {
    expect(sayModelFailure(messages, classifyModelFailure({ code: 'HTTP_418' }))).toBe(messages.model.unknown);
  });

  it.each(toolCodes)('has words for the code %s of a call', (code) => {
    hasWords(sayTool(messages, classifyTool(failedWith(code))));
  });

  it('has words for a call that was not allowed, whoever did not allow it', () => {
    const said = (policy: string | null) =>
      sayTool(messages, classifyTool(failedWith('APPROVAL_REJECTED', { approval: { outcome: 'rejected', policy } })));
    expect(said('ask')).toBe(messages.tool.declined);
    expect(said('never')).toBe(messages.tool.refused.policy);
    expect(said(null)).toBe(messages.tool.refused.approval);
  });

  it.each([
    'HOOK_BLOCKED', 'APPROVAL_UNAVAILABLE', 'APPROVAL_CANCELLED', 'OUTSIDE_WORKING_DIRECTORY', 'NOT_WRITABLE',
    'RULES_UNREADABLE', 'RULES_NOT_READ', 'WORKING_DIRECTORY_MOVED', 'TOOL_NOT_STARTED', 'TOOL_OUTCOME_UNKNOWN',
    'A_CODE_OF_TOMORROW',
  ])('has words for the code %s, which the design does not list', (code) => {
    hasWords(sayTool(messages, classifyTool(failedWith(code))));
  });

  it('has words for a call that has not ended, and for one that went well', () => {
    expect(sayTool(messages, classifyTool(call({ result: null })))).toBe(messages.tool.running);
    const waiting = call({ result: null, approval: { outcome: null, policy: 'ask' } });
    expect(sayTool(messages, classifyTool(waiting))).toBe(messages.tool.waitingForApproval);
    expect(sayTool(messages, classifyTool(call({ result: null, interrupted: true })))).toBe(messages.tool.noResult);
    expect(sayTool(messages, classifyTool(call()))).toBe(messages.tool.done);
  });

  it('names the exit code of a command, and the time after which one was stopped', () => {
    const exited = sayTool(messages, classifyTool(call({ result: 'no\n[exit code 3]' })));
    expect(exited).toEqual(messages.tool.exited(3));
    expect(exited.what).toContain('3');
    const stopped = sayTool(messages, classifyTool(call({ result: '[the command was stopped after 120 seconds]' })));
    expect(stopped).toEqual(messages.tool.timedOut.command(120));
    expect(stopped.why).toContain(messages.duration(120));
  });

  it.each(turnEndReasons)('has words for a turn that ended as %s', (reason) => {
    const said = sayTurn(messages, classifyTurnEnd(reason, null));
    // A turn that completed did not go wrong. The line about it says how long it took.
    if (reason === 'completed') expect(said).toBeNull();
    else hasWords(said!);
  });

  it.each([
    ['aborted', 'user'], ['aborted', 'parent'], ['aborted', 'hook'], ['aborted', 'budget'], ['aborted', 'disposed'],
    ['blocked', 'the turn has taken its 30 steps'],
    ['blocked', 'the turn has used 200341 tokens, and it has 200000'],
    ['blocked', 'the turn has run for 601 seconds, and it has 600'],
    ['blocked', 'a hook stopped the input: not on a Friday'],
    ['error', 'RATE_LIMIT: Too many requests.'],
    ['error', 'KeyError: 3'],
    ['paused', null],
  ])('has words for a turn that ended as %s with the detail %s', (reason, detail) => {
    const outcome = classifyTurnEnd(reason, detail);
    const said = sayTurn(messages, outcome)!;
    hasWords(said);
    // A sentence of the service is for a view of the details. No sentence of the interface repeats it.
    if (detail?.includes(' ')) expect(`${said.what} ${said.why ?? ''}`).not.toContain(detail);
  });

  it('says of a turn that the model failed what it says of the failure', () => {
    const outcome = classifyTurnEnd('error', 'SERVER: down');
    expect(sayTurn(messages, outcome)).toBe(messages.model.server);
  });

  it('names the limit of steps that a turn reached', () => {
    const said = sayTurn(messages, classifyTurnEnd('blocked', 'the turn has taken its 30 steps'))!;
    expect(said.what).toContain('30');
  });

  it.each(sandboxModes.flatMap(mode => approvalPolicies.map(policy => [mode, policy])))(
    'says what a session may do in the mode %s with the policy %s', (mode, policy) => {
      const said = sayPermissions(messages, mode, policy);
      expect(said.trim()).not.toBe('');
      expect(said).not.toBe(messages.permissions.unknown);
    },
  );

  it('does not say what a session may do where the status does not tell', () => {
    expect(sayPermissions(messages, null, 'ask')).toBe(messages.permissions.unknown);
    expect(sayPermissions(messages, 'workspace-write', null)).toBe(messages.permissions.unknown);
    expect(sayPermissions(messages, 'a mode of tomorrow', 'ask')).toBe(messages.permissions.unknown);
    // With full access no command waits for a yes, whatever the policy.
    expect(sayPermissions(messages, 'danger-full-access', null)).toBe(messages.permissions.fullAccess);
  });

  it.each(sandboxStrengths)('says how well commands are held where the strength is %s', (strength) => {
    expect(sayConfinement(messages, strength)?.trim()).toBeTruthy();
  });

  it('says nothing of a strength that it does not know', () => {
    expect(sayConfinement(messages, null)).toBeNull();
    expect(sayConfinement(messages, 'airtight')).toBeNull();
  });

  it('names every action that can come next', () => {
    const actions: Exclude<ActionKey, 'none'>[] = ['retry', 'openModelSettings', 'openPermissions', 'compact'];
    for (const action of actions) expect(messages.actions[action].trim()).not.toBe('');
    expect(Object.keys(messages.actions).sort()).toEqual([...actions].sort());
  });

  it.each(logs)('has words for everything that happened in %s', (name) => {
    const projection = projectEvents(eventsOf(name));
    for (const item of projection.items) {
      if (item.type === 'tool') hasWords(sayTool(messages, classifyTool(item)));
      if (item.type === 'notice' && item.cause.kind === 'attempt') {
        hasWords(sayModelFailure(messages, classifyModelFailure(item.cause.failure)));
      }
      if (item.type === 'notice' && item.cause.kind === 'retry') {
        expect(messages.wait.retry(item.cause.delayMs / 1000, item.cause.attempt).trim()).not.toBe('');
      }
    }
    for (const entry of groupTurns(projection)) {
      if (entry.kind !== 'turn') continue;
      const outcome = turnOutcome(entry);
      expect(outcome).not.toBeNull();
      if (outcome!.kind !== 'completed') hasWords(sayTurn(messages, outcome!)!);
    }
  });
});

describe('the words of the design', () => {
  const turnsOf = (name: 'tour'): Turn[] =>
    groupTurns(projectEvents(eventsOf(name))).filter((entry): entry is Turn => entry.kind === 'turn');

  it('are said as the design says them', () => {
    // Section 5.3: a turn that the user stopped, and one that came to its end.
    const [first, second] = turnsOf('tour');
    expect(sayTurn(zhCN, turnOutcome(second)!)).toEqual({ what: '你停下了这个回合。' });
    expect(zhCN.turn.took(42, 3)).toBe('用了 42 秒，3 步');
    expect(zhCN.turn.took((first.end!.time - first.startedAt) / 1000, first.rounds)).toBe('用了 14 秒，5 步');
    // Section 5.3: the wait before a request is made again.
    expect(zhCN.wait.retry(4, 2)).toBe('4 秒后第 2 次重试。');
    // Section 5.4: what a call says once the question about it is answered.
    expect(zhCN.tool.allowedByYou).toBe('你同意了');
    expect(zhCN.tool.declined.what).toBe('你没有同意');
    // Section 5.6: the permissions in a sentence.
    expect(sayPermissions(zhCN, 'workspace-write', 'ask')).toBe('可以修改这个项目里的文件，运行命令之前会问。');
  });

  it('are said in English too', () => {
    const [first, second] = turnsOf('tour');
    expect(sayTurn(en, turnOutcome(second)!)).toEqual({ what: 'You stopped this turn.' });
    expect(en.turn.took((first.end!.time - first.startedAt) / 1000, first.rounds)).toBe('Took 14 s, 5 steps');
    expect(en.turn.took(1, 1)).toBe('Took 1 s, 1 step');
    expect(en.wait.retry(4, 2)).toBe('Trying again in 4 s (retry 2).');
    expect(en.wait.retry(0, 3)).toBe('Trying again now (retry 3).');
    expect(en.tool.exited(1).what).toBe('Exit code 1');
    expect(en.turn.limit.steps(1).what).toBe('This turn reached its limit of 1 step.');
  });
});
