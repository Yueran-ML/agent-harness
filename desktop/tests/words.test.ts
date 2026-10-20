import { describe, expect, it } from 'vitest';
import { messagesFor } from '../src/i18n';
import { splitDuration } from '../src/messages/duration';
import { en, type Messages } from '../src/messages/en';
import { ja } from '../src/messages/ja';
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

const catalogues: [name: string, messages: Messages][] = [['English', en], ['Chinese', zhCN], ['Japanese', ja]];

// A sentence that takes numbers is a function. These are the numbers it is tried with:
// one and many, short and long. A count comes as short as its language writes it, which
// is 12.4K in English and 1.2万 in Chinese and Japanese.
const samples: Record<string, unknown[][]> = {
  'duration': [[0.5], [1], [42], [60], [125], [3600], [3725]],
  'tool.exited': [[1], [137]],
  'tool.timedOut.command': [[0.5], [120]],
  'model.named.noKey': [['DeepSeek'], ['my-own']],
  'model.named.keyRefused': [['DeepSeek'], ['my-own']],
  'model.named.keyProblem': [['DeepSeek'], ['my-own']],
  'label.showAllLines': [[81], [1240]],
  'label.skillGiven': [['review'], [null]],
  'hint.moreSkills': [[1], [3], [12]],
  'turn.took': [[1, 1], [42, 3], [125, 12]],
  'turn.limit.steps': [[1], [30], [null]],
  'wait.retry': [[0, 3], [1, 1], [4, 2]],
  'wait.retried': [[0, 3], [1, 1], [4, 2]],
  'label.ago.minutes': [[1], [45]],
  'label.ago.hours': [[1], [23]],
  'label.ago.days': [[1], [30]],
  'label.context': [[0], [87]],
  'label.verb.other': [['fetch']],
  'hint.starting': [['F:\\work\\project']],
  'hint.reading': [[0, 4200], [1500, 4200]],
  'hint.consentOther': [['fetch']],
  'hint.scope.anyCall': [['fetch']],
  'hint.scope.onTarget': [['peek', 'notes.txt']],
  'hint.contextOf': [['12.4K', '128K'], ['2670', '104.9万'], ['1.2万', '100万']],
  'hint.contextUnknown': [['950'], ['1.2万']],
  'hint.about': [['0.1.0', 1]],
  'label.effort.button': [['High'], ['最高']],
  'label.stats.turns': [[1], [6]],
  'label.stats.steps': [[1], [371]],
  'label.stats.speed': [['313'], ['1,024']],
  'label.stats.tokens': [['94.1M'], ['9410万']],
  'label.stats.cached': [['99.7'], ['0.0']],
  'label.trace.found': [[1, 1], [3, 17]],
  'label.trace.turn': [[1], [12]],
  'label.trace.onlyCalls': [[1], [3]],
  'label.trace.tokens': [['12K', '320'], ['1.2万', '4,096']],
  'hint.trace.step': [
    [{ turn: 1, step: 1, took: null, calls: 0, wrong: 0, failed: 0 }],
    [{ turn: 2, step: 3, took: '2.1 s', calls: 1, wrong: 0, failed: 1 }],
    [{ turn: 12, step: 40, took: '1 min 4 s', calls: 5, wrong: 2, failed: 3 }],
  ],
  'hint.stats': [
    [{ turns: 1, steps: 1, speed: null, tokens: '0', read: '0', cached: null, written: '0' }],
    [{ turns: 6, steps: 371, speed: '313', tokens: '94,123,456', read: '93,800,000', cached: '93,500,000', written: '323,456' }],
  ],
  'label.updates.told': [['0.1.3'], ['12.0.10']],
  'label.updates.toldInFull': [['0.1.3'], ['12.0.10']],
  'hint.updates.version': [['0.1.2'], ['0.1.2-dev']],
  'hint.updates.newer': [['0.1.3'], ['12.0.10']],
  // The name of a variable, which is written in capitals. A name with an underscore in it
  // would be taken for a code of the service by the test below.
  'hint.key.fromVariable': [['MYKEY']],
  'hint.key.done': [['openai', 'gpt-test']],
  'hint.key.reached': [['DeepSeek', 1], ['Ollama', 12]],
  'hint.key.noModels': [['LM Studio']],
  'hint.key.notListed': [['deepseek-old']],
  'hint.key.forgetAsk': [['OpenAI']],
  'hint.key.forgotten': [['OpenAI']],
  'hint.key.noneToForget': [['OpenAI']],
  'label.models.button': [['deepseek-flash'], ['qwen3:8b']],
  'label.models.use': [['deepseek-old']],
  'hint.models.asking': [['DeepSeek'], ['my-own']],
  'hint.models.none': [['LM Studio']],
  'label.attachments.remove': [['notes.md']],
  'hint.attachments.refused.gone': [['notes.md']],
  'hint.attachments.refused.notAFile': [['photos']],
  'hint.attachments.refused.tooBig': [['film.mp4']],
  'hint.attachments.refused.tooMany': [['data.csv']],
  'label.github.left': [['14:59'], ['0:07']],
  'hint.start.noKey': [['DeepSeek'], ['my-own']],
  'hint.github.signedIn': [['octocat']],
  'hint.github.moreRepositories': [[1], [3], [12]],
  'hint.github.signOutAsk': [['octocat']],
  'hint.feishu.forgetAsk': [['cli_a1b2c3d4']],
  'hint.feishu.done': [['cli_a1b2c3d4', 'Lark'], ['cli_a1b2c3d4', 'Feishu']],
  'hint.feishu.connecting': [['Lark'], ['Feishu']],
  'hint.feishu.reconnecting': [['Lark'], ['Feishu']],
  'hint.feishu.connected': [['Lark'], ['Feishu']],
  'hint.feishu.steps': [['Lark']],
  'label.feishu.pair': [['Lark'], ['Feishu']],
};

