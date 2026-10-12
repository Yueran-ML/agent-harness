import { describe, expect, it } from 'vitest';
import type { SessionEvent } from '../src/protocol';
import { emptyProjection, projectEvents, type ConversationItem } from '../src/state';
import { foldsAway, groupTurns, itemsOf, lastThought, placed, promptOf, saidOnTheWay, type Entry, type Turn } from '../src/turns';
import { eventsOf, logs } from './logs';

// A log is written here as a list of events. Each stands where it is listed,
// and a tenth of a second passes between two of them.
type Written = [type: string, data?: Record<string, unknown>];
const log = (...written: Written[]): SessionEvent[] =>
  written.map(([type, data = {}], seq) => ({ seq, type, data, time: 1000 + seq * 100 }));

const text = (words: string) => [{ type: 'text', text: words }];
const user = (words: string, source = 'user'): Written =>
  ['user/message', { source, message: { role: 'user', content: text(words) } }];
const reply = (words: string, finish: string | null = 'stop'): Written =>
  ['assistant/message', { message: { role: 'assistant', content: text(words) }, finish_reason: finish }];
const thought = (words: string): Written =>
  ['assistant/message', {
    message: { role: 'assistant', content: [{ type: 'reasoning', text: words }] },
    finish_reason: 'stop',
  }];
const call = (id: string, name = 'read'): Written => ['tool/call', { call_id: id, name, arguments: '{}' }];
const result = (id: string, words = 'ok'): Written =>
  ['tool/result', { message: { role: 'tool', call_id: id, content: text(words) }, error: null }];
const step = (turn: number, number: number): Written => ['step/start', { turn, step: number }];
const begin = (turn: number): Written => ['turn/start', { turn }];
const end = (turn: number, reason = 'completed', detail: string | null = null): Written =>
  ['turn/end', { turn, reason, detail }];
const summary = (words: string): Written =>
  ['user/message', { source: 'compaction', message: { role: 'user', content: text(words) } }];

// One word for an item, so that a whole conversation can be read in a line.
function sketch(item: ConversationItem): string {
  switch (item.type) {
    case 'user': return item.source;
    case 'assistant': return item.text ? 'text' : 'reasoning';
    case 'tool': return item.name;
    case 'notice': return 'notice';
    case 'compaction': return 'summary';
  }
}

// The entries of a conversation in a line: a turn by its number, anything else by its word.
const outline = (entries: Entry[]) =>
  entries.map(entry => entry.kind === 'turn' ? entry.number : sketch(entry.item));

const turns = (entries: Entry[]): Turn[] =>
  entries.filter((entry): entry is Turn => entry.kind === 'turn');

const turnsOf = (...written: Written[]): Turn[] => turns(groupTurns(projectEvents(log(...written))));

describe('the turn of an item', () => {
  it('is the turn that was under way when the item came, and none outside of turns', () => {
    const projection = projectEvents(log(
      result('early', 'a result that no call asked for'),
      begin(1), user('first'), reply('done'), end(1),
      summary('what was said so far'),
      begin(2), user('second'), call('a'), result('a'), end(2, 'aborted', 'user'),
      summary('what was said after that'),
    ));
    expect(projection.items.map(item => [sketch(item), item.turn])).toEqual([
      ['Tool result', null],
      ['user', 1], ['text', 1],
      ['summary', null],
      // The line that says how a turn ended belongs to that turn.
      ['user', 2], ['read', 2], ['notice', 2],
      ['summary', null],
    ]);
  });

  it('gives a turn that runs to the items that have come so far', () => {
    const projection = projectEvents(log(begin(1), user('first'), call('a')));
    expect(projection.items.map(item => item.turn)).toEqual([1, 1]);
    expect(projection.turn).toBe(1);
  });

  it.each(logs)('keeps the items of every turn of %s together', (name) => {
    const projection = projectEvents(eventsOf(name));
    const numbers = projection.items.map(item => item.turn);
    // Where the number changes from one item to the next, a turn begins or ends.
    const changes = numbers.filter((number, index) => index === 0 || number !== numbers[index - 1]);
    const begun = changes.filter(number => number !== null);
    // A turn that has gone by does not come back.
    expect(begun).toEqual(projection.turns.map(record => record.number));
  });
});

