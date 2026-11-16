import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  applyOp, CopyMismatch, defaultColorSettings, emptyGrade, firstComposition, fixedOf, keptColor, mergePatch, SOUND_FIELDS,
  sortFusionLinks, sortLinks,
  type Clip, type ClipTrack, type FusionLink, type FusionNode, type FusionPolygon, type GradeNode, type Link, type Media, type Op, type Project, type Timeline,
} from '../src/studio/engine';

interface Example { name: string; before: Project; after: Project; done: Op; inverse: Op }
const examples: Example[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/ops-examples.json', import.meta.url), 'utf8'));

describe('the schema 5 project copy agrees with the Rust model', () => {
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

describe('the sound of schema 4 in the copy is what the engine writes, field by field', () => {
  const example = examples.find(each => each.name === 'add_track of sound, 5.1, muted, with a strip')!;
  const keys = (value: object) => Object.keys(value).sort();

  it('names every field of a track of sound, its strip, its clips and its timeline', () => {
    const timeline = example.after.timelines[0];
    const track = timeline.tracks.find(each => each.kind === 'audio' && each.format === '5.1') as ClipTrack;
    expect(keys(track)).toEqual([...SOUND_FIELDS.track, 'id', 'kind', 'name', 'enabled', 'locked', 'clips', 'transitions'].sort());
    expect(keys(track.strip!)).toEqual([...SOUND_FIELDS.strip].sort());
    expect(keys(track.clips[0]).filter(key => (SOUND_FIELDS.clip as string[]).includes(key))).toEqual([...SOUND_FIELDS.clip].sort());
    expect(keys(timeline).filter(key => (SOUND_FIELDS.timeline as string[]).includes(key))).toEqual([...SOUND_FIELDS.timeline].sort());
    expect(keys(timeline.audio!)).toEqual([...SOUND_FIELDS.timelineAudio].sort());
    expect(example.after.schema).toBe(5);
    // A track of picture writes none of it.
    const picture = timeline.tracks.find(each => each.kind === 'video')!;
    expect(keys(picture).filter(key => (SOUND_FIELDS.track as string[]).includes(key))).toEqual([]);
  });

  it('reads a channel of one source, of several and of none as the engine writes them', () => {
    const track = example.after.timelines[0].tracks.find(each => each.kind === 'audio' && each.format === '5.1') as ClipTrack;
    const sources = track.clips[0].channels!.map(map => map.source);
    expect(sources.slice(0, 4)).toEqual([{ stream: 1, channel: 0 }, { stream: 1, channel: 1 }, [{ stream: 1, channel: 0 }, { stream: 1, channel: 1 }], null]);
    expect(fixedOf(track.clips[0].gain_db)).toBe(0);
    const keyed = examples.find(each => each.name === 'set_clip of a gain of keyframes')!;
    expect(fixedOf((keyed.done as Extract<Op, { op: 'set_clip' }>).gain_db!)).toBeNull();
  });
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

describe('the compositions of the copy, where the examples do not reach', () => {
  const at = (name: string) => structuredClone(examples.find(example => example.name === name)!);
  const clipOf = (project: Project) => (project.timelines[0].tracks[0] as ClipTrack).clips[0];
  /** The composition of c1 in the examples of a Merge over a Background. */
  const merged = () => structuredClone(clipOf(at('set_fusion_node').before).fusion!);

  it('orders links by the node that they go into, as Rust orders strings, and then by the name of the input', () => {
    const links: FusionLink[] = [
      { from: 'a', to: 'n9', input: 'foreground' }, { from: 'b', to: 'n9', input: 'background' },
      { from: 'n9', to: 'MediaOut1', input: 'input' }, { from: 'c', to: 'n10', input: 'input' },
    ];
    expect(sortFusionLinks(links).map(link => `${link.to}.${link.input}`))
      .toEqual(['MediaOut1.input', 'n10.input', 'n9.background', 'n9.foreground']);
    // The engine keeps them so: every composition that it writes is in that order.
    expect(sortFusionLinks([...merged().links].reverse())).toStrictEqual(merged().links);
  });

  it('has the composition that a clip has at first as the engine writes it, and writes none on a clip without one', () => {
    const example = at('set_fusion of the composition that a clip has at first');
    expect((example.done as Extract<Op, { op: 'set_fusion' }>).value).toStrictEqual(firstComposition());
    expect('fusion' in clipOf(example.before)).toBe(false);
    expect(clipOf(example.after).fusion).toStrictEqual(firstComposition());
    // Links given out of their order are put in it, as the engine puts them.
    const reversed = { ...merged(), links: [...merged().links].reverse() };
    const set = applyOp(example.before, { op: 'set_fusion', timeline: 't1', clip: 'c1', value: reversed });
    expect(clipOf(set).fusion).toStrictEqual(merged());
    expect('fusion' in clipOf(applyOp(set, { op: 'set_fusion', timeline: 't1', clip: 'c1', value: null }))).toBe(false);
  });

  it('refuses what names a clip without a composition, or a node, a link or a setting that the copy does not hold', () => {
    const plain = at('set_fusion of the composition that a clip has at first').before;
    const project = at('set_fusion_node').before;
    for (const [on, op] of [
      [plain, { op: 'connect_fusion', timeline: 't1', clip: 'c1', link: { from: 'MediaIn1', to: 'MediaOut1', input: 'input' } }],
      [plain, { op: 'set_fusion_keys', timeline: 't1', clip: 'c1', path: 'n9.blend', param: 1 }],
      [project, { op: 'set_fusion_node', timeline: 't1', clip: 'c1', id: 'n99', label: 'x' }],
      [project, { op: 'remove_fusion_node', timeline: 't1', clip: 'c1', id: 'n99' }],
      [project, { op: 'disconnect_fusion', timeline: 't1', clip: 'c1', to: 'n9', input: 'input' }],
      [project, { op: 'set_fusion_keys', timeline: 't1', clip: 'c1', path: 'n9.apply', param: 1 }],
      [project, { op: 'set_fusion_keys', timeline: 't1', clip: 'c1', path: 'n99.blend', param: 1 }],
      [project, { op: 'set_fusion_keys', timeline: 't1', clip: 'c1', path: 'blend', param: 1 }],
      [project, { op: 'splice_fusion_keys', timeline: 't1', clip: 'c1', path: 'n9.blend', at: 0, remove: 1, keys: [] }],
      [project, { op: 'set_fusion', timeline: 't1', clip: 'c99', value: null }],
    ] as [Project, Op][]) expect(() => applyOp(on, op)).toThrow(CopyMismatch);
  });
});

describe('the masks of the compositions of the copy', () => {
  const at = (name: string) => structuredClone(examples.find(example => example.name === name)!);
  const clipOf = (project: Project) => (project.timelines[0].tracks[0] as ClipTrack).clips[0];
  const AT = { timeline: 't1', clip: 'c1' } as const;
  /** A mask of each kind as the engine writes a new one: every setting, none left out. */
  const common = { level: 1, soft_edge: 0, border_width: 0, invert: false, solid: true, paint_mode: 'merge' } as const;
  const ellipse: FusionNode = {
    id: 'e1', kind: 'ellipse', label: '', pass_through: false, pos: null,
    params: { ...common, center: { x: 0.5, y: 0.5 }, width: 0.5, height: 0.5, angle: 0 },
  };
  const rectangle: FusionNode = {
    id: 'r1', kind: 'rectangle', label: '', pass_through: false, pos: null,
    params: { ...common, center: { x: 0.5, y: 0.5 }, width: 0.5, height: 0.5, corner_radius: 0, angle: 0 },
  };
  const polygon: FusionNode = {
    id: 'p1', kind: 'polygon', label: '', pass_through: false, pos: null,
    params: { ...common, center: { x: 0.5, y: 0.5 }, size: 1, angle: 0, points: [{ x: -0.1, y: -0.1 }, { x: 0.1, y: -0.1 }, { x: 0, y: 0.1 }], fill: 'alternate' },
  };
  /** The composition of a Merge, n9, over a Background, n10, with the Merge limited by the ellipse, the rectangle coming into the ellipse, and the polygon read by nothing. */
  const masked = (): Project => {
    let project = at('set_fusion_node').before;
    project = applyOp(project, { op: 'add_fusion_node', ...AT, node: ellipse, links: [{ from: 'e1', to: 'n9', input: 'mask' }] });
    project = applyOp(project, { op: 'add_fusion_node', ...AT, node: rectangle, links: [{ from: 'r1', to: 'e1', input: 'mask' }] });
    return applyOp(project, { op: 'add_fusion_node', ...AT, node: polygon });
  };
  const nodeOf = (project: Project, id: string) => clipOf(project).fusion!.nodes.find(node => node.id === id)!;

  it('keeps a mask after the pictures of its node, as the engine orders the names of the inputs', () => {
    const links: FusionLink[] = [
      { from: 'a', to: 'n9', input: 'mask' }, { from: 'b', to: 'n9', input: 'foreground' }, { from: 'c', to: 'n9', input: 'background' },
      { from: 'd', to: 'n10', input: 'mask' }, { from: 'e', to: 'n10', input: 'input' },
    ];
    expect(sortFusionLinks(links).map(link => `${link.to}.${link.input}`)).toEqual(['n10.input', 'n10.mask', 'n9.background', 'n9.foreground', 'n9.mask']);
    expect(clipOf(masked()).fusion!.links.map(link => `${link.from}>${link.to}.${link.input}`)).toEqual([
      'n9>MediaOut1.input', 'r1>e1.mask', 'n10>n9.background', 'MediaIn1>n9.foreground', 'e1>n9.mask',
    ]);
  });

  it('adds a mask with every setting, links it in place of the mask that an input had, and takes it away with its links', () => {
    const project = masked();
    expect(clipOf(project).fusion!.nodes.map(node => node.kind)).toEqual(['media_in', 'media_out', 'merge', 'background', 'ellipse', 'rectangle', 'polygon']);
    expect(nodeOf(project, 'e1')).toStrictEqual(ellipse);
    const relinked = applyOp(project, { op: 'connect_fusion', ...AT, link: { from: 'p1', to: 'n9', input: 'mask' } });
    expect(clipOf(relinked).fusion!.links.filter(link => link.input === 'mask').map(link => `${link.from}>${link.to}`)).toEqual(['r1>e1', 'p1>n9']);
    const unlinked = applyOp(relinked, { op: 'disconnect_fusion', ...AT, to: 'n9', input: 'mask' });
    expect(clipOf(unlinked).fusion!.links.some(link => link.to === 'n9' && link.input === 'mask')).toBe(false);
    const removed = applyOp(project, { op: 'remove_fusion_node', ...AT, id: 'e1', links: [{ from: 'r1', to: 'n9', input: 'mask' }] });
    expect(clipOf(removed).fusion!.links.filter(link => link.input === 'mask')).toEqual([{ from: 'r1', to: 'n9', input: 'mask' }]);
    expect(project).toStrictEqual(masked());
  });

  it('sets the controls of a mask by a patch, the points of a polygon whole, and a number of a point by its place', () => {
    const project = masked();
    const set = applyOp(project, { op: 'set_fusion_node', ...AT, id: 'e1', params: { invert: true, paint_mode: 'subtract', soft_edge: 0.05, center: { x: 0.25 } } });
    expect(nodeOf(set, 'e1').params).toStrictEqual({ ...ellipse.params, invert: true, paint_mode: 'subtract', soft_edge: 0.05, center: { x: 0.25, y: 0.5 } });
    const keys = { keys: [{ frame: 0, value: 0.1, interp: 'linear' as const }, { frame: 24, value: 0.3, interp: 'linear' as const }] };
    const keyed = applyOp(project, { op: 'set_fusion_keys', ...AT, path: 'p1.points.2.y', param: keys });
    expect((nodeOf(keyed, 'p1').params as FusionPolygon).points).toStrictEqual([{ x: -0.1, y: -0.1 }, { x: 0.1, y: -0.1 }, { x: 0, y: keys }]);
    const spliced = applyOp(keyed, { op: 'splice_fusion_keys', ...AT, path: 'p1.points.2.y', at: 1, remove: 1, keys: [] });
    expect((nodeOf(spliced, 'p1').params as FusionPolygon).points[2].y).toStrictEqual({ keys: [keys.keys[0]] });
    // A list is put in place of a list, whole: the points that had keys are gone with them.
    const redrawn = applyOp(keyed, { op: 'set_fusion_node', ...AT, id: 'p1', params: { fill: 'non_zero_winding', points: [{ x: 0, y: 0 }] } });
    expect(nodeOf(redrawn, 'p1').params).toStrictEqual({ ...polygon.params, fill: 'non_zero_winding', points: [{ x: 0, y: 0 }] });
    // A point that the polygon has not, a point whole, and a setting that takes no keys are not settings of the copy.
    for (const path of ['p1.points.3.x', 'p1.points.2', 'p1.points', 'e1.invert', 'e1.paint_mode', 'e1.corner_radius', 'p1.fill']) {
      expect(() => applyOp(project, { op: 'set_fusion_keys', ...AT, path, param: 0.5 }), path).toThrow(CopyMismatch);
    }
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
