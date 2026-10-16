import { describe, expect, it } from 'vitest';
import {
  added, ATTACHMENTS_MOST, Attachments, attachedOf, isPicture, keptOf, sizeOf, withAttachments, type Attachment,
} from '../src/app/attachments';

const file = (id: string, name = `${id}.md`, size = 100): Attachment => ({ id, name, size });

describe('what the shell says of the files that it takes', () => {
  it('is read as the files that it holds and those that it did not take, and why', () => {
    expect(attachedOf({
      attached: [{ id: 'attachment-1', name: 'notes.md', size: 1200, path: 'never read' }, { id: 'attachment-2', name: 'empty.txt', size: 0 }],
      refused: [{ name: 'photos', why: 'notAFile' }, { name: 'film.mp4', why: 'tooBig' }, { name: 'gone.txt', why: 'gone' }, { name: 'x', why: 'tooMany' }],
    })).toEqual({
      attached: [{ id: 'attachment-1', name: 'notes.md', size: 1200 }, { id: 'attachment-2', name: 'empty.txt', size: 0 }],
      refused: [{ name: 'photos', why: 'notAFile' }, { name: 'film.mp4', why: 'tooBig' }, { name: 'gone.txt', why: 'gone' }, { name: 'x', why: 'tooMany' }],
    });
  });

  it('leaves out what it cannot read, and reads nothing as nothing', () => {
    expect(attachedOf({
      attached: [
        { id: '', name: 'a', size: 1 }, { id: 'b', name: '', size: 1 }, { id: 'c', name: 'c', size: -1 }, { id: 'd', name: 'd', size: '1' },
        { id: 'e', name: 'e', size: Number.NaN }, { id: 'f', name: 'f', size: Number.POSITIVE_INFINITY }, null, 'g', { id: 7, name: 'h', size: 1 },
      ],
      refused: [{ name: '', why: 'gone' }, { name: 'x', why: 'other' }, null],
    })).toEqual({ attached: [], refused: [] });
    for (const answer of [null, undefined, 'files', 7, { attached: 'many', refused: {} }]) {
      expect(attachedOf(answer), String(answer)).toEqual({ attached: [], refused: [] });
    }
  });
});

describe('where the shell says the files of a message are now', () => {
  it('is a path for each file, from the folder of the project', () => {
    const kept = ['.columbina/attachments/20261016-153012/notes.md', '.columbina/attachments/20261016-153012/data (2).csv'];
    expect(keptOf({ kept }, 2)).toEqual(kept);
  });

  it('is nothing where it names another number of files, or what cannot stand in a message', () => {
    expect(keptOf({ kept: ['a.md'] }, 2)).toBeNull();
    expect(keptOf({ kept: ['a.md', 'b.md'] }, 1)).toBeNull();
    for (const kept of [[''], [7], ['a\nb.md'], ['a\rb.md'], ['a\u0007b.md']]) expect(keptOf({ kept }, 1), JSON.stringify(kept)).toBeNull();
    // Each path stands on a line of its own, so a backquote in a name is no harm.
    expect(keptOf({ kept: ['a`b.md'] }, 1)).toEqual(['a`b.md']);
    for (const answer of [null, undefined, 'a.md', { kept: 'a.md' }]) expect(keptOf(answer, 1), String(answer)).toBeNull();
  });
});

describe('the files of a message', () => {
  it('take what comes, up to ten, and say which came over that', () => {
    expect(ATTACHMENTS_MOST).toBe(10);
    const eight = Array.from({ length: 8 }, (_unused, index) => file(`a${index}`));
    const coming = [file('b1'), file('b2'), file('b3')];
    expect(added(eight, coming)).toEqual({ files: [...eight, file('b1'), file('b2')], over: [file('b3')] });
    expect(added([], coming)).toEqual({ files: coming, over: [] });
    const ten = [...eight, file('b1'), file('b2')];
    expect(added(ten, [file('c')])).toEqual({ files: ten, over: [file('c')] });
    // A message with more than it may have takes nothing more, and gives nothing back.
    expect(added([...ten, file('d')], [file('e'), file('f')])).toEqual({ files: [...ten, file('d')], over: [file('e'), file('f')] });
  });

  it('are kept with their task, as its draft is', () => {
    const kept = new Attachments();
    const here = { project: 'F:\\work', task: 's-1' };
    const there = { project: 'F:\\work', task: 's-2' };
    const elsewhere = { project: 'F:\\other', task: 's-1' };
    expect(kept.of(here)).toEqual([]);
    const files = [file('a'), file('b')];
    kept.keep(here, files);
    files.push(file('c'));
    expect(kept.of(here)).toEqual([file('a'), file('b')]);
    expect(kept.of(there)).toEqual([]);
    expect(kept.of(elsewhere)).toEqual([]);
    kept.keep(here, []);
    expect(kept.of(here)).toEqual([]);
  });
});

describe('a size', () => {
  it('is written in bytes, kilobytes and megabytes, as a person reads it', () => {
    expect([0, 900, 1023, 1024, 1500, 12 * 1024, 1024 * 1024 - 1].map(sizeOf)).toEqual(['0 B', '900 B', '1023 B', '1 KB', '1 KB', '12 KB', '1024 KB']);
    expect([1024 * 1024, 3.44 * 1024 * 1024, 9.96 * 1024 * 1024, 10 * 1024 * 1024, 25 * 1024 * 1024].map(sizeOf))
      .toEqual(['1.0 MB', '3.4 MB', '10.0 MB', '10 MB', '25 MB']);
  });
});

describe('a picture', () => {
  it('is known by the ending of its name, in whatever case', () => {
    for (const name of ['shot.png', 'PHOTO.JPG', 'a.jpeg', 'b.gif', 'c.webp', 'd.bmp', 'e.heic', 'f.HEIF', 'g.tif', 'h.tiff', 'i.avif', 'j.ico']) {
      expect(isPicture(name), name).toBe(true);
    }
    // A drawing that is written as text is read as text.
    for (const name of ['notes.md', 'logo.svg', 'png', 'a.png.txt', 'pngs', 'archive.zip']) expect(isPicture(name), name).toBe(false);
  });
});

describe('a message with files', () => {
  const kept = ['.columbina/attachments/20261016-153012/notes.md', '.columbina/attachments/20261016-153012/data (2).csv'];

  it('ends with the heading and where each file is', () => {
    expect(withAttachments('  Sum these up.  ', 'Attached files:', kept)).toBe(
      'Sum these up.\n\nAttached files:\n- .columbina/attachments/20261016-153012/notes.md\n'
        + '- .columbina/attachments/20261016-153012/data (2).csv',
    );
  });

  it('is the list alone where nothing was written', () => {
    expect(withAttachments(' \n ', '附件：', kept.slice(0, 1))).toBe('附件：\n- .columbina/attachments/20261016-153012/notes.md');
  });
});
