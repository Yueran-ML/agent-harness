import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  agoOf, argumentsOf, contextPercent, countOf, folderName, goneWords, HISTORY_PAGE, joined, objectOf, presetOf, presets, retryText, scopeOf,
  stepped, STILL_STARTING_MS, tail, taskTitle, toolKind, verbOf, waitingWords,
} from '../src/app/tell';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import { projectEvents, type ToolItem } from '../src/state';
import { groupTurns, type Turn } from '../src/turns';
import { permissionPresetFromCommand } from '../src/uiActions';
import { sayPermissions } from '../src/words';
import { approvalPolicies, sandboxModes } from './appendix';

const minute = 60_000;
const hour = 60 * minute;
const day = 24 * hour;

// What the service says in the places that the interface reads word for word.
const permissions = readFileSync(new URL('../../src/columbina/permissions/__init__.py', import.meta.url), 'utf8');

describe('the name of a folder', () => {
  it('is the last part of a path, however the path is written', () => {
    expect(folderName('F:\\work\\project')).toBe('project');
    expect(folderName('/home/someone/project')).toBe('project');
    expect(folderName('F:\\work\\project\\')).toBe('project');
    expect(folderName('/home/someone/project/')).toBe('project');
    expect(folderName('F:\\mixed/separators\\here')).toBe('here');
  });

  it('is the path itself where the path has no parts', () => {
    expect(folderName('/')).toBe('/');
    expect(folderName('')).toBe('');
  });

  it('keeps blanks and characters of other scripts', () => {
    expect(folderName('F:\\my work\\我的 项目')).toBe('我的 项目');
  });
});

describe('how long ago', () => {
  const now = 1_800_000_000_000;

  it.each([
    [0, 'just now'],
    [59_999, 'just now'],
    [minute, '1 min ago'],
    [59 * minute + 59_999, '59 min ago'],
    [hour, '1 h ago'],
    [23 * hour + 59 * minute, '23 h ago'],
    [day, '1 d ago'],
    [45 * day, '45 d ago'],
  ])('%d ms is "%s"', (elapsed, said) => {
    expect(agoOf(en, now - elapsed, now)).toBe(said);
  });

  it('takes a moment that lies ahead for now', () => {
    // The log is written by another program, whose clock may be a little ahead.
    expect(agoOf(en, now + 5 * minute, now)).toBe('just now');
  });

  it('is said in the language of the catalogue', () => {
    expect(agoOf(zhCN, now - 3 * hour, now)).toBe('3 小时前');
  });
});

describe('what a task is called', () => {
  it('is the first line of what it began with', () => {
    expect(taskTitle(en, 'Fix the build.\nIt fails on Windows.')).toBe('Fix the build.');
    expect(taskTitle(en, 'One line')).toBe('One line');
  });

  it('skips blank lines and blanks at the ends', () => {
    expect(taskTitle(en, '\n\n  Fix the build.  \nmore')).toBe('Fix the build.');
    expect(taskTitle(en, 'Fix the build.   \r\nmore')).toBe('Fix the build.');
  });

  it('is the name of a new task where nothing was said yet', () => {
    expect(taskTitle(en, null)).toBe('New task');
    expect(taskTitle(en, '')).toBe('New task');
    expect(taskTitle(en, ' \n\t ')).toBe('New task');
    expect(taskTitle(zhCN, null)).toBe('新任务');
  });
});

describe('how full the context is', () => {
  it('is a whole number of percent', () => {
    expect(contextPercent(0, 1000)).toBe(0);
    expect(contextPercent(254, 1000)).toBe(25);
    expect(contextPercent(255, 1000)).toBe(26);
    expect(contextPercent(1000, 1000)).toBe(100);
  });

  it('is never more than all of it', () => {
    // A request may count more than the window that the service names.
    expect(contextPercent(1500, 1000)).toBe(100);
  });

  it('is not known where the size of the window is not', () => {
    expect(contextPercent(500, null)).toBeNull();
    expect(contextPercent(500, undefined)).toBeNull();
    expect(contextPercent(500, 0)).toBeNull();
    expect(contextPercent(500, -1)).toBeNull();
  });
});