describe('the record of a turn', () => {
  it('says where the turn stands, when it began, how many steps it took and how it ended', () => {
    const projection = projectEvents(log(
      begin(1), user('first'),
      step(1, 1), call('a'), result('a'), ['step/end', { turn: 1, step: 1 }],
      step(1, 2), reply('done'), ['step/end', { turn: 1, step: 2 }],
      end(1),
      summary('what was said so far'),
      begin(2), user('second'),
      step(2, 1),
      end(2, 'error', 'SERVER: the service is down'),
      begin(3),
    ));
    expect(projection.turns).toEqual([
      {
        number: 1, at: 0, seq: 0, startedAt: 1000, steps: 2, provider: null,
        end: { seq: 9, time: 1900, reason: 'completed', detail: null },
      },
      {
        // Three items of the first turn stand before it, and the summary between the two.
        number: 2, at: 4, seq: 11, startedAt: 2100, steps: 1, provider: null,
        end: { seq: 14, time: 2400, reason: 'error', detail: 'SERVER: the service is down' },
      },
      { number: 3, at: 6, seq: 15, startedAt: 2500, steps: 0, provider: null, end: null },
    ]);
    expect(emptyProjection().turns).toEqual([]);
  });

  it('names the service that the requests of the turn went to', () => {
    const header = (provider?: string): Written =>
      ['request/header', { config: provider === undefined ? {} : { provider, model: 'a-model' }, tools: [] }];
    const projection = projectEvents(log(
      // A header holds for the requests after it.
      begin(1), user('first'), header('deepseek'), end(1, 'error', 'AUTH: no key'),
      // The log writes a header only where something changed. The one before holds on.
      begin(2), user('second'), end(2),
      // Another service is in use from the middle of this turn: the turn ends as one of that service.
      begin(3), user('third'), header('my-own'), end(3, 'error', 'AUTH: refused'),
      begin(4), user('fourth'), header(), end(4),
      begin(5),
    ));
    expect(projection.turns.map(record => record.provider)).toEqual(['deepseek', 'deepseek', 'my-own', null, null]);
    expect(turns(groupTurns(projection)).map(turn => turn.provider)).toEqual(['deepseek', 'deepseek', 'my-own', null, null]);
  });

  it('names no service for a turn that the log has no record of', () => {
    // An item that names a turn which never began: the turn is made up from the item.
    const [made] = turns(groupTurns({
      ...emptyProjection(),
      items: [{ id: 'event-4', seq: 4, time: 1400, turn: 9, type: 'assistant', text: 'hello', reasoning: '', interrupted: false }],
    }));
    expect(made).toMatchObject({ number: 9, provider: null });
  });

  it('is not closed by the end of another turn', () => {
    const projection = projectEvents(log(begin(2), user('second'), end(1, 'error', 'late')));
    expect(projection.turns[0].end).toBeNull();
    expect(projection.turn).toBe(2);
    // The line is there, as before. It stands in the turn that is under way.
    expect(projection.items.map(item => [sketch(item), item.turn])).toEqual([['user', 2], ['notice', 2]]);
  });

  it('does not count the step of another turn', () => {
    const projection = projectEvents(log(begin(2), step(1, 4), step(2, 1)));
    expect(projection.turns[0].steps).toBe(1);
  });

  it.each([
    ['deepseek-read', [2]],
    ['deepseek-fix', [4]],
    ['tour', [5, 2, 4, 1]],
  ] as const)('counts the steps of every turn of %s', (name, steps) => {
    const projection = projectEvents(eventsOf(name));
    expect(projection.turns.map(turn => turn.steps)).toEqual(steps);
    expect(projection.turns.map(turn => turn.number)).toEqual(steps.map((_, index) => index + 1));
    expect(projection.turns.every(turn => turn.end !== null)).toBe(true);
  });
});