type Said = { path: string; text: string };

// What is written in every language as it is: the names of the two platforms of Feishu, and the
// names of the fields of an app, as both of their consoles write them.
const NAMES = new Set(['Feishu', 'Lark', 'App ID', 'App Secret']);

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
  result: 'ok', error: null, beganAt: 0, durationMs: 10, trimmed: false, interrupted: false, approval: null,
  ...given,
});
const failedWith = (code: string, given: Partial<ToolItem> = {}): ToolItem =>
  call({ result: 'What the model reads.', error: { code, reason: 'What the service says of it.' }, ...given });

const hasWords = (saying: Saying) => {
  expect(saying.what.trim()).not.toBe('');
  if ('why' in saying) expect(saying.why?.trim()).not.toBe('');
};

describe('the catalogues', () => {
  it('have the same shape', () => {
    expect(shapeOf(zhCN)).toEqual(shapeOf(en));
    expect(shapeOf(ja)).toEqual(shapeOf(en));
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
      // An event is named with a slash, as in `turn/end`. An address of the web has slashes
      // of its own and names no event, and neither does the unit of how fast a model writes.
      const event = /[a-z]+\/[a-z]+/;
      const address = /https?:\/\/[^\s，。]+/g;
      const found = said.filter(({ text }) => event.test(text.replace(address, '').replace(/\btok\/s\b/g, ''))
        || values.some(value => text.includes(value)));
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
    // A word of Japanese that is written in katakana is long.
    expect(lines(ja).filter(line => line.length > 12)).toEqual([]);
  });

  it('are written as Chinese is written, in the Chinese one', () => {
    const said = everythingSaid(zhCN);
    const chinese = '[\\u4e00-\\u9fff]';
    // No blank between two Chinese characters, and no punctuation of half width beside one.
    const blank = new RegExp(`${chinese}\\s+${chinese}`);
    const halfWidth = new RegExp(`${chinese}\\s*[,;:?()]|[,;:?()]\\s*${chinese}|${chinese}\\.(\\s|$)`);
    expect(said.filter(({ text }) => blank.test(text))).toEqual([]);
    expect(said.filter(({ text }) => halfWidth.test(text))).toEqual([]);
    // A blank between Chinese and a number or a word in Latin letters. A count that is
    // written with 万 is one number, and has none inside.
    const touching = new RegExp(`${chinese}[A-Za-z0-9]|[A-Za-z0-9%](?![万亿])${chinese}`);
    expect(said.filter(({ text }) => touching.test(text))).toEqual([]);
    // The user is addressed as 你.
    expect(said.filter(({ text }) => text.includes('您'))).toEqual([]);
    // Every sentence is in Chinese: none was left as it is in English.
    const english = new Set(everythingSaid(en).map(({ text }) => text));
    const translated = said.filter(({ text }) => !NAMES.has(text));
    expect(translated.filter(({ text }) => english.has(text))).toEqual([]);
    expect(translated.filter(({ text }) => !new RegExp(chinese).test(text))).toEqual([]);
  });

  it('are written as Japanese is written, in the Japanese one', () => {
    const said = everythingSaid(ja);
    const kana = '[\\u3040-\\u30ff]';
    const japanese = '[\\u3005\\u3040-\\u30ff\\u4e00-\\u9fff]';
    // No blank between two Japanese characters, and no punctuation of half width beside one.
    const blank = new RegExp(`${japanese}\\s+${japanese}`);
    const halfWidth = new RegExp(`${japanese}\\s*[,;:?()]|[,;:?()]\\s*${japanese}|${japanese}\\.(\\s|$)`);
    expect(said.filter(({ text }) => blank.test(text))).toEqual([]);
    expect(said.filter(({ text }) => halfWidth.test(text))).toEqual([]);
    // A blank between Japanese and a number or a word in Latin letters. A count that is
    // written with 万 is one number, and has none inside.
    const touching = new RegExp(`${japanese}[A-Za-z0-9]|[A-Za-z0-9%](?![万億兆])${japanese}`);
    expect(said.filter(({ text }) => touching.test(text))).toEqual([]);
    // Every line is in Japanese: none was left as it is in English.
    const english = new Set(everythingSaid(en).map(({ text }) => text));
    const translated = said.filter(({ text }) => !NAMES.has(text));
    expect(translated.filter(({ text }) => english.has(text))).toEqual([]);
    expect(translated.filter(({ text }) => !new RegExp(japanese).test(text))).toEqual([]);
    // And none was left as it is in Chinese, which has no kana. A sentence of Japanese has some.
    expect(said.filter(({ text }) => text.endsWith('。') && !new RegExp(kana).test(text))).toEqual([]);
    // A sentence is in the polite form. Those that name a version are no sentences but lines with a stop.
    const polite = /(です|ます|ました|ません|でした|ください)(（[^（）]*）)?$/;
    const versions = ['hint.about', 'hint.updates.version'];
    const sentences = said.filter(({ path, text }) => text.endsWith('。') && !versions.includes(path))
      .flatMap(({ path, text }) => text.split('。').filter(Boolean).map(sentence => ({ path, sentence })));
    expect(sentences.length).toBeGreaterThan(150);
    expect(sentences.filter(({ sentence }) => !polite.test(sentence))).toEqual([]);
    // A word from English that ends in a long vowel keeps its long mark.
    const clipped = /(フォルダ|ブラウザ|コンピュータ|フィルタ|サーバ|ユーザ|プロバイダ|メモリ)(?!ー)/;
    expect(said.filter(({ text }) => clipped.test(text))).toEqual([]);
  });
});

