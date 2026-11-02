import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  applyOp, CopyMismatch, defaultColorSettings, emptyGrade, keptColor, mergePatch, sortLinks,
  type Clip, type ClipTrack, type GradeNode, type Link, type Media, type Op, type Project, type Timeline,
} from '../src/studio/engine';

interface Example { name: string; before: Project; after: Project; done: Op; inverse: Op }
const examples: Example[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/ops-examples.json', import.meta.url), 'utf8'));

describe('the schema 3 project copy agrees with the Rust model', () => {
  for (const example of examples) {
    it(example.name, () => {
      const before = structuredClone(example.before);
      const changed = applyOp({ ...before, next_id: example.after.next_id }, example.done);
      expect(changed).toStrictEqual(example.after);
      expect(before).toStrictEqual(example.before);
      // Undo never reuses an id; compare at the next_id already allocated by the forward step.
      expect(applyOp(changed, example.inverse)).toStrictEqual({ ...before, next_id: example.after.next_id });
    });
  }
});

describe('the grades of the copy, where the examples do not reach', () => {
  const at = (name: string) => structuredClone(examples.find(example => example.name === name)!);
  const clipOf = (project: Project) => (project.timelines[0].tracks[0] as ClipTrack).clips[0];
  const target = { level: 'clip', timeline: 't1', clip: 'c1' } as const;

  it('orders links as Rust orders strings, by code point, then rgb before key, then input as a number', () => {
    const mixer = (id: string): GradeNode => ({ id, kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 12 } });
    // U+FF5E is before U+1F600 in UTF-8, but after it in UTF-16 code units.
    const ids = ['\u{1F600}', '\u{FF5E}', 'n2', 'n10'];
    let project = at('add_node between the source and the output').before;
    project = applyOp(project, { op: 'set_grade', grade: target, value: { nodes: ids.map(mixer), links: [] } });
    for (const link of [
      { from: 'n2', to: '\u{1F600}', kind: 'rgb', input: 0 }, { from: 'n2', to: '\u{FF5E}', kind: 'rgb', input: 0 },
      { from: 'source', to: 'n2', kind: 'rgb', input: 10 }, { from: 'source', to: 'n2', kind: 'rgb', input: 2 },
      { from: 'n10', to: 'output', kind: 'rgb', input: 0 }, { from: 'source', to: 'n10', kind: 'rgb', input: 0 },
    ] as Link[]) project = applyOp(project, { op: 'connect', grade: target, link });
    expect(clipOf(project).grade.links.map(link => `${link.to}:${link.input}`)).toEqual(['n10:0', 'n2:2', 'n2:10', 'output:0', '\u{FF5E}:0', '\u{1F600}:0']);
    expect(sortLinks([{ from: 'a', to: 'n1', kind: 'key', input: 0 }, { from: 'b', to: 'n1', kind: 'rgb', input: 3 }]).map(link => link.kind)).toEqual(['rgb', 'key']);
  });

  it('merges a patch of params as the engine does: objects field by field, anything else whole, null too', () => {
    const base = { lut: 'a.cube', primaries: { lift: { y: 0, r: 0 }, contrast: 1 }, curves: { y: { points: [[0, 0], [1, 1]] } } };
    expect(mergePatch(base, { lut: null, primaries: { lift: { r: 0.1 }, contrast: { keys: [{ frame: 0, value: 1.2, interp: 'hold' }] } }, curves: { y: { points: [[0, 0.1]] } } }))
      .toEqual({ lut: null, primaries: { lift: { y: 0, r: 0.1 }, contrast: { keys: [{ frame: 0, value: 1.2, interp: 'hold' }] } }, curves: { y: { points: [[0, 0.1]] } } });
    expect(mergePatch({ keys: [] }, 2)).toBe(2);
    expect(base.primaries.lift.r).toBe(0);
  });

  it('bridges a node taken away without links, as the engine does', () => {
    const example = at('remove_node, bridged');
    const { links: _bridge, ...op } = example.done as Extract<Op, { op: 'remove_node' }>;
    expect(applyOp({ ...example.before, next_id: example.after.next_id }, op)).toStrictEqual(example.after);
  });

  it('reads a clip and a timeline built without grades, as the engine reads them', () => {
    const example = at('insert_clip of media');
    const { grade: _grade, group: _group, ...clip } = (example.done as Extract<Op, { op: 'insert_clip' }>).clip;
    expect(applyOp({ ...example.before, next_id: example.after.next_id }, { ...(example.done as Extract<Op, { op: 'insert_clip' }>), clip: clip as Clip })).toStrictEqual(example.after);
    const added = at('add_timeline');
    const { grade: _timelineGrade, ...timeline } = (added.done as Extract<Op, { op: 'add_timeline' }>).timeline;
    expect(applyOp({ ...added.before, next_id: added.after.next_id }, { op: 'add_timeline', index: 0, timeline: timeline as Timeline })).toStrictEqual(added.after);
    expect(emptyGrade()).toEqual(clipOf(added.after).grade);
  });

  it('refuses what names a grade, a node, a link or a group that the copy does not hold', () => {
    const project = at('set_node').before;
    for (const op of [
      { op: 'set_node', grade: { level: 'pre_clip', group: 'k1' }, id: 'n9', label: 'x' },
      { op: 'set_node', grade: target, id: 'n99', label: 'x' },
      { op: 'disconnect', grade: target, to: 'output', kind: 'key', input: 0 },
      { op: 'remove_group', id: 'k1' },
      { op: 'set_clip_group', timeline: 't1', clip: 'c99', group: null },
    ] as Op[]) expect(() => applyOp(project, op)).toThrow(CopyMismatch);
  });
});

