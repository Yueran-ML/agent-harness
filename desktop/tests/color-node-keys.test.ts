/**
 * The keyframes of the nodes of a grade, as the Keyframe Editor of the Color page keeps them
 * (`src/studio/color/nodeKeys`, DaVinci Resolve Reference Manual 20, Ch. 148): the settings that
 * take keys are those of the engine's catalog; a keyframe of a node is a key on each of them; the
 * operations of each command, in each mode, applied as the page's copy of the project applies
 * them; dynamic and static; the eases of Change Dynamic Attributes; what Auto-Keyframe makes of a
 * change; and the keys of the keyboard. And its geometry (`src/studio/color/keyTracks`): the frames
 * of keys through the clock of a clip, the marks of the tracks, and what a pointer is on.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Key } from '../src/app/keys';
import {
  applyOp, emptyGrade, mergePatch, type Corrector, type Grade, type GradeNode, type Interp, type Keyframe, type Op, type Param, type Project, type Target,
} from '../src/studio/engine';
import { fullCorrector, getAt } from '../src/studio/color/corrector';
import { keyClockOf } from '../src/studio/color/keyFrames';
import {
  DIAMOND, LANE, LANES_WIDTH, easePath, frameAtX, hitMark, keyFrameAt, keyRange, lanesHeight, marksOf, rowAt, rowMiddle, rowTop, rulerTicks, stepTime,
  timelineFrameOf, timesOf, xOfFrame, TIMELINE_CLOCK, type KeySpan,
} from '../src/studio/color/keyTracks';
import { makeDefaultNode, onDefaultNode, type Worked } from '../src/studio/color/nodeEdits';
import {
  EASE_MOST, EASE_REACH, EASE_STEP, NODE_KEYED, addKeyframe, addKeyframes, autoKeyed, autoValue, autoWhole, changeKeyframes, deleteAllKeyframes,
  deleteKeyframes, easeAmount, easeInterp, easeKeyframes, easeOf, easeShare, easesOf, hasKeyframes, keyedAt, keyframeFrames, keyframeKey, keyframesOf,
  masterKeyframes, moveKeyframe, moveRange, nodesFor, settingOps, settingsIn, settingsOf, tracksOf, withKeyframe, withKeyframeMoved, type KeyOp,
} from '../src/studio/color/nodeKeys';
import { newWindow } from '../src/studio/color/windowEdits';
import { setInterp } from '../src/studio/keyframes/curveGeometry';
import { EASE_SHARE, valueAt } from '../src/studio/keyframes/value';
import { clip, media, timeline } from './timeline-fixture';

interface Entry { path: string }
const catalog: { corrector: Entry[]; key_mixer: Entry[] } = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/catalog.json', import.meta.url), 'utf8'));

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const RATE = { num: 25, den: 1 };
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
/** A corrector as the engine makes one, with `params` put over its settings. */
const corrector = (id: string, params: unknown = {}, locked = false): GradeNode =>
  ({ id, kind: 'corrector', label: '', enabled: true, locked, params: mergePatch(fullCorrector({}), params) as Corrector });
const mixer = (id: string): GradeNode => ({ id, kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 2 } });
const keyMixer = (id: string): GradeNode => ({ id, kind: 'key_mixer', label: '', enabled: true, params: {
  inputs: [{ invert: false, matte: false, gain: 1, offset: 0 }, { invert: false, matte: false, gain: 0.5, offset: 0.1 }],
  output: { invert: false, gain: 1, offset: 0 },
} });
const gradeOf = (...nodes: GradeNode[]): Grade => ({ nodes, links: emptyGrade().links });
const projectOf = (grade: Grade): Project => ({ schema: 5, next_id: 10, bins: [], groups: [], media: [media], timelines: [timeline([clip('c1', 0, 100, { grade })])] });

/** A grade after the operations of a command, as the page's copy of the project applies them. */
function applied(grade: Grade, worked: Worked): Grade {
  if (!Array.isArray(worked)) throw new Error('The command gave no operations.');
  let project = projectOf(grade);
  for (const op of worked) project = applyOp(project, op as Op);
  const track = project.timelines[0].tracks[0];
  if (track.kind === 'subtitle') throw new Error('The track holds no clips.');
  return track.clips[0].grade;
}
const nodeOf = (grade: Grade, id: string): GradeNode => grade.nodes.find(node => node.id === id)!;
const framesOf = (node: GradeNode): number[] => keyframesOf(node).map(each => each.frame);
const opsOf = (worked: Worked): KeyOp[] => {
  if (!Array.isArray(worked)) throw new Error('The command gave no operations.');
  return worked as KeyOp[];
};
/** How many settings of a corrector as the engine makes one take keys. */
const PLAIN = settingsOf(corrector('n1')).length;