describe('picking a catalogue', () => {
  it.each(['zh-CN', 'zh', 'zh_CN', 'ZH-cn', 'zh-Hans', 'zh-Hans-CN', 'zh-SG', 'zh-TW'])(
    'gives the Chinese one for %s', (locale) => {
      expect(messagesFor(locale)).toBe(zhCN);
    },
  );

  it.each(['ja', 'ja-JP', 'ja_JP', 'JA-jp', 'ja-Jpan-JP'])(
    'gives the Japanese one for %s', (locale) => {
      expect(messagesFor(locale)).toBe(ja);
    },
  );

  it.each(['en', 'en-US', 'en_GB', 'ko', 'ko-KR', 'fr-FR', 'zhuang', 'jam', 'jv-ID', '', 'C'])(
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
    expect([0.5, 1, 42, 60, 125, 3600, 3725].map(ja.duration)).toEqual([
      '0.5 秒', '1 秒', '42 秒', '1 分', '2 分 5 秒', '1 時間', '1 時間 2 分',
    ]);
  });
});

describe.each(catalogues)('what is said in %s', (_name, messages) => {
  it.each(modelFailureCodes)('has words for the failure %s of a request', (code) => {
    for (const status of [undefined, null, 401]) {
      hasWords(sayModelFailure(messages, classifyModelFailure({ code, status })));
    }
  });

  // The three things that can be wrong with a key, each by the status that tells it apart.
  const ofAKey = [['noKey', null], ['keyRefused', 401], ['keyProblem', undefined]] as const;

  it.each(ofAKey)('says of %s which service is meant, where that is known', (kind, status) => {
    const failure = classifyModelFailure({ code: 'AUTH', message: 'what the service wrote', status });
    expect(failure.kind).toBe(kind);
    expect(sayModelFailure(messages, failure)).toBe(messages.model[kind]);
    expect(sayModelFailure(messages, failure, null)).toBe(messages.model[kind]);
    expect(sayModelFailure(messages, failure, '')).toBe(messages.model[kind]);
    const named = sayModelFailure(messages, failure, 'DeepSeek');
    expect(named).toEqual(messages.model.named[kind]('DeepSeek'));
    expect(named.what).toContain('DeepSeek');
    const unnamed: Saying = messages.model[kind];
    expect(named.what).not.toBe(unnamed.what);
    // What can be done about it is said as it is said without the name.
    expect(named.why).toBe(unnamed.why);
    // A turn that ended with that failure says the same.
    const ended = classifyTurnEnd('error', 'AUTH: what the service wrote', { code: 'AUTH', message: 'what the service wrote', status });
    expect(sayTurn(messages, ended, 'DeepSeek')).toEqual(messages.model.named[kind]('DeepSeek'));
    expect(sayTurn(messages, ended)).toBe(messages.model[kind]);
  });

  it('has nothing to say of a service where a turn did not end with the failure of a request', () => {
    expect(sayTurn(messages, classifyTurnEnd('aborted', 'user'), 'DeepSeek')).toEqual(sayTurn(messages, classifyTurnEnd('aborted', 'user')));
  });

  it('names the service only where something is wrong with a key', () => {
    for (const code of modelFailureCodes.filter(code => code !== 'AUTH')) {
      for (const status of [undefined, null, 401]) {
        const failure = classifyModelFailure({ code, status });
        expect(sayModelFailure(messages, failure, 'DeepSeek'), `${code} ${status}`).toBe(messages.model[failure.kind]);
      }
    }
    // Every kind that is said with a name is one that is said without, too.
    expect(Object.keys(messages.model.named).sort()).toEqual(['keyProblem', 'keyRefused', 'noKey']);
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

  it('says that a turn reached its limit of steps where the log does not say how many they were', () => {
    const counted = sayTurn(messages, classifyTurnEnd('blocked', 'the turn has taken its 30 steps', null, 'STEP_BUDGET'))!;
    const uncounted = sayTurn(messages, classifyTurnEnd('blocked', 'no step is left', null, 'STEP_BUDGET'))!;
    hasWords(uncounted);
    // It makes up no number, and it says the same of what the user can do next.
    expect(uncounted.what).not.toMatch(/\d|null|NaN/);
    expect(uncounted.what).not.toBe(counted.what);
    expect(uncounted.why).toBe(counted.why);
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

  it('are said in Japanese too', () => {
    const [first, second] = turnsOf('tour');
    expect(sayTurn(ja, turnOutcome(second)!)).toEqual({ what: 'あなたがこのターンを止めました。' });
    expect(ja.turn.took((first.end!.time - first.startedAt) / 1000, first.rounds)).toBe('14 秒、5 ステップで完了');
    expect(ja.wait.retry(4, 2)).toBe('4 秒後に再試行します（2 回目）。');
    expect(ja.wait.retry(0, 3)).toBe('再試行しています（3 回目）。');
    expect(ja.tool.allowedByYou).toBe('あなたが承認');
    expect(ja.tool.declined.what).toBe('あなたが却下');
    expect(ja.tool.exited(1).what).toBe('終了コード 1');
    expect(ja.turn.limit.steps(30).what).toBe('このターンは、上限の 30 ステップに達しました。');
    expect(ja.turn.limit.steps(null).what).toBe('このターンは、ステップ数の上限に達しました。');
    expect(sayPermissions(ja, 'workspace-write', 'ask'))
      .toBe('このプロジェクトのファイルを変更できます。コマンドを実行する前に確認します。');
  });

  it('name the service that a key is missing for, or that did not accept one', () => {
    expect(en.model.named.noKey('DeepSeek')).toEqual({ what: 'No API key is set up for DeepSeek.' });
    expect(zhCN.model.named.noKey('DeepSeek')).toEqual({ what: '还没有配置 DeepSeek 的 API 密钥。' });
    expect(ja.model.named.noKey('DeepSeek')).toEqual({ what: 'DeepSeek の API キーが設定されていません。' });
    expect(en.model.named.keyRefused('my-own').what).toBe('my-own did not accept the API key.');
    expect(zhCN.model.named.keyRefused('my-own').what).toBe('my-own 没有接受这个 API 密钥。');
    expect(ja.model.named.keyRefused('my-own').what).toBe('my-own が API キーを受け付けませんでした。');
    expect(en.model.named.keyProblem('DeepSeek').what).toBe('DeepSeek cannot be used with the API key as it is.');
    expect(zhCN.model.named.keyProblem('DeepSeek').what).toBe('现在的 API 密钥用不了 DeepSeek。');
    expect(ja.model.named.keyProblem('DeepSeek').what).toBe('現在の API キーでは、DeepSeek を利用できません。');
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
    expect(en.turn.limit.steps(null).what).toBe('This turn reached its limit of steps.');
  });
});
