import type {
  AssistantItem, ConversationItem, NoticeItem, Projection, TurnRecord, UserItem,
} from './state';

/** How a turn ended, as the log says it. `outcomes.ts` puts it into words. */
export interface TurnEnd {
  reason: string;
  detail: string | null;
  /** When the turn ended, in milliseconds since 1970. */
  time: number;
  /** The line that the earlier interface shows for this end, if it shows one. */
  notice: NoticeItem | null;
}

/**
 * One turn of a conversation: what it began with, what happened on the way,
 * and what came of it.
 *
 * Its items are those of the projection, in the order of the log: first the
 * opening, then the steps, then the answer, then the line of its end.
 */
export interface Turn {
  kind: 'turn';
  number: number;
  /** When the turn began, in milliseconds since 1970. */
  startedAt: number;
  /**
   * How often the model was asked, with what it asked for carried out each
   * time. The protocol calls that a step. Here, `steps` are items.
   */
  rounds: number;
  /** Whether the turn is under way. It has no answer and no end then. */
  running: boolean;
  /** The service that the requests of the turn went to, by the name that the log gives it, or null. */
  provider: string | null;
  /**
   * What the turn was given before anything else happened in it. As a rule
   * that is the task of the user. A skill that the user handed over stands
   * before the task, and what a hook adds to the task stands after it.
   */
  opening: UserItem[];
  /**
   * What lies between the opening and the answer: what the model wrote
   * beside its calls, the calls, notices, and what was said on the way.
   */
  steps: ConversationItem[];
  /** What the model wrote last, if the turn came to its end as turns are meant to. */
  answer: AssistantItem | null;
  /** Null while the turn runs, and for a turn whose end the log does not hold. */
  end: TurnEnd | null;
}

/** An item that belongs to no turn, such as a summary that was written between two turns. */
export interface Outside {
  kind: 'outside';
  item: ConversationItem;
}

export type Entry = Turn | Outside;

/** A turn while its items are being collected. */
interface Gathering {
  turn: Turn;
  items: ConversationItem[];
  end: TurnRecord['end'];
}

/**
 * Group the items of a projection by turn.
 *
 * Every item lands in exactly one entry, and the entries follow the log: what
 * `itemsOf` returns for them, one entry after another, is `projection.items`.
 * A turn stands where it began, also while it holds nothing yet.
 */
export function groupTurns(projection: Projection): Entry[] {
  const entries: Entry[] = [];
  const gathered: Gathering[] = [];
  const byNumber = new Map<number, Gathering>();
  const lastRecord = projection.turns.at(-1);

  const begin = (record: TurnRecord): Gathering => {
    const turn: Turn = {
      kind: 'turn',
      number: record.number,
      startedAt: record.startedAt,
      rounds: record.steps,
      // Only the last turn of a log can be under way.
      running: record.end === null && record === lastRecord && projection.turn === record.number,
      provider: record.provider,
      opening: [],
      steps: [],
      answer: null,
      end: null,
    };
    const gathering: Gathering = { turn, items: [], end: record.end };
    entries.push(turn);
    gathered.push(gathering);
    byNumber.set(record.number, gathering);
    return gathering;
  };

  let next = 0;
  projection.items.forEach((item, index) => {
    // A turn that began before this item stands before it.
    while (next < projection.turns.length && projection.turns[next].at <= index) {
      begin(projection.turns[next]);
      next += 1;
    }
    if (item.turn === null) {
      entries.push({ kind: 'outside', item });
      return;
    }
    // `projectEvents` keeps a record of every turn that an item names. Should a
    // projection name one without a record, the turn begins with that item, so
    // that nothing is lost.
    const gathering = byNumber.get(item.turn) ?? begin({
      number: item.turn, at: index, seq: item.seq, startedAt: item.time, steps: 0, provider: null, end: null,
    });
    gathering.items.push(item);
  });
  // The turns that began after the last item hold nothing yet.
  for (; next < projection.turns.length; next += 1) begin(projection.turns[next]);

  for (const gathering of gathered) arrange(gathering);
  return entries;
}

/** Give the items of a turn their places: the opening, the steps, the answer and the end. */
function arrange({ turn, items, end }: Gathering): void {
  let rest = items;

  if (end !== null) {
    // The line that says how the turn ended came with the event that ended it.
    const last = rest.at(-1);
    const notice = last?.type === 'notice' && last.seq === end.seq ? last : null;
    if (notice) rest = rest.slice(0, -1);
    turn.end = { reason: end.reason, detail: end.detail, time: end.time, notice };
  }

  // The opening ends where something other than input begins.
  for (const item of rest) {
    if (item.type !== 'user') break;
    turn.opening.push(item);
  }
  rest = rest.slice(turn.opening.length);

  // Only a turn that completed has an answer, and the answer is the last thing in
  // it. Text that is followed by a call says what the model is about to do.
  const last = rest.at(-1);
  if (end?.reason === 'completed' && last?.type === 'assistant' && last.text) {
    turn.answer = last;
    rest = rest.slice(0, -1);
  }
  turn.steps = rest;
}

/** The items of an entry, in the order of the log. */
export function itemsOf(entry: Entry): ConversationItem[] {
  if (entry.kind === 'outside') return [entry.item];
  return [
    ...entry.opening,
    ...entry.steps,
    ...(entry.answer ? [entry.answer] : []),
    ...(entry.end?.notice ? [entry.end.notice] : []),
  ];
}

/**
 * What a person said that opened the turn: the task, or else what was said
 * while nothing ran. Null for a turn that something else opened.
 */
export function promptOf(turn: Turn): UserItem | null {
  return turn.opening.find(item => item.source === 'user')
    ?? turn.opening.find(item => item.source === 'steer')
    ?? null;
}

/** What a person said while the turn ran. What a hook or a plugin put in is not among it. */
export function saidOnTheWay(turn: Turn): UserItem[] {
  return turn.steps.filter((item): item is UserItem => item.type === 'user' && item.source === 'steer');
}