describe('the settings of a node that take keys', () => {
  it('are those of the catalog of the engine, in its order', () => {
    expect(NODE_KEYED.corrector).toEqual(catalog.corrector.map(entry => entry.path));
    expect(NODE_KEYED.key_mixer).toEqual(catalog.key_mixer.map(entry => entry.path));
    expect(NODE_KEYED.parallel_mixer).toEqual([]);
    expect(NODE_KEYED.layer_mixer).toEqual([]);
  });

  it('are found in a node by their paths, a star for each place of a list, and what the node has not left out', () => {
    const plain = settingsOf(corrector('n1'));
    // Primaries 26, Log 11, the four intensities, Soft Clip 12, Key 8, Blur 12; and the two points of each of the four curves.
    expect(PLAIN).toBe(73 + 16);
    expect(new Set(plain.map(each => each.path)).size).toBe(PLAIN);
    expect(plain.every(each => typeof each.param === 'number')).toBe(true);
    expect(plain.slice(0, 4).map(each => each.path)).toEqual(['primaries.lift.y', 'primaries.lift.r', 'primaries.lift.g', 'primaries.lift.b']);
    expect(plain.filter(each => each.path.startsWith('curves.y.')).map(each => [each.path, each.param])).toEqual([
      ['curves.y.intensity', 100], ['curves.y.points.0.0', 0], ['curves.y.points.1.0', 1], ['curves.y.points.0.1', 0], ['curves.y.points.1.1', 1],
    ]);
    // A new corrector has no qualifier, no shot match, no window and no point on an HSL curve.
    expect(plain.some(each => /^(qualifier|shot_match|windows|hsl)\./.test(each.path))).toBe(false);
    // A window of each shape has the settings of its shape: one softness or four, and the points of a linear one.
    const windowed = settingsOf(corrector('n1', { windows: [newWindow('w1', 'circular'), newWindow('w2', 'linear')] })).map(each => each.path);
    expect(windowed).toHaveLength(PLAIN + 7 + 18);
    expect(windowed.filter(path => path.startsWith('windows.0.'))).toEqual([
      'windows.0.opacity', 'windows.0.transform.size', 'windows.0.transform.aspect', 'windows.0.transform.pan', 'windows.0.transform.tilt',
      'windows.0.transform.rotate', 'windows.0.shape.softness',
    ]);
    expect(windowed).toContain('windows.1.shape.softness.3');
    expect(windowed).toContain('windows.1.shape.points.2.y');
    expect(windowed).not.toContain('windows.1.shape.softness');
    // A setting with keys is one too; a Key Mixer has those of its inputs and of its output; the other mixers none.
    const keyed = corrector('n1', { primaries: { saturation: { keys: [key(0, 50)] } } });
    expect(settingsOf(keyed).find(each => each.path === 'primaries.saturation')!.param).toEqual({ keys: [key(0, 50)] });
    expect(settingsOf(keyMixer('k1')).map(each => [each.path, each.param])).toEqual([
      ['inputs.0.gain', 1], ['inputs.1.gain', 0.5], ['inputs.0.offset', 0], ['inputs.1.offset', 0.1], ['output.gain', 1], ['output.offset', 0],
    ]);
    expect(settingsOf(mixer('m1'))).toEqual([]);
    // Params that say nothing of a palette (a node that a client built) have none of its settings.
    expect(settingsIn('corrector', { primaries: { contrast: 1.2 } }).map(each => each.path)).toEqual(['primaries.contrast']);
  });
});