describe('a conversation by turn', () => {
  it.each(logs)('puts every item of %s in one place, and keeps the order of the log', (name) => {
    const projection = projectEvents(eventsOf(name));
    const entries = groupTurns(projection);
    const placed = entries.flatMap(itemsOf);
    expect(placed).toHaveLength(projection.items.length);
    // The very items of the projection, each once, and in its order.
    placed.forEach((item, index) => expect(item).toBe(projection.items[index]));
    // One entry for every turn of the log, and none twice.
    expect(turns(entries).map(turn => turn.number)).toEqual(projection.turns.map(record => record.number));
    // What stands in a turn belongs to it, and what stands outside belongs to none.
    for (const entry of entries) {
      const owner = entry.kind === 'turn' ? entry.number : null;
      expect(itemsOf(entry).every(item => item.turn === owner)).toBe(true);
    }
  });

  it('reads the first real run as one turn: a question, a look into a file, an answer', () => {
    const entries = groupTurns(projectEvents(eventsOf('deepseek-read')));
    expect(entries).toHaveLength(1);
    const [turn] = turns(entries);
    expect(turn).toMatchObject({ number: 1, running: false, rounds: 2 });
    expect(turn.opening.map(item => item.text)).toEqual([
      'What is the codename of release 0.1? It is written in notes.txt. Answer with the codename only.',
    ]);
    expect(turn.steps.map(sketch)).toEqual(['text', 'read']);
    expect(turn.answer?.text).toBe('Halyard');
    expect(turn.end).toMatchObject({ reason: 'completed', detail: null, notice: null });
  });

  it('reads the second real run as one turn of four rounds', () => {
    const [turn] = turns(groupTurns(projectEvents(eventsOf('deepseek-fix'))));
    expect(turn.opening.map(sketch)).toEqual(['user']);
    // The first reply held thoughts and two calls, and no text. The third held a call alone.
    expect(turn.steps.map(sketch)).toEqual(['reasoning', 'read', 'read', 'text', 'edit', 'powershell']);
    expect(turn.answer?.text.startsWith('Fixed. In `calc.py`')).toBe(true);
    expect(turn.rounds).toBe(4);
    expect(turn.startedAt).toBe(1791534415704);
    expect(turn.end?.time).toBe(1791534420269);
  });

  it('reads the sample as four turns, with a summary after the third and after the fourth', () => {
    const entries = groupTurns(projectEvents(eventsOf('tour')));
    expect(outline(entries)).toEqual([1, 2, 3, 'summary', 4, 'summary']);
    const [first, second, third, fourth] = turns(entries);

    expect(first.opening.map(sketch)).toEqual(['user']);
    expect(first.steps.map(sketch)).toEqual([
      'text', 'read', 'read',
      // What the user said on the way, then a request that failed and the wait before the next.
      'steer', 'notice', 'notice',
      'text', 'todo_write', 'edit',
      'todo_write', 'bash',
      'todo_write',
    ]);
    expect(first.answer?.text.startsWith('mean() divided by one too few.')).toBe(true);
    expect(saidOnTheWay(first).map(item => item.text)).toEqual(['Keep the change as small as you can.']);

    // The user stopped the second turn while the model was writing.
    expect(second.steps.map(sketch)).toEqual(['text', 'bash', 'text']);
    expect(second.steps.at(-1)).toMatchObject({ type: 'assistant', interrupted: true });
    expect(second.answer).toBeNull();
    expect(second.end).toMatchObject({ reason: 'aborted', detail: 'user' });
    expect(second.end?.notice).toMatchObject({ type: 'notice', text: 'Turn stopped: user' });

    expect(third.steps.map(sketch)).toEqual(['text', 'subagent', 'bash', 'text', 'bash']);
    expect(third.answer?.text.startsWith('The mistake came in with commit 0029d5f')).toBe(true);

    expect(fourth.opening.map(item => item.text)).toEqual(['What is left to do?']);
    expect(fourth.steps).toEqual([]);
    expect(fourth.answer?.text.startsWith('Only the commit.')).toBe(true);
    expect(turns(entries).map(turn => turn.rounds)).toEqual([5, 2, 4, 1]);
  });

  it('gives a turn that runs no answer yet, although the model has written', () => {
    const [turn] = turnsOf(begin(1), user('go on'), reply('So far so good.'));
    expect(turn).toMatchObject({ running: true, answer: null, end: null });
    expect(turn.steps.map(sketch)).toEqual(['text']);
  });

  it('shows a turn that has begun and holds nothing yet, where it stands', () => {
    const entries = groupTurns(projectEvents(log(
      begin(1), user('first'), reply('done'), end(1),
      summary('what was said so far'),
      begin(2),
    )));
    expect(outline(entries)).toEqual([1, 'summary', 2]);
    expect(turns(entries)[1]).toMatchObject({
      number: 2, running: true, opening: [], steps: [], answer: null, end: null, startedAt: 1500,
    });
    expect(turns(entries)[0].running).toBe(false);
  });

  it('takes for the answer the last thing of a turn that completed, if the model wrote it', () => {
    const asked = (...written: Written[]) => turnsOf(begin(1), user('ask'), ...written)[0];

    expect(asked(reply('first'), call('a'), result('a'), reply('last'), end(1)).answer?.text).toBe('last');

    // Text that came with a call says what the model is about to do, not what came of it.
    const concluded = asked(
      reply('I will call it a day.', 'tool-calls'), call('a', 'finish'), result('a'), end(1),
    );
    expect(concluded.answer).toBeNull();
    expect(concluded.steps.map(sketch)).toEqual(['text', 'finish']);

    // Thoughts alone are no answer.
    expect(asked(thought('hm'), end(1))).toMatchObject({ answer: null, steps: [{ type: 'assistant' }] });
  });

  it.each([
    ['aborted', 'user'],
    ['blocked', 'the turn has taken its 3 steps'],
    ['error', 'SERVER: down'],
    ['max-tokens', null],
    ['interrupted', null],
    ['forked', null],
  ])('gives a turn that ended as %s no answer, and keeps what the model wrote as a step', (reason, detail) => {
    const [turn] = turnsOf(begin(1), user('ask'), reply('half a rep'), end(1, reason, detail));
    expect(turn.answer).toBeNull();
    expect(turn.steps.map(sketch)).toEqual(['text']);
    expect(turn.end).toMatchObject({ reason, detail, time: 1300 });
    // The line of the earlier interface is kept with the end, and is no step.
    expect(turn.end?.notice?.type).toBe('notice');
    expect(turn.running).toBe(false);
  });

  it('keeps the end of a turn that ended in a way it does not know', () => {
    const [turn] = turnsOf(begin(1), user('ask'), reply('words'), end(1, 'paused', 'later'));
    expect(turn.end).toEqual({ reason: 'paused', detail: 'later', time: 1300, notice: null });
    expect(turn.answer).toBeNull();
  });

  it('opens a turn with all that it was given before anything else happened in it', () => {
    const opened = (...written: Written[]) => turnsOf(begin(1), ...written, reply('done'), end(1))[0];

    // A skill that the user handed over comes before the task that is to be done by it.
    const skilled = opened(user('The user wants you to go by this skill.', 'skill'), user('tidy up'));
    expect(skilled.opening.map(sketch)).toEqual(['skill', 'user']);
    expect(promptOf(skilled)?.text).toBe('tidy up');

    // What a hook adds to an input follows it.
    const hooked = opened(user('deploy'), user('The branch is main.', 'hook/UserPromptSubmit'));
    expect(hooked.opening.map(sketch)).toEqual(['user', 'hook/UserPromptSubmit']);
    expect(promptOf(hooked)?.text).toBe('deploy');

    // A turn that no task opened: the user spoke while nothing ran.
    const steered = opened(user('one more thing', 'steer'));
    expect(steered.opening.map(sketch)).toEqual(['steer']);
    expect(promptOf(steered)?.text).toBe('one more thing');
    expect(saidOnTheWay(steered)).toEqual([]);

    // Where a task opened the turn, the task is what was asked, whatever else the user said.
    const both = opened(user('wait', 'steer'), user('the task'), user('and hurry', 'steer'));
    expect(both.opening.map(sketch)).toEqual(['steer', 'user', 'steer']);
    expect(promptOf(both)?.text).toBe('the task');

    // What a plugin gave in the place of the user is nothing that a person said.
    expect(promptOf(opened(user('for the model', 'inject')))).toBeNull();
  });

  it('tells what the user said on the way from what a hook or a plugin put in', () => {
    const [turn] = turnsOf(
      begin(1), user('ask'),
      reply('I look.', 'tool-calls'), call('a'), result('a'),
      user('look in src', 'steer'),
      reply('Found it.'),
      user('The tests have not run yet.', 'hook/Stop'),
      reply('They pass.'),
      end(1),
    );
    expect(turn.steps.map(sketch)).toEqual(['text', 'read', 'steer', 'text', 'hook/Stop']);
    expect(saidOnTheWay(turn).map(item => item.text)).toEqual(['look in src']);
    expect(turn.answer?.text).toBe('They pass.');
  });

  it('leaves what belongs to no turn where it stands', () => {
    const entries = groupTurns(projectEvents(log(
      result('early', 'a result that no call asked for'),
      begin(1), user('ask'), reply('done'), end(1),
      summary('what was said so far'),
    )));
    expect(entries.map(entry => entry.kind)).toEqual(['outside', 'turn', 'outside']);
    expect(itemsOf(entries[0]).map(sketch)).toEqual(['Tool result']);
    expect(itemsOf(entries[1]).map(sketch)).toEqual(['user', 'text']);
    expect(itemsOf(entries[2]).map(sketch)).toEqual(['summary']);
  });

  it('loses no item of a projection that names a turn without a record of it', () => {
    const made = projectEvents(log(begin(7), user('ask'), reply('done')));
    const entries = groupTurns({ ...made, turns: [] });
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ kind: 'turn', number: 7, running: false, end: null, rounds: 0 });
    expect(itemsOf(entries[0])).toEqual(made.items);
  });

  it('has nothing to show for an empty log', () => {
    expect(groupTurns(emptyProjection())).toEqual([]);
  });
});

