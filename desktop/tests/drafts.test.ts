import { describe, expect, it } from 'vitest';
import { Drafts, samePlace, Unread, type Place } from '../src/app/drafts';

const place = (project: string | null, task: string): Place => ({ project, task });

describe('what is written and not sent', () => {
  it('is nothing for a task that nothing was kept for', () => {
    expect(new Drafts().of(place('F:\\work', 'a'))).toBe('');
  });

  it('is kept for its task and given back as it was', () => {
    const drafts = new Drafts();
    drafts.keep(place('F:\\work', 'a'), 'half a thought\n  with a second line ');
    expect(drafts.of(place('F:\\work', 'a'))).toBe('half a thought\n  with a second line ');
    drafts.keep(place('F:\\work', 'a'), 'another thought');
    expect(drafts.of(place('F:\\work', 'a'))).toBe('another thought');
  });

  it('is kept for one task only', () => {
    const drafts = new Drafts();
    drafts.keep(place('F:\\work', 'a'), 'for a');
    expect(drafts.of(place('F:\\work', 'b'))).toBe('');
  });

  it('is kept apart for tasks of the same name in two projects', () => {
    // The service knows a task by its project and its name. A name may be given by hand.
    const drafts = new Drafts();
    drafts.keep(place('F:\\one', 'review'), 'of the first project');
    drafts.keep(place('F:\\two', 'review'), 'of the second project');
    expect(drafts.of(place('F:\\one', 'review'))).toBe('of the first project');
    expect(drafts.of(place('F:\\two', 'review'))).toBe('of the second project');
    expect(drafts.of(place(null, 'review'))).toBe('');
  });

  it('does not take two places for one, whatever stands in their names', () => {
    const drafts = new Drafts();
    drafts.keep(place('a\nb', 'c'), 'first');
    drafts.keep(place('a', 'b\nc'), 'second');
    drafts.keep(place('["a","b"]', 'c'), 'third');
    expect(drafts.of(place('a\nb', 'c'))).toBe('first');
    expect(drafts.of(place('a', 'b\nc'))).toBe('second');
    expect(drafts.of(place('["a","b"]', 'c'))).toBe('third');
    // A project that is not known is not the project that is called "null".
    drafts.keep(place(null, 'c'), 'fourth');
    expect(drafts.of(place('null', 'c'))).toBe('');
  });

  it('is forgotten when nothing is left of it', () => {
    const drafts = new Drafts();
    drafts.keep(place('F:\\work', 'a'), 'something');
    drafts.keep(place('F:\\work', 'a'), '');
    expect(drafts.of(place('F:\\work', 'a'))).toBe('');
    expect(drafts.size).toBe(0);
  });

  describe('once it was sent', () => {
    it('is no draft any more', () => {
      const drafts = new Drafts();
      drafts.keep(place('F:\\work', 'a'), '/compact');
      drafts.sent(place('F:\\work', 'a'), '/compact');
      expect(drafts.of(place('F:\\work', 'a'))).toBe('');
    });

    it('stays where something else was written meanwhile', () => {
      const drafts = new Drafts();
      drafts.keep(place('F:\\work', 'a'), '/compact and more');
      drafts.sent(place('F:\\work', 'a'), '/compact');
      expect(drafts.of(place('F:\\work', 'a'))).toBe('/compact and more');
    });

    it('leaves the same words alone that wait in another task', () => {
      const drafts = new Drafts();
      drafts.keep(place('F:\\work', 'a'), '/compact');
      drafts.keep(place('F:\\work', 'b'), '/compact');
      drafts.sent(place('F:\\work', 'a'), '/compact');
      expect(drafts.of(place('F:\\work', 'b'))).toBe('/compact');
    });
  });
});