describe('a keyframe of a node', () => {
  it('is a key on every setting of the node, of the value that the setting has there, so that the picture stays', () => {
    const node = corrector('n1', { primaries: { saturation: 60 } });
    const ops = addKeyframe(TARGET, node, 20, 'dynamic');
    expect(ops).toHaveLength(PLAIN);
    expect(ops.every(op => op.op === 'splice_keys' && op.id === 'n1' && op.grade === TARGET && op.at === 0 && op.remove === 0 && op.keys.length === 1
      && op.keys[0].frame === 20 && op.keys[0].interp === 'linear')).toBe(true);
    expect(ops.find(op => op.path === 'primaries.saturation')).toEqual({ op: 'splice_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation', at: 0, remove: 0, keys: [key(20, 60)] });
    const after = nodeOf(applied(gradeOf(node), ops), 'n1');
    expect(after).toEqual(withKeyframe(node, 20, 'dynamic'));
    expect(keyframesOf(after)).toEqual([{ frame: 20, kind: 'dynamic', mixed: false }]);
    expect(hasKeyframes(after)).toBe(true);
    expect(hasKeyframes(node)).toBe(false);
    // Each setting is, at every frame, the number that it was.
    const before = settingsOf(node);
    expect(settingsOf(after).map(each => [each.path, valueAt(each.param, 0), valueAt(each.param, 20), valueAt(each.param, 90)]))
      .toEqual(before.map(each => [each.path, each.param, each.param, each.param]));
    // A static keyframe holds; one put where the node has one already changes nothing.
    expect(addKeyframe(TARGET, node, 20, 'static').every(op => op.op === 'splice_keys' && op.keys[0].interp === 'hold')).toBe(true);
    expect(addKeyframe(TARGET, after, 20, 'dynamic')).toEqual([]);
  });

  it('is put on the curve of a setting that has keys, between them, and keeps the shape of a Bezier stretch', () => {
    const node = corrector('n1', { primaries: { saturation: { keys: [key(0, 50), key(100, 100)] } } });
    const ops = addKeyframe(TARGET, node, 50, 'dynamic');
    expect(ops.find(op => op.path === 'primaries.saturation')).toEqual({ op: 'splice_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation', at: 1, remove: 0, keys: [key(50, 75)] });
    expect(ops.find(op => op.path === 'primaries.contrast')).toEqual({ op: 'splice_keys', grade: TARGET, id: 'n1', path: 'primaries.contrast', at: 0, remove: 0, keys: [key(50, 1)] });
    // A static keyframe holds from its frame to the next keyframe, as p. 3385 has it.
    expect(keyedAt({ keys: [key(0, 50), key(100, 100)] }, 50, 'static')).toEqual({ keys: [key(0, 50), key(50, 75, 'hold'), key(100, 100)] });
    // A dynamic keyframe put on a stretch that holds goes on in a line from the value held.
    expect(keyedAt({ keys: [key(0, 50, 'hold'), key(100, 100)] }, 50, 'dynamic')).toEqual({ keys: [key(0, 50, 'hold'), key(50, 50), key(100, 100)] });
    // A Bezier stretch is cut in two, and is the curve that it was.
    const eased: Param = { keys: [key(0, 0, easeInterp(2, 2)), key(100, 1)] };
    const cut = keyedAt(eased, 30, 'dynamic');
    expect(typeof cut === 'number' ? [] : cut.keys.map(each => each.frame)).toEqual([0, 30, 100]);
    for (const frame of [10, 25, 30, 55, 80, 95]) expect(valueAt(cut, frame)).toBeCloseTo(valueAt(eased, frame), 9);
  });

  it('is dynamic or static by its keys: where they are of both kinds it is shown as dynamic, and said to be mixed', () => {
    const mixed = corrector('n1', { primaries: { saturation: { keys: [key(10, 50, 'hold')] }, contrast: { keys: [key(10, 1), key(30, 1.2, 'hold')] } } });
    expect(keyframesOf(mixed)).toEqual([{ frame: 10, kind: 'dynamic', mixed: true }, { frame: 30, kind: 'static', mixed: false }]);
    // A Bezier stretch is dynamic.
    expect(keyframesOf(corrector('n1', { primaries: { hue: { keys: [key(5, 50, easeInterp(1, 0)), key(9, 60)] } } })).map(each => each.kind)).toEqual(['dynamic', 'dynamic']);
  });

  it('is taken away with every key at its frame, and the settings left without a key are numbers again', () => {
    const node = corrector('n1', { primaries: { saturation: 60 } });
    const two = withKeyframe(withKeyframe(node, 20, 'dynamic'), 40, 'static');
    expect(framesOf(two)).toEqual([20, 40]);
    const first = opsOf(deleteKeyframes(TARGET, [two], 20));
    expect(first).toHaveLength(PLAIN);
    expect(first.every(op => op.op === 'splice_keys' && op.at === 0 && op.remove === 1 && op.keys.length === 0)).toBe(true);
    const one = nodeOf(applied(gradeOf(two), first), 'n1');
    expect(keyframesOf(one)).toEqual([{ frame: 40, kind: 'static', mixed: false }]);
    // The last keyframe: each setting is set to the number of its key, since a setting keeps one key at least.
    const last = opsOf(deleteKeyframes(TARGET, [one], 40));
    expect(last).toHaveLength(PLAIN);
    expect(last.every(op => op.op === 'set_keys' && typeof op.param === 'number')).toBe(true);
    expect(last.find(op => op.path === 'primaries.saturation')).toEqual({ op: 'set_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation', param: 60 });
    expect(nodeOf(applied(gradeOf(one), last), 'n1')).toEqual(node);
    // Where the node has no keyframe there is nothing to do.
    expect(deleteKeyframes(TARGET, [node], 20)).toBeNull();
    expect(deleteKeyframes(TARGET, [two], 30)).toBeNull();
  });

  it('are all taken away at once, each setting left as it is at the playhead', () => {
    const node = corrector('n1', { primaries: { saturation: { keys: [key(0, 50), key(100, 100)] }, contrast: { keys: [key(0, 1, 'hold'), key(100, 2)] } } });
    const ops = opsOf(deleteAllKeyframes(TARGET, [node], 50));
    expect(ops).toEqual([
      { op: 'set_keys', grade: TARGET, id: 'n1', path: 'primaries.contrast', param: 1 },
      { op: 'set_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation', param: 75 },
    ]);
    const after = nodeOf(applied(gradeOf(node), ops), 'n1');
    expect(hasKeyframes(after)).toBe(false);
    expect(deleteAllKeyframes(TARGET, [after], 50)).toBeNull();
  });

  it('is changed to static and back to dynamic, key by key', () => {
    const node = withKeyframe(withKeyframe(corrector('n1'), 20, 'dynamic'), 60, 'dynamic');
    const held = opsOf(changeKeyframes(TARGET, [node], 20, 'static'));
    expect(held).toHaveLength(PLAIN);
    expect(held.every(op => op.op === 'splice_keys' && op.at === 0 && op.remove === 1 && op.keys.length === 1 && op.keys[0].frame === 20 && op.keys[0].interp === 'hold')).toBe(true);
    const still = nodeOf(applied(gradeOf(node), held), 'n1');
    expect(keyframesOf(still)).toEqual([{ frame: 20, kind: 'static', mixed: false }, { frame: 60, kind: 'dynamic', mixed: false }]);
    const back = nodeOf(applied(gradeOf(still), changeKeyframes(TARGET, [still], 20, 'dynamic')), 'n1');
    expect(back).toEqual(node);
    // What is of the kind already is left: a Bezier stretch stays as it is where it is made dynamic.
    expect(changeKeyframes(TARGET, [node], 20, 'dynamic')).toBeNull();
    const eased = nodeOf(applied(gradeOf(node), easeKeyframes(TARGET, [node], 20, 2, 1)), 'n1');
    expect(changeKeyframes(TARGET, [eased], 20, 'dynamic')).toBeNull();
    expect(changeKeyframes(TARGET, [node], 33, 'static')).toBeNull();
  });

  it('is moved in time with all its keys, no further than the keyframes beside it', () => {
    const node = [10, 20, 40].reduce((made, frame) => withKeyframe(made, frame, 'dynamic'), corrector('n1', { primaries: { saturation: 60 } }));
    expect(moveRange(node, 20)).toEqual([11, 39]);
    expect(moveRange(node, 10)).toEqual([-Infinity, 19]);
    expect(moveRange(node, 40)).toEqual([21, Infinity]);
    expect(moveRange(node, 15)).toBeNull();
    const ops = moveKeyframe(TARGET, node, 20, 30);
    expect(ops).toHaveLength(PLAIN);
    expect(ops.find(op => op.path === 'primaries.saturation')).toEqual({ op: 'set_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation',
      param: { keys: [key(10, 60), key(30, 60), key(40, 60)] } });
    const moved = nodeOf(applied(gradeOf(node), ops), 'n1');
    expect(framesOf(moved)).toEqual([10, 30, 40]);
    expect(moved).toEqual(withKeyframeMoved(node, 20, 30));
    // Put where it is, it is what puts the settings back.
    expect(nodeOf(applied(gradeOf(moved), moveKeyframe(TARGET, node, 20, 20)), 'n1')).toEqual(node);
    // Only the settings that have a key at the frame are sent.
    const lone = corrector('n2', { primaries: { saturation: { keys: [key(5, 50), key(9, 60)] } } });
    expect(moveKeyframe(TARGET, lone, 9, 12)).toEqual([{ op: 'set_keys', grade: TARGET, id: 'n2', path: 'primaries.saturation', param: { keys: [key(5, 50), key(12, 60)] } }]);
  });

  it('makes a setting what it is to be with one operation, or with none where it is that already', () => {
    expect(settingOps(TARGET, 'n1', 'primaries.hue', 50, 50)).toEqual([]);
    expect(settingOps(TARGET, 'n1', 'primaries.hue', { keys: [key(0, 50)] }, { keys: [key(0, 50)] })).toEqual([]);
    expect(settingOps(TARGET, 'n1', 'primaries.hue', { keys: [key(0, 50)] }, 50)).toEqual([{ op: 'set_keys', grade: TARGET, id: 'n1', path: 'primaries.hue', param: 50 }]);
    expect(settingOps(TARGET, 'n1', 'primaries.hue', 50, { keys: [key(4, 50)] })).toEqual([{ op: 'splice_keys', grade: TARGET, id: 'n1', path: 'primaries.hue', at: 0, remove: 0, keys: [key(4, 50)] }]);
    // The engine takes no setting of keys without one.
    expect(settingOps(TARGET, 'n1', 'primaries.hue', 50, { keys: [] })).toEqual([]);
  });
});