describe('what was said beside the log', () => {
  // The answer of a command, by the line that asked for it and how far the log had come.
  const said = (line: string, after: number) => ({ line, after });
  // The conversation in a line: a turn by its number, an answer by its command, anything else by its word.
  const order = (written: Written[], beside: { line: string; after: number }[]) =>
    placed(groupTurns(projectEvents(log(...written))), beside)
      .map(entry => (entry.kind === 'beside' ? entry.said.line : entry.kind === 'turn' ? entry.number : sketch(entry.item)));

  // Two turns with a summary between them. The events are numbered from nought:
  // the first turn is 0 to 3, the summary is 4, the second turn is 5 to 8.
  const two: Written[] = [
    begin(1), user('first'), reply('done'), end(1),
    summary('what was said so far'),
    begin(2), user('second'), reply('done'), end(2),
  ];

  it('stands where it was said: after what the log held by then, and before what came later', () => {
    // Asked after the first turn. The turn that followed does not come to stand above the answer.
    expect(order(two, [said('/model', 3)])).toEqual([1, '/model', 'summary', 2]);
    expect(order(two, [said('/compact', 4)])).toEqual([1, 'summary', '/compact', 2]);
    expect(order(two, [said('/help', 8)])).toEqual([1, 'summary', 2, '/help']);
  });

  it('stands before everything where nothing was in the log yet', () => {
    expect(order(two, [said('/help', -1)])).toEqual(['/help', 1, 'summary', 2]);
    expect(order([], [said('/help', -1)])).toEqual(['/help']);
  });

  it('stands after a turn that had begun, also where the turn has gone on since', () => {
    // The log had come to the middle of the second turn. The turn is one thing and is not cut in two.
    expect(order(two, [said('/todos', 6)])).toEqual([1, 'summary', 2, '/todos']);
    // The event that opened the turn was the last one: the turn had begun.
    expect(order(two, [said('/todos', 5)])).toEqual([1, 'summary', 2, '/todos']);
  });

  it('keeps the order in which several things were said at one place', () => {
    expect(order(two, [said('/model', 3), said('/permissions', 3), said('/help', 3)]))
      .toEqual([1, '/model', '/permissions', '/help', 'summary', 2]);
  });

  it('is put in its place whatever the order in which it is handed over', () => {
    expect(order(two, [said('/help', 8), said('/model', 3), said('/first', -1)]))
      .toEqual(['/first', 1, '/model', 'summary', 2, '/help']);
  });

  it('loses nothing and adds nothing', () => {
    const entries = groupTurns(projectEvents(log(...two)));
    const beside = [said('/a', 100), said('/b', -1), said('/c', 4)];
    const all = placed(entries, beside);
    expect(all.filter(entry => entry.kind !== 'beside')).toEqual(entries);
    expect(all.filter(entry => entry.kind === 'beside').map(entry => entry.kind === 'beside' && entry.said).sort((a, b) => (a && b ? a.after - b.after : 0)))
      .toEqual([...beside].sort((a, b) => a.after - b.after));
    // What was handed over is left as it was.
    expect(beside.map(one => one.line)).toEqual(['/a', '/b', '/c']);
    expect(placed(entries, [])).toEqual(entries);
  });

  it('knows where a turn began in the log', () => {
    expect(turnsOf(...two).map(turn => turn.seq)).toEqual([0, 5]);
  });
});