describe('what is said while the page waits for the service', () => {
  const nothing = { answered: false, folder: null, reading: null };

  it('is that the service is being started in the folder, until it answers', () => {
    expect(waitingWords(en, { ...nothing, folder: 'F:/work' })).toBe('Starting Columbina in F:/work.');
    expect(waitingWords(zhCN, { ...nothing, folder: 'F:/work' })).toBe(zhCN.hint.starting('F:/work'));
  });

  it('is that the task is being opened, once the service has answered or where no folder is named', () => {
    expect(waitingWords(en, { ...nothing, answered: true, folder: 'F:/work' })).toBe('Opening the task.');
    expect(waitingWords(en, nothing)).toBe('Opening the task.');
  });

  it('is how far a long log is read, whatever else holds', () => {
    const reading = { read: 1500, of: 4200 };
    expect(waitingWords(en, { answered: true, folder: 'F:/work', reading })).toBe('Read 1500 of 4200 events.');
    expect(waitingWords(en, { ...nothing, reading: { read: 0, of: 501 } })).toBe('Read 0 of 501 events.');
    expect(waitingWords(zhCN, { ...nothing, reading })).toBe('已读取 1500 / 4200 条。');
    expect(waitingWords(ja, { ...nothing, reading })).toBe('4200 件のうち 1500 件を読み込みました。');
  });

  it('says more of a start after three seconds, as the design has it', () => {
    expect(STILL_STARTING_MS).toBe(3000);
    expect(en.hint.stillStarting).toBe('It is still starting.');
  });

  it('takes a log of more than one page for a long one', () => {
    expect(HISTORY_PAGE).toBe(500);
  });
});

describe('a service that is gone', () => {
  it('has ended if it ever answered, and could not be started if it never did', () => {
    expect(goneWords(en, true)).toBe('The service process has ended.');
    expect(goneWords(en, false)).toBe('The service process could not be started.');
    for (const words of [en, zhCN, ja]) {
      expect(goneWords(words, true)).toBe(words.hint.serviceEnded);
      expect(goneWords(words, false)).toBe(words.hint.serviceNotStarted);
      expect(goneWords(words, true)).not.toBe(goneWords(words, false));
    }
  });
});

describe('a count of tokens', () => {
  it('is as short as the language writes it', () => {
    expect([950, 2670, 12_400, 128_000, 1_048_576].map(count => countOf(count, 'en')))
      .toEqual(['950', '2.7K', '12.4K', '128K', '1M']);
    // Chinese counts in ten thousands, and names them with a character of its own.
    expect([950, 2670, 12_400, 128_000, 1_048_576].map(count => countOf(count, 'zh-CN')))
      .toEqual(['950', '2670', '1.2万', '12.8万', '104.9万']);
  });

  it('stands in a sentence of Chinese with a blank after a digit, and with none after a character', () => {
    const said = (used: number, window: number) => zhCN.hint.contextOf(countOf(used, 'zh-CN'), countOf(window, 'zh-CN'));
    expect(said(2670, 1_048_576)).toBe('最近一次请求用了 2670 个 token，上限是 104.9万。');
    expect(said(12_400, 1_048_576)).toBe('最近一次请求用了 1.2万个 token，上限是 104.9万。');
    expect(zhCN.hint.contextUnknown(countOf(950, 'zh-CN'))).toBe('最近一次请求用了 950 个 token。');
    expect(zhCN.hint.contextUnknown(countOf(12_400, 'zh-CN'))).toBe('最近一次请求用了 1.2万个 token。');
  });

  it('stands in a sentence of Japanese in the same way', () => {
    expect([950, 2670, 12_400, 128_000, 1_048_576].map(count => countOf(count, 'ja')))
      .toEqual(['950', '2670', '1.2万', '12.8万', '104.9万']);
    const said = (used: number, window: number) => ja.hint.contextOf(countOf(used, 'ja'), countOf(window, 'ja'));
    expect(said(2670, 8192)).toBe('直近のリクエストで、8192 トークンのうち 2670 を使いました。');
    expect(said(2670, 1_048_576)).toBe('直近のリクエストで、104.9万トークンのうち 2670 を使いました。');
    expect(said(12_400, 128_000)).toBe('直近のリクエストで、12.8万トークンのうち 1.2万を使いました。');
    expect(ja.hint.contextUnknown(countOf(950, 'ja'))).toBe('直近のリクエストで 950 トークンを使いました。');
    expect(ja.hint.contextUnknown(countOf(12_400, 'ja'))).toBe('直近のリクエストで 1.2万トークンを使いました。');
  });

  it('stands in a sentence of English as it is', () => {
    expect(en.hint.contextOf(countOf(2670, 'en'), countOf(1_048_576, 'en'))).toBe('2.7K of 1M tokens in the last request.');
    expect(en.hint.contextUnknown(countOf(950, 'en'))).toBe('950 tokens in the last request.');
  });
});