describe('the commands of the Keyframe Editor', () => {
  const grade = gradeOf(corrector('n1'), corrector('n2', { primaries: { saturation: 70 } }), mixer('n3'), keyMixer('n4'));

  it('are for every node in the All mode, for the current node in the Color mode, and for the track named', () => {
    expect(nodesFor(grade, 'scope', 'all', 'n2').map(node => node.id)).toEqual(['n1', 'n2', 'n3', 'n4']);
    expect(nodesFor(grade, 'scope', 'color', 'n2').map(node => node.id)).toEqual(['n2']);
    expect(nodesFor(grade, 'scope', 'color', null)).toEqual([]);
    expect(nodesFor(grade, 'master', 'color', 'n2').map(node => node.id)).toEqual(['n1', 'n2', 'n3', 'n4']);
    expect(nodesFor(grade, { node: 'n4' }, 'all', 'n2').map(node => node.id)).toEqual(['n4']);
  });

  it('add a keyframe to every track, or to the current node alone, as one transaction', () => {
    const all = opsOf(addKeyframes(TARGET, nodesFor(grade, 'scope', 'all', 'n2'), 20, 'dynamic'));
    // The two correctors and the Key Mixer; a Parallel Mixer has nothing to key.
    expect(all).toHaveLength(2 * PLAIN + 6);
    expect([...new Set(all.map(op => op.id))]).toEqual(['n1', 'n2', 'n4']);
    const everywhere = applied(grade, all);
    expect(everywhere.nodes.map(framesOf)).toEqual([[20], [20], [], [20]]);
    expect(keyframeFrames(everywhere.nodes)).toEqual([20]);
    const color = opsOf(addKeyframes(TARGET, nodesFor(grade, 'scope', 'color', 'n2'), 20, 'static'));
    expect(color).toHaveLength(PLAIN);
    expect(color.every(op => op.id === 'n2' && op.op === 'splice_keys' && op.keys[0].interp === 'hold')).toBe(true);
    expect(applied(grade, color).nodes.map(framesOf)).toEqual([[], [20], [], []]);
    // The Color mode with a mixer current, or with no node, has nothing to do.
    expect(addKeyframes(TARGET, nodesFor(grade, 'scope', 'color', 'n3'), 20, 'dynamic')).toBeNull();
    expect(addKeyframes(TARGET, [], 20, 'dynamic')).toBeNull();
  });

  it('pass over a locked node, and are refused where only locked nodes would change', () => {
    const locked = corrector('n1', {}, true);
    const open = corrector('n2');
    expect(addKeyframes(TARGET, [locked], 20, 'dynamic')).toEqual({ refused: 'node_locked' });
    expect(opsOf(addKeyframes(TARGET, [locked, open], 20, 'dynamic')).every(op => op.id === 'n2')).toBe(true);
    expect(deleteAllKeyframes(TARGET, [withKeyframe(locked, 5, 'dynamic')], 0)).toEqual({ refused: 'node_locked' });
    // A locked node with nothing to change refuses nothing.
    expect(deleteKeyframes(TARGET, [locked], 20)).toBeNull();
  });

  it('make the default node of a grade without nodes first, in the same transaction', () => {
    const shown = corrector('n10');
    const worked = onDefaultNode(addKeyframes(TARGET, [shown], 5, 'dynamic'), { target: TARGET, id: 'n10' });
    expect(Array.isArray(worked) && worked[0]).toEqual(makeDefaultNode(TARGET));
    expect(Array.isArray(worked) && worked).toHaveLength(PLAIN + 1);
  });

  it('show on the master track every keyframe of every track', () => {
    const one = withKeyframe(corrector('n1'), 10, 'dynamic');
    const two = withKeyframe(withKeyframe(corrector('n2'), 10, 'static'), 30, 'static');
    const tracks = tracksOf(gradeOf(one, two, mixer('n3')));
    expect(tracks.map(track => track.node)).toEqual([null, 'n1', 'n2', 'n3']);
    expect(tracks[0].keyframes).toEqual([{ frame: 10, kind: 'dynamic', mixed: true }, { frame: 30, kind: 'static', mixed: false }]);
    expect(tracks[1].keyframes).toEqual([{ frame: 10, kind: 'dynamic', mixed: false }]);
    expect(tracks[2].keyframes).toEqual([{ frame: 10, kind: 'static', mixed: false }, { frame: 30, kind: 'static', mixed: false }]);
    expect(tracks[3].keyframes).toEqual([]);
    expect(masterKeyframes([])).toEqual([]);
  });

  it('are asked for by Ctrl+[, Ctrl+], Alt+] and the two brackets alone', () => {
    const pressed = (given: Partial<Key>): Key => ({ key: '', shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...given });
    expect(keyframeKey(pressed({ key: '[', code: 'BracketLeft', ctrl: true }))).toEqual({ kind: 'add', key: 'dynamic' });
    expect(keyframeKey(pressed({ key: ']', code: 'BracketRight', ctrl: true }))).toEqual({ kind: 'add', key: 'static' });
    expect(keyframeKey(pressed({ key: ']', code: 'BracketRight', alt: true }))).toEqual({ kind: 'delete' });
    // Alt changes what the key types on some layouts: it is known by where it lies.
    expect(keyframeKey(pressed({ key: '~', code: 'BracketRight', alt: true }))).toEqual({ kind: 'delete' });
    expect(keyframeKey(pressed({ key: '[' }))).toEqual({ kind: 'step', direction: -1 });
    expect(keyframeKey(pressed({ key: ']', repeat: true }))).toEqual({ kind: 'step', direction: 1 });
    // A key held down adds and deletes once; other chords and other keys ask nothing.
    expect(keyframeKey(pressed({ key: '[', code: 'BracketLeft', ctrl: true, repeat: true }))).toBeNull();
    expect(keyframeKey(pressed({ key: '[', code: 'BracketLeft', alt: true }))).toBeNull();
    expect(keyframeKey(pressed({ key: ']', code: 'BracketRight', ctrl: true, alt: true }))).toBeNull();
    expect(keyframeKey(pressed({ key: ']', code: 'BracketRight', shift: true }))).toBeNull();
    expect(keyframeKey(pressed({ key: '[', composing: true }))).toBeNull();
    expect(keyframeKey(pressed({ key: 'k', code: 'KeyK', ctrl: true }))).toBeNull();
  });
});