describe('the way to the answer of a turn', () => {
  const answered = (thoughts: string, words: string): Written =>
    ['assistant/message', {
      message: { role: 'assistant', content: [...(thoughts ? [{ type: 'reasoning', text: thoughts }] : []), ...text(words)] },
      finish_reason: 'stop',
    }];

  it('holds what the model thought before it wrote the answer', () => {
    const [turn] = turnsOf(begin(1), user('ask'), answered('first this, then that', 'Done.'), end(1));
    expect(turn.answer?.text).toBe('Done.');
    expect(lastThought(turn)).toBe('first this, then that');
    // Nothing else happened on the way, and still there is something to put away and to open.
    expect(turn.steps).toEqual([]);
    expect(foldsAway(turn)).toBe(true);
  });

  it('is nothing where the model wrote its answer at once', () => {
    const [turn] = turnsOf(begin(1), user('ask'), answered('', 'Done.'), end(1));
    expect(lastThought(turn)).toBe('');
    expect(foldsAway(turn)).toBe(false);
  });

  it('can be put away where steps led to the answer, thought or not', () => {
    const [plain, thinking] = turnsOf(
      begin(1), user('ask'), call('a'), result('a'), answered('', 'Done.'), end(1),
      begin(2), user('ask'), call('b'), result('b'), answered('so that is it', 'Done.'), end(2),
    );
    expect(foldsAway(plain)).toBe(true);
    expect(lastThought(plain)).toBe('');
    expect(foldsAway(thinking)).toBe(true);
    expect(lastThought(thinking)).toBe('so that is it');
  });

  it('is shown in full where the turn has no answer', () => {
    // A turn that runs, and one that was stopped before the model answered.
    const [stopped, running] = turnsOf(
      begin(1), user('ask'), thought('hm'), call('a'), result('a'), end(1, 'aborted', 'user'),
      begin(2), user('ask'), call('b'),
    );
    expect(foldsAway(stopped)).toBe(false);
    expect(lastThought(stopped)).toBe('');
    expect(foldsAway(running)).toBe(false);
    expect(lastThought(running)).toBe('');
  });

  it('keeps what was thought at earlier steps where it was thought', () => {
    const [turn] = turnsOf(
      begin(1), user('ask'),
      ['assistant/message', { message: { role: 'assistant', content: [{ type: 'reasoning', text: 'look first' }, ...text('I will look.')] }, finish_reason: 'tool-calls' }],
      call('a'), result('a'), answered('now I know', 'Done.'), end(1),
    );
    expect(turn.steps.map(item => (item.type === 'assistant' ? item.reasoning : item.type))).toEqual(['look first', 'tool']);
    expect(lastThought(turn)).toBe('now I know');
  });
});