describe('what a longer yes covers', () => {
  const asked = (tool: string, target: string | null, standing: string | null) => ({ tool, target, standing });

  it('is nothing where the service offers none', () => {
    expect(scopeOf(en, asked('powershell', 'git push', null))).toBeNull();
  });

  it('is said in the words of the interface for each of the three ways of the service', () => {
    expect(scopeOf(en, asked('powershell', 'git status', 'this command, word for word'))).toBe(en.hint.scope.thisCommand);
    expect(scopeOf(en, asked('fetch', null, 'any call of fetch'))).toBe(en.hint.scope.anyCall('fetch'));
    expect(scopeOf(en, asked('write', 'notes.txt', 'write on notes.txt'))).toBe(en.hint.scope.onTarget('write', 'notes.txt'));
  });

  it('names the tool and what it acts on', () => {
    expect(en.hint.scope.anyCall('fetch')).toContain('fetch');
    expect(en.hint.scope.onTarget('write', 'notes.txt')).toContain('write on notes.txt');
    expect(zhCN.hint.scope.onTarget('write', 'notes.txt')).toContain('notes.txt');
  });

  it('is said in general words for a way that is not known, and never as it came', () => {
    for (const standing of ['every command of this kind', 'any call of another', 'write on other.txt', '']) {
      const said = scopeOf(en, asked('write', 'notes.txt', standing));
      expect(said).toBe(en.hint.scope.other);
      if (standing) expect(said).not.toContain(standing);
    }
    // "tool on target" without a target is no way that the service has.
    expect(scopeOf(en, asked('write', null, 'write on null'))).toBe(en.hint.scope.other);
  });

  it('says how far such a yes reaches: this task, and until the project is closed', () => {
    // The service keeps it for one task, in memory, and its process ends with the project
    // (`Server._granted` in the sources of the service). Words that promised more would be false.
    const said = (words: typeof en) => [
      words.hint.scope.thisCommand, words.hint.scope.anyCall('fetch'),
      words.hint.scope.onTarget('write', 'notes.txt'), words.hint.scope.other,
    ];
    for (const sentence of said(en)) {
      expect(sentence).toContain('in this task');
      expect(sentence).toContain('until the project is closed');
    }
    for (const sentence of said(zhCN)) {
      expect(sentence).toContain('这个任务');
      expect(sentence).toContain('到项目关闭为止');
    }
  });

  it('knows the ways of the service as its sources have them', () => {
    // Should one of these be worded anew there, the interface falls back to general
    // words, and this test says where to look.
    expect(permissions).toContain('return "this command, word for word"');
    expect(permissions).toContain('return f"any call of {tool}" if target is None else f"{tool} on {target}"');
  });
});

describe('the presets of the permissions', () => {
  it('are those that the command of the service takes', () => {
    for (const preset of presets) {
      expect(permissionPresetFromCommand(`/permissions ${preset.id}`)).toBe(preset.id);
    }
    expect(presets.map(preset => preset.id)).toEqual(['read-only', 'workspace-write', 'full-access']);
  });

  it('set what the sources of the service say they set', () => {
    for (const preset of presets) {
      expect(permissions).toContain(`"${preset.id}": ("${preset.mode}", "${preset.policy}"),`);
    }
  });

  it('have a sentence each, and no two the same', () => {
    const said = presets.map(preset => sayPermissions(en, preset.mode, preset.policy));
    expect(new Set(said).size).toBe(presets.length);
    expect(said).not.toContain(en.permissions.unknown);
  });

  it('are found again from what holds', () => {
    for (const preset of presets) expect(presetOf(preset.mode, preset.policy)).toBe(preset.id);
  });

  it('are none where what holds is none of them', () => {
    expect(presetOf('read-only', 'never')).toBeNull();
    expect(presetOf('workspace-write', 'never')).toBeNull();
    expect(presetOf(null, null)).toBeNull();
    expect(presetOf('read-only', null)).toBeNull();
    expect(presetOf('something-new', 'ask')).toBeNull();
  });

  it('take full access for full access whatever the policy is', () => {
    for (const policy of [...approvalPolicies, null]) {
      expect(presetOf('danger-full-access', policy)).toBe('full-access');
      // The sentence of the interface says the same of both.
      expect(sayPermissions(en, 'danger-full-access', policy)).toBe(en.permissions.fullAccess);
    }
  });

  it('cover every mode that the service has', () => {
    expect(presets.map(preset => preset.mode).sort()).toEqual([...sandboxModes].sort());
  });
});