describe('Change Dynamic Attributes', () => {
  it('maps an ease to how far a level handle reaches: none at 0, a third at 2, two thirds at the most', () => {
    expect(easeShare(0)).toBe(0);
    expect(easeShare(2)).toBe(EASE_SHARE);
    expect(easeShare(EASE_MOST)).toBe(EASE_REACH);
    expect(easeShare(-3)).toBe(0);
    expect(easeShare(99)).toBe(EASE_REACH);
    expect(easeShare(Number.NaN)).toBe(0);
    for (let amount = 0; amount <= EASE_MOST; amount += EASE_STEP) expect(easeAmount(easeShare(amount))).toBe(amount);
  });

  it('writes no ease as a line, and eases as the two handles of a Bezier stretch, read back as they were given', () => {
    expect(easeInterp(0, 0)).toBe('linear');
    expect(easeInterp(2, 0)).toEqual({ bezier: { out: [EASE_SHARE, 0], in: [0, 0] } });
    expect(easeInterp(0, EASE_MOST)).toEqual({ bezier: { out: [0, 0], in: [EASE_REACH, 0] } });
    for (let start = 0; start <= EASE_MOST; start += EASE_STEP) {
      for (let end = 0; end <= EASE_MOST; end += EASE_STEP) expect(easesOf(easeInterp(start, end))).toEqual({ start, end });
    }
    expect(easesOf('hold')).toBeNull();
    // Both at 2 is the Ease In and Out of the Edit page's Curve Editor.
    expect(easeInterp(2, 2)).toEqual(setInterp([key(0, 0), key(10, 1)], [0, 1], 'in_out')[0].interp);
    // A stretch eased at its start alone leaves its key slowly and comes to the next at speed; one with no ease is the line.
    const eased: Param = { keys: [key(0, 0, easeInterp(EASE_MOST, 0)), key(100, 1)] };
    expect(valueAt(eased, 25)).toBeLessThan(0.25);
    expect(valueAt(eased, 50)).toBeLessThan(0.5);
    expect(valueAt({ keys: [key(0, 0, { bezier: { out: [0, 0], in: [0, 0] } }), key(100, 1)] }, 37)).toBeCloseTo(0.37, 9);
  });

  it('sets the stretch that leaves a dynamic keyframe, on every setting of the node, and shows what it set', () => {
    const node = withKeyframe(withKeyframe(corrector('n1', { primaries: { saturation: 60 } }), 0, 'dynamic'), 50, 'dynamic');
    expect(easeOf(node, 0)).toEqual({ start: 0, end: 0 });
    // The last keyframe leaves no stretch.
    expect(easeOf(node, 50)).toBeNull();
    const ops = opsOf(easeKeyframes(TARGET, [node], 0, 2, 1));
    expect(ops).toHaveLength(PLAIN);
    expect(ops.find(op => op.path === 'primaries.saturation')).toEqual({ op: 'splice_keys', grade: TARGET, id: 'n1', path: 'primaries.saturation', at: 0, remove: 1,
      keys: [key(0, 60, { bezier: { out: [EASE_SHARE, 0], in: [easeShare(1), 0] } })] });
    const eased = nodeOf(applied(gradeOf(node), ops), 'n1');
    expect(easeOf(eased, 0)).toEqual({ start: 2, end: 1 });
    expect(keyframesOf(eased).map(each => each.kind)).toEqual(['dynamic', 'dynamic']);
    // Set back to none, the stretches are lines again; the same eases again change nothing.
    expect(nodeOf(applied(gradeOf(eased), easeKeyframes(TARGET, [eased], 0, 0, 0)), 'n1')).toEqual(node);
    expect(easeKeyframes(TARGET, [eased], 0, 2, 1)).toBeNull();
    // A static keyframe, and the last one, take none.
    const held = withKeyframe(withKeyframe(corrector('n2'), 0, 'static'), 50, 'dynamic');
    expect(easeOf(held, 0)).toBeNull();
    expect(easeKeyframes(TARGET, [held], 0, 2, 2)).toBeNull();
    expect(easeKeyframes(TARGET, [node], 50, 2, 2)).toBeNull();
  });
});

