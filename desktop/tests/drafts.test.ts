import { describe, expect, it } from 'vitest';
import { Drafts, samePlace, type Place } from '../src/app/drafts';

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