describe('a call of a tool', () => {
  const call = (name: string, args: string): ToolItem => ({
    id: 'event-1', seq: 1, time: 0, turn: 1, type: 'tool', callId: 'a', name, arguments: args,
    result: null, error: null, durationMs: null, trimmed: false, interrupted: false, approval: null,
  });

  it('has its arguments read as an object, and as nothing where they are none', () => {
    expect(argumentsOf(call('read', '{"path":"a.txt"}'))).toEqual({ path: 'a.txt' });
    expect(argumentsOf(call('read', '{"path":"a.t'))).toBeNull();
    expect(argumentsOf(call('read', '["a.txt"]'))).toBeNull();
    expect(argumentsOf(call('read', 'null'))).toBeNull();
    expect(argumentsOf(call('read', '"a.txt"'))).toBeNull();
    expect(argumentsOf(call('read', ''))).toBeNull();
  });

  it.each([
    ['read', 'read'], ['write', 'write'], ['edit', 'edit'], ['glob', 'list'], ['grep', 'search'],
    ['bash', 'run'], ['powershell', 'run'], ['todo_write', 'todo'], ['skill', 'skill'],
    ['subagent', 'subagent'], ['mcp__files__fetch', 'other'], ['Read', 'other'], ['', 'other'],
  ] as const)('of %s is of the kind %s', (name, kind) => {
    expect(toolKind(name)).toBe(kind);
  });

  it('has a word for what it does, and for a tool that is not known the name of the tool', () => {
    expect(verbOf(en, 'grep')).toBe('Search');
    expect(verbOf(en, 'powershell')).toBe('Run');
    expect(verbOf(zhCN, 'edit')).toBe('修改');
    expect(verbOf(en, 'mcp__files__fetch')).toBe('Call mcp__files__fetch');
  });

  it('acts on the first of the arguments that say what on', () => {
    expect(objectOf({ command: 'git status', path: 'x' })).toBe('git status');
    expect(objectOf({ path: 'src/a.py', pattern: '*.py' })).toBe('src/a.py');
    expect(objectOf({ pattern: 'TODO', description: 'x' })).toBe('TODO');
    expect(objectOf({ description: 'look around', name: 'x' })).toBe('look around');
    expect(objectOf({ name: 'review' })).toBe('review');
  });

  it('acts on nothing that can be named where no such argument is text', () => {
    expect(objectOf(null)).toBe('');
    expect(objectOf({})).toBe('');
    expect(objectOf({ path: 3, command: ['git'] })).toBe('');
    expect(objectOf({ url: 'https://example.com' })).toBe('');
  });

  it('skips an argument that is blank', () => {
    expect(objectOf({ command: '   ', path: 'a.txt' })).toBe('a.txt');
    expect(objectOf({ command: '' })).toBe('');
  });
});

describe('the last lines of a text', () => {
  it('are all of it where it is short', () => {
    expect(tail('one\ntwo', 2)).toEqual({ shown: 'one\ntwo', cut: 0 });
    expect(tail('', 2)).toEqual({ shown: '', cut: 0 });
  });

  it('leave out what comes before them, and say how much', () => {
    expect(tail('one\ntwo\nthree\nfour', 2)).toEqual({ shown: 'three\nfour', cut: 2 });
  });

  it('do not count the end of the last line as a line', () => {
    expect(tail('one\ntwo\n', 2)).toEqual({ shown: 'one\ntwo', cut: 0 });
    expect(tail('one\ntwo\nthree\n', 2)).toEqual({ shown: 'two\nthree', cut: 1 });
    // An empty line before the end is a line.
    expect(tail('one\n\n', 2)).toEqual({ shown: 'one\n', cut: 0 });
  });
});