describe('a draft that holds what was sent and never read', () => {
  const task = place('work', 'a');

  it('says so for as long as it is kept that way', () => {
    const drafts = new Drafts();
    expect(drafts.returned(task)).toBe(false);
    drafts.keep(task, 'do the other thing first', true);
    expect(drafts.of(task)).toBe('do the other thing first');
    expect(drafts.returned(task)).toBe(true);
    // Kept again as words of the user's own, it is a draft like any other.
    drafts.keep(task, 'do the other thing first, please');
    expect(drafts.returned(task)).toBe(false);
  });

  it('is told apart from the draft of another task', () => {
    const drafts = new Drafts();
    drafts.keep(task, 'sent and not read', true);
    drafts.keep(place('work', 'b'), 'written and not sent');
    expect(drafts.returned(place('work', 'b'))).toBe(false);
    expect(drafts.returned(place('other', 'a'))).toBe(false);
    expect(drafts.returned(task)).toBe(true);
  });

  it('is nothing once nothing is left of it, or once it was sent again', () => {
    const drafts = new Drafts();
    drafts.keep(task, 'sent and not read', true);
    drafts.keep(task, '', true);
    expect(drafts.returned(task)).toBe(false);
    expect(drafts.size).toBe(0);
    drafts.keep(task, 'sent and not read', true);
    drafts.sent(task, 'sent and not read');
    expect(drafts.returned(task)).toBe(false);
    expect(drafts.of(task)).toBe('');
  });
});

describe('what waits unread in a task that is left', () => {
  const first = place('work', 'a');
  const second = place('work', 'b');

  it('is what was seen waiting last while its turn was under way', () => {
    const unread = new Unread();
    expect(unread.take(first, true, [])).toEqual([]);
    unread.see(first, true, ['do the other thing first']);
    unread.see(first, true, ['do the other thing first', 'and then the tests']);
    // The page is drawn with the next task before it is known that the task has changed.
    unread.see(second, false, []);
    expect(unread.take(second, false, [])).toEqual(['do the other thing first', 'and then the tests']);
    // It is taken once.
    expect(unread.take(first, false, [])).toEqual([]);
  });

  it('is less once the agent has read some of it', () => {
    const unread = new Unread();
    unread.take(first, true, ['one', 'two']);
    unread.see(first, true, ['two']);
    expect(unread.take(second, false, [])).toEqual(['two']);
  });

  it('is nothing once the turn is over: what waits then is not dropped when the task is left', () => {
    const unread = new Unread();
    unread.take(first, true, ['one']);
    unread.see(first, false, ['one']);
    expect(unread.take(second, false, [])).toEqual([]);
    // Nor where the task was at rest from the beginning.
    unread.see(second, false, ['waits in the log']);
    expect(unread.take(first, false, [])).toEqual([]);
    // Nor where it is at rest in the moment it is shown, and left before the page is drawn again.
    unread.take(second, false, ['waits in the log']);
    expect(unread.take(first, false, [])).toEqual([]);
  });

  it('is what the task that is shown had from the moment it is shown', () => {
    const unread = new Unread();
    unread.take(first, false, []);
    // The next task is opened in the middle of a turn, with something waiting already.
    expect(unread.take(second, true, ['said earlier'])).toEqual([]);
    expect(unread.take(null, false, [])).toEqual(['said earlier']);
  });

  it('is kept apart for tasks of the same name in two projects, and for no task at all', () => {
    const unread = new Unread();
    unread.take(first, true, ['of the first project']);
    // A task of that name in another project is another task.
    unread.see(place('other', 'a'), true, ['of the other project']);
    expect(unread.take(place('other', 'a'), true, ['of the other project'])).toEqual(['of the first project']);
    expect(unread.take(null, false, [])).toEqual(['of the other project']);
    // No task is shown: nothing waits, whatever is said of it.
    unread.see(null, true, ['of no task']);
    expect(unread.take(null, true, ['of no task'])).toEqual([]);
    expect(unread.take(first, false, [])).toEqual([]);
  });
});

describe('two places', () => {
  it('are the same where project and task are', () => {
    expect(samePlace(place('F:\\work', 'a'), place('F:\\work', 'a'))).toBe(true);
    expect(samePlace(place(null, 'a'), place(null, 'a'))).toBe(true);
    expect(samePlace(null, null)).toBe(true);
  });

  it('differ where either differs, or where one is none', () => {
    expect(samePlace(place('F:\\work', 'a'), place('F:\\work', 'b'))).toBe(false);
    expect(samePlace(place('F:\\one', 'a'), place('F:\\two', 'a'))).toBe(false);
    expect(samePlace(place(null, 'a'), place('F:\\work', 'a'))).toBe(false);
    expect(samePlace(place('F:\\work', 'a'), null)).toBe(false);
    expect(samePlace(null, place('F:\\work', 'a'))).toBe(false);
  });
});