describe('Auto-Keyframe', () => {
  const params = fullCorrector({ primaries: { saturation: { keys: [key(0, 50), key(100, 100)] } } });

  it('puts a dynamic keyframe on every setting at the frame of the change, where the node has none there', () => {
    const keyed = autoKeyed(params, 50)!;
    expect(settingsIn('corrector', keyed)).toHaveLength(PLAIN);
    expect(settingsIn('corrector', keyed).every(({ param }) => typeof param !== 'number' && param.keys.some(each => each.frame === 50 && each.interp === 'linear'))).toBe(true);
    expect(getAt(keyed, 'primaries.saturation')).toEqual({ keys: [key(0, 50), key(50, 75), key(100, 100)] });
    expect(getAt(keyed, 'primaries.contrast')).toEqual({ keys: [key(50, 1)] });
    // Asked again for the same corrector and frame, it is the same; it is what `addKeyframe` makes of the node.
    expect(autoKeyed(params, 50)).toBe(keyed);
    const node: GradeNode = { id: 'n1', kind: 'corrector', label: '', enabled: true, params };
    expect(nodeOf(applied(gradeOf(node), addKeyframe(TARGET, node, 50, 'dynamic')), 'n1').params).toEqual(keyed);
    // Where a setting has a key at the frame, the node has a keyframe there, and the change goes into it.
    expect(autoKeyed(params, 100)).toBeNull();
  });

  it('puts a number that was changed into the new key, and leaves a setting that was not changed as it is at the frame', () => {
    const keyed = autoKeyed(params, 50)!;
    const auto = { keyed, shown: params };
    // The control showed 50, the key at its left (p. 3384): given back, the setting keeps what it is at the frame, 75.
    expect(autoValue('primaries.saturation', 50, keyed, auto, 50)).toEqual({ keys: [key(0, 50), key(50, 75), key(100, 100)] });
    expect(autoValue('primaries.saturation', 60, keyed, auto, 50)).toEqual({ keys: [key(0, 50), key(50, 60), key(100, 100)] });
    expect(autoValue('primaries.contrast', 1.2, keyed, auto, 50)).toEqual({ keys: [key(50, 1.2)] });
    expect(autoValue('primaries.contrast', 1, keyed, auto, 50)).toEqual({ keys: [key(50, 1)] });
  });

  it('puts the points of a curve into the new keys number by number, and what is no curve as it is given', () => {
    const keyed = autoKeyed(params, 50)!;
    const auto = { keyed, shown: params };
    const at = (value: number): Param => ({ keys: [key(50, value)] });
    expect(autoWhole('curves.y.points', [[0, 0.1], [1, 1]], keyed, auto, 50)).toEqual([[at(0), at(0.1)], [at(1), at(1)]]);
    // A curve that gained a point is put whole, and so is a switch.
    const three = [[0, 0], [0.5, 0.6], [1, 1]];
    expect(autoWhole('curves.y.points', three, keyed, auto, 50)).toBe(three);
    expect(autoWhole('curves.ganged', false, keyed, auto, 50)).toBe(false);
  });
});