describe('several texts as one', () => {
  it('stand on lines of their own, in their order', () => {
    expect(joined(['first', 'second', 'third'])).toBe('first\nsecond\nthird');
    expect(joined(['only'])).toBe('only');
  });

  it('leave out what has nothing in it', () => {
    expect(joined(['', 'first', '  ', '\n', 'second', ''])).toBe('first\nsecond');
    expect(joined(['', ' '])).toBe('');
    expect(joined([])).toBe('');
  });

  it('keep each text as it was written', () => {
    expect(joined(['  indented', 'two\nlines '])).toBe('  indented\ntwo\nlines ');
  });
});

describe('what the agent is given when a turn is tried again', () => {
  const at = (type: string, data: Record<string, unknown>) => ({ type, data });
  const said = (text: string, source = 'user') => at('user/message', {
    message: { role: 'user', content: [{ type: 'text', text }] }, source,
  });
  const failed = () => at('assistant/attempt', { failure: { code: 'TRANSPORT', message: 'no route' } });
  const turnOf = (...events: ReturnType<typeof at>[]): Turn => {
    const all = [
      at('turn/start', { turn: 1 }), ...events,
      at('turn/end', { turn: 1, reason: 'error', detail: 'TRANSPORT: no route' }),
    ];
    const entries = groupTurns(projectEvents(all.map((event, index) => ({ ...event, seq: index, time: index * 100 }))));
    return entries.find((entry): entry is Turn => entry.kind === 'turn')!;
  };
  const goOn = en.prompt.goOn;

  it('is the task of the turn where nothing of it was done yet', () => {
    expect(retryText(turnOf(said('Fix the build.'), failed()), goOn)).toBe('Fix the build.');
  });

  it('is the task as it was written, also where it looks like a command', () => {
    // Whoever sends it has to send it as words for the agent. See `useWorkspace.prompt`.
    expect(retryText(turnOf(said('/permissions full-access'), failed()), goOn)).toBe('/permissions full-access');
  });

  it('is a word to go on where the turn had begun its work', () => {
    const call = at('tool/call', { call_id: 'a', name: 'read', arguments: '{"path":"a.txt"}' });
    const result = at('tool/result', {
      message: { role: 'tool', call_id: 'a', content: [{ type: 'text', text: 'ok' }] }, error: null,
    });
    expect(retryText(turnOf(said('Fix the build.'), call, result, failed()), goOn)).toBe(goOn);
    const wrote = at('assistant/message', {
      message: { role: 'assistant', content: [{ type: 'text', text: 'Looking.' }] }, finish_reason: 'stop',
    });
    expect(retryText(turnOf(said('Fix the build.'), wrote, failed()), goOn)).toBe(goOn);
  });

  it('is a word to go on where no person opened the turn', () => {
    expect(retryText(turnOf(said('A note of a hook.', 'hook/SessionStart'), failed()), goOn)).toBe(goOn);
    expect(retryText(turnOf(failed()), goOn)).toBe(goOn);
  });
});

describe('an arrow key among things in a row', () => {
  const row = ['changes', 'details', 'more'] as const;

  it('leads to the next one and to the one before', () => {
    expect(stepped(row, 'changes', 'ArrowRight')).toBe('details');
    expect(stepped(row, 'details', 'ArrowLeft')).toBe('changes');
    expect(stepped(row, 'changes', 'ArrowDown')).toBe('details');
    expect(stepped(row, 'details', 'ArrowUp')).toBe('changes');
  });

  it('goes around the ends', () => {
    expect(stepped(row, 'more', 'ArrowRight')).toBe('changes');
    expect(stepped(row, 'changes', 'ArrowLeft')).toBe('more');
  });

  it('leads to the first and the last with Home and End', () => {
    expect(stepped(row, 'details', 'Home')).toBe('changes');
    expect(stepped(row, 'details', 'End')).toBe('more');
  });

  it('leads nowhere for another key, or from something that is not in the row', () => {
    for (const key of ['Enter', ' ', 'Tab', 'a', 'PageDown']) expect(stepped(row, 'details', key)).toBeNull();
    expect(stepped(row, 'elsewhere' as typeof row[number], 'ArrowRight')).toBeNull();
    expect(stepped([] as string[], 'x', 'Home')).toBeNull();
  });

  it('stays where it is in a row of one', () => {
    expect(stepped(['only'], 'only', 'ArrowRight')).toBe('only');
    expect(stepped(['only'], 'only', 'ArrowLeft')).toBe('only');
  });
});