describe('the colour management of the copy, where the examples do not reach', () => {
  const at = (name: string) => structuredClone(examples.find(example => example.name === name)!);

  it('has the defaults the engine writes, and reads a project without colour settings as the engine does', () => {
    const example = at('set_color_settings to ACEScct');
    expect(defaultColorSettings()).toStrictEqual(example.before.color);
    const { color: _color, ...before } = example.before;
    expect(keptColor({})).toStrictEqual(defaultColorSettings());
    expect(keptColor({ color: { science: 'acescct' } as never }).odt).toBe('Rec.709');
    expect(applyOp({ ...before, next_id: example.after.next_id }, example.done)).toStrictEqual(example.after);
  });

  it('merges a patch of colour settings field by field into the objects within it', () => {
    const example = at('set_color_settings of a LUT of the project');
    const project = applyOp(example.before, { op: 'set_color_settings', settings: { output_lut: { '1d': 'shaper.cube' } } });
    const after = applyOp(project, example.done);
    expect(after.color?.output_lut).toStrictEqual({ '1d': 'shaper.cube', '3d': 'Film/Kodak 2383.cube' });
    expect(after.color?.lut_interpolation).toBe('tetrahedral');
  });

  it('sets the colour of media by hand, keeping what is left out and clearing a space set to null', () => {
    const example = at('set_media_color');
    const strip = (project: Project): Project => ({ ...project, media: project.media.map(({ color_override: _o, ...media }) => media as Media) });
    expect(applyOp({ ...strip(example.before), next_id: example.after.next_id }, example.done)).toStrictEqual(example.after);
    const set = applyOp(example.after, { op: 'set_media_color', id: 'm1', levels: 'video' });
    const none = { '1d': null, '3d': null };
    expect(set.media[0].color_override).toStrictEqual({ input: 'DJI D-Gamut/D-Log', levels: 'video', lut: none, inverse_drt: true });
    expect(applyOp(set, { op: 'set_media_color', id: 'm1', input: null }).media[0].color_override)
      .toStrictEqual({ input: null, levels: 'video', lut: none, inverse_drt: true });
    expect(() => applyOp(example.before, { op: 'set_media_color', id: 'm99', levels: 'full' })).toThrow(CopyMismatch);
  });

  it('sets the LUTs of a source clip as a pair, and its Inverse DRT, as the engine does', () => {
    const example = at("set_media_color of a source clip's LUTs and Inverse DRT");
    expect(applyOp(example.before, example.done)).toStrictEqual(example.after);
    expect(applyOp(example.after, example.inverse)).toStrictEqual(example.before);
    // A LUT left out of the pair is none.
    const one = applyOp(example.after, { op: 'set_media_color', id: 'm1', lut: { '3d': 'b.cube' } as never });
    expect(one.media[0].color_override?.lut).toStrictEqual({ '1d': null, '3d': 'b.cube' });
    expect(one.media[0].color_override?.inverse_drt).toBe(false);
  });
});