describe('the frames of the keys in the Keyframe Editor', () => {
  it('are those of the clock of the clip, through its direction and its speed', () => {
    // Backward: the frame 20 of the timeline shows the picture 49, and the playhead goes from right to left.
    const backward = clip('c1', 10, 60, { reverse: true });
    const span: KeySpan = { first: 10, last: 69, clock: keyClockOf(backward, RATE, media) };
    expect(keyRange(span)).toEqual([0, 59]);
    expect(keyFrameAt(span, 20)).toBe(49);
    expect(keyFrameAt(span, 20.7)).toBe(49);
    expect(timelineFrameOf(span, 49)).toBe(20);
    // A playhead outside the clip is at its nearest frame.
    expect(keyFrameAt(span, 0)).toBe(59);
    expect(keyFrameAt(span, 500)).toBe(0);
    // A keyframe added at the playhead of that clip is at the picture shown, not at the frame of the clip.
    const ops = opsOf(addKeyframes(TARGET, [corrector('n1')], keyFrameAt(span, 20), 'dynamic'));
    expect(ops.every(op => op.op === 'splice_keys' && op.keys[0].frame === 49)).toBe(true);
    // Twice as fast: every other picture is shown, and a key between two is at the frame that first shows a later one.
    const fast = clip('c1', 10, 30, { speed: { num: 2, den: 1 } });
    const quick: KeySpan = { first: 10, last: 39, clock: keyClockOf(fast, RATE, media) };
    expect(keyFrameAt(quick, 15)).toBe(10);
    expect(keyRange(quick)).toEqual([0, 58]);
    expect(timelineFrameOf(quick, 10)).toBe(15);
    expect(timelineFrameOf(quick, 11)).toBe(16);
    expect(timesOf(quick, [11, 10, 500, 10])).toEqual([15, 16]);
    // Half as fast: a picture is shown on two frames, and is at the first of them.
    const slow = clip('c1', 0, 40, { speed: { num: 1, den: 2 } });
    const long: KeySpan = { first: 0, last: 39, clock: keyClockOf(slow, RATE, media) };
    expect([0, 1, 2, 3].map(frame => keyFrameAt(long, frame))).toEqual([0, 0, 1, 1]);
    expect(timelineFrameOf(long, 1)).toBe(2);
    // The grade of a timeline counts its own frames.
    const whole: KeySpan = { first: 0, last: 299, clock: TIMELINE_CLOCK };
    expect([keyFrameAt(whole, 120), timelineFrameOf(whole, 120)]).toEqual([120, 120]);
    expect(keyRange(whole)).toEqual([0, 299]);
  });

  it('are stepped through in the order of the timeline', () => {
    expect(stepTime([10, 20, 30], 20, 1)).toBe(30);
    expect(stepTime([10, 20, 30], 20, -1)).toBe(10);
    expect(stepTime([10, 20, 30], 25, -1)).toBe(20);
    expect(stepTime([10], 10, 1)).toBeNull();
    expect(stepTime([10], 5, -1)).toBeNull();
    expect(stepTime([], 5, 1)).toBeNull();
  });

  it('are laid along the lanes, and found again from a place in them', () => {
    const range = [0, 100] as const;
    expect(xOfFrame(range, 480, 0)).toBe(LANE.pad);
    expect(xOfFrame(range, 480, 50)).toBe(240);
    expect(xOfFrame(range, 480, 100)).toBe(480 - LANE.pad);
    expect(frameAtX(range, 480, 240)).toBe(50);
    expect(frameAtX(range, 480, 241.9)).toBe(50);
    expect(frameAtX(range, 480, -50)).toBe(0);
    expect(frameAtX(range, 480, 5000)).toBe(100);
    // A clip of one frame shows it at the left.
    expect(xOfFrame([7, 7], 480, 7)).toBe(LANE.pad);
    expect(frameAtX([7, 7], 480, 300)).toBe(7);
    expect([rowTop(0), rowMiddle(0), rowMiddle(1), lanesHeight(3)]).toEqual([LANE.ruler, LANE.ruler + LANE.row / 2, LANE.ruler + LANE.row * 1.5, LANE.ruler + 3 * LANE.row]);
    expect([rowAt(LANE.ruler - 1, 3), rowAt(LANE.ruler, 3), rowAt(LANE.ruler + 3 * LANE.row - 1, 3), rowAt(LANE.ruler + 3 * LANE.row, 3)]).toEqual([null, 0, 2, null]);
    expect(rulerTicks(range, 480)).toEqual([0, 20, 40, 60, 80, 100]);
    expect(rulerTicks([7, 7], 480)).toEqual([7]);
  });

  it('are marked on each track, a mark taken by a pointer on its track within reach', () => {
    const one = withKeyframe(corrector('n1'), 50, 'dynamic');
    const two = withKeyframe(withKeyframe(corrector('n2'), 50, 'static'), 150, 'static');
    const marks = marksOf(tracksOf(gradeOf(one, two)), [0, 100], LANES_WIDTH);
    // The keyframe at 150 is of a picture that the clip does not show.
    expect(marks.map(mark => [mark.row, mark.node, mark.frame, mark.kind, mark.x, mark.y])).toEqual([
      [0, null, 50, 'dynamic', 240, rowMiddle(0)], [1, 'n1', 50, 'dynamic', 240, rowMiddle(1)], [2, 'n2', 50, 'static', 240, rowMiddle(2)],
    ]);
    expect(marks[0].mixed).toBe(true);
    expect(hitMark(marks, { x: 243, y: rowMiddle(1) + 3 })).toBe(marks[1]);
    expect(hitMark(marks, { x: 236, y: rowMiddle(2) })).toBe(marks[2]);
    expect(hitMark(marks, { x: 250, y: rowMiddle(1) })).toBeNull();
    expect(hitMark(marks, { x: 240, y: LANE.ruler - 4 })).toBeNull();
    expect(DIAMOND).toBe('M0,-5 L5,0 L0,5 L-5,0 Z');
  });

  it('draw the curve of the two eases of a dynamic keyframe', () => {
    const box = { width: 160, height: 90 };
    expect(easePath(0, 0, box)).toBe('M0,90 C0,90 160,0 160,0');
    expect(easePath(2, 2, box)).toBe('M0,90 C53.33,90 106.67,0 160,0');
    expect(easePath(EASE_MOST, 0, box)).toBe('M0,90 C106.67,90 160,0 160,0');
  });
});
