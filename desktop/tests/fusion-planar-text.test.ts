/**
 * The Planar Tracker and the Text+ of a composition, worked out apart from the page (PROTOCOL.md,
 * "The Planar Tracker", "The region of a Planar Tracker", and the Text+ of the engine): the copy of
 * the project that mirrors the engine's examples, the homographies of a path as the engine reads
 * them, the operations of the region, the corners and the path, the controls of both on the picture
 * of the viewer, and the settings of a Text+ and its parts.
 *
 * The picture is that of the worked example of the protocol: a composition of 1440 by 1080 in a
 * timeline of 1920 by 1080, which lies at 240, 0.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  applyOp, type Composition, type FusionHomography, type FusionLink, type FusionNode, type FusionPicture, type FusionPlanarTracker, type FusionTextPlus, type Op,
  type Project,
} from '../src/studio/engine';
import {
  ADDABLE, ADDABLE_EFFECTS, addNode, newBorder, newFollower, newNode, newOutline, newPlanarTracker, newTextPlus, removeNodes,
} from '../src/studio/fusion/edits';
import { handedOn, hasMaskInput, hasOutput, INPUTS, pictureInputs } from '../src/studio/fusion/graph';
import {
  controlsOf, draggedCorner, draggedRegion, draggedValues, edgeAt, handleName, hasControls, overlayOf, regionWithPoint,
} from '../src/studio/fusion/onscreen';
import {
  applyTo, boxCorners, clearTrack, cornersOp, fitHomography, homographyAt, IDENTITY, insertRegionPoint, inverse, matrixOf, MOST_REGION_POINTS, motionAt,
  pathFrames, regionOp, removeRegionPoint, restoredPlanar, setPlanar, type Matrix3,
} from '../src/studio/fusion/planar';
import { FUSION_SETTINGS, hasKeys, paramOf, putValue, settingOf, settingsOf, textPartsOf } from '../src/studio/fusion/settings';
import { limitedText, MOST_TEXT, setPart, setText } from '../src/studio/fusion/textPlus';
import { clip, media, timeline } from './timeline-fixture';

const AT = { timeline: 't1', clip: 'c1' };
const PICTURE: FusionPicture = { x: 240, y: 0, width: 1440, height: 1080 };
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
/** The homography that moves nothing, one that shifts `x` across and `y` up, and one of a little perspective, a turn and a shift. */
const STILL: FusionHomography = [1, 0, 0, 0, 1, 0, 0, 0];
const shifted = (x: number, y: number): FusionHomography => [1, 0, x, 0, 1, y, 0, 0];
const TILTED: FusionHomography = [1.02, -0.03, 0.05, 0.02, 0.97, -0.04, 0.08, -0.06];
/** A Planar Tracker, the node `n6`, with what else of it is given. */
const planar = (given: Partial<FusionPlanarTracker> = {}, id = 'n6'): FusionNode => ({ ...newNode('planar_tracker', null, id), params: { ...newPlanarTracker(), ...given } } as FusionNode);
/** A Text+, the node `n7`, with what else of it is given. */
const text = (given: Partial<FusionTextPlus> = {}, id = 'n7'): FusionNode => ({ ...newNode('text_plus', null, id), params: { ...newTextPlus(), ...given } } as FusionNode);
/** MediaIn1 through the Transform n3 into the background of the Merge n4, the Background n5 its foreground, and the Merge into MediaOut1. */
const comp = (): Composition => ({
  nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), newNode('transform', null, 'n3'), newNode('merge', null, 'n4'),
    newNode('background', null, 'n5')],
  links: [link('MediaIn1', 'n3'), link('n3', 'n4', 'background'), link('n5', 'n4', 'foreground'), link('n4', 'MediaOut1')],
});
/** A composition of a MediaIn into a MediaOut and one more node, which is its third. */
const withNode = (node: FusionNode): Composition => ({ nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), node], links: [link('MediaIn1', 'MediaOut1')] });
/** A composition as the copy leaves it once it has applied operations to the one clip that has it. */
const applied = (fusion: Composition, ops: Op[]): Composition => {
  const project: Project = { schema: 5, next_id: 20, bins: [], groups: [], media: [media], timelines: [timeline([clip('c1', 0, 100, { fusion })])] };
  return (ops.reduce(applyOp, project).timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!;
};
/** Two matrices alike to many decimals. */
const near = (a: Matrix3 | null, b: Matrix3) => {
  expect(a).not.toBeNull();
  a!.forEach((row, i) => row.forEach((value, j) => expect(value, `${i},${j}`).toBeCloseTo(b[i][j], 9)));
};
/** Two places alike to many decimals. */
const nearPlace = (a: [number, number] | null, b: [number, number]) => {
  expect(a).not.toBeNull();
  expect(a![0]).toBeCloseTo(b[0], 9);
  expect(a![1]).toBeCloseTo(b[1], 9);
};

interface Example { name: string; before: Project; after: Project; done: Op; inverse: Op }
const examples: Example[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/ops-examples.json', import.meta.url), 'utf8'));

describe('a Planar Tracker in the copy of the project', () => {
  it('is added and set as the engine does in its examples, a frame of its path given null taken away', () => {
    const ours = examples.filter(each => each.name.includes('planar tracker'));
    expect(ours.length).toBeGreaterThanOrEqual(2);
    for (const example of ours) {
      const before = { ...structuredClone(example.before), next_id: example.after.next_id };
      const changed = applyOp(before, example.done);
      expect(changed, example.name).toStrictEqual(example.after);
      expect(applyOp(changed, example.inverse), example.name).toStrictEqual(before);
    }
    // The inverse of a write of frames of the path gives the frames that it put `null`, which takes them away.
    const frames = ours.find(each => each.name.startsWith('set_fusion_node of frames of the path'))!;
    const inverse = frames.inverse as Extract<Op, { op: 'set_fusion_node' }>;
    expect(Object.values((inverse.params as { path: Record<string, unknown> }).path)).toContain(null);
  });

  it('merges a patch of its path frame by frame: a frame given null is not there, the others kept or put in place', () => {
    const node = planar({ path: { 0: STILL, 12: shifted(0.1, 0), 24: shifted(0.2, 0) } });
    const after = applied(withNode(node), [{ op: 'set_fusion_node', ...AT, id: 'n6', params: { path: { 12: null, 13: shifted(0.15, 0), 24: shifted(0.25, 0) } } }]);
    expect((after.nodes[2].params as FusionPlanarTracker).path).toStrictEqual({ 0: STILL, 13: shifted(0.15, 0), 24: shifted(0.25, 0) });
    // Its region is a list, put in place of the list whole; its corners an object, merged.
    const region = [{ x: 0.1, y: 0.1 }, { x: 0.9, y: 0.1 }, { x: 0.5, y: 0.9 }];
    const set = applied(withNode(node), [{ op: 'set_fusion_node', ...AT, id: 'n6', params: { region, corners: { tl: { x: 0, y: 1 } } } }]);
    const params = set.nodes[2].params as FusionPlanarTracker;
    expect([params.region, params.corners]).toStrictEqual([region, { ...newPlanarTracker().corners, tl: { x: 0, y: 1 } }]);
    // Clear Track gives every frame null: the path is empty then.
    const cleared = applied(withNode(node), clearTrack(AT, node));
    expect((cleared.nodes[2].params as FusionPlanarTracker).path).toStrictEqual({});
  });
});

describe('a Planar Tracker in the node editor', () => {
  it('is made with every setting as the engine makes a new one, has no setting that takes keys, and has its picture, its foreground and its mask', () => {
    expect(newNode('planar_tracker', null, 'p1')).toStrictEqual({
      id: 'p1', label: '', pass_through: false, pos: null, kind: 'planar_tracker',
      params: {
        operation: 'none', motion: 'perspective', reference: 0,
        region: [{ x: 0.25, y: 0.25 }, { x: 0.75, y: 0.25 }, { x: 0.75, y: 0.75 }, { x: 0.25, y: 0.75 }],
        corners: { tl: { x: 0.25, y: 0.75 }, tr: { x: 0.75, y: 0.75 }, br: { x: 0.75, y: 0.25 }, bl: { x: 0.25, y: 0.25 } },
        path: {},
      },
    });
    // The corners of a new one are those of the box about its region.
    expect(boxCorners(newPlanarTracker().region)).toStrictEqual(newPlanarTracker().corners);
    expect([FUSION_SETTINGS.planar_tracker, settingsOf(planar()), hasKeys(planar({ path: { 0: STILL } }))]).toEqual([[], [], false]);
    expect(INPUTS.planar_tracker).toEqual(['input', 'foreground', 'mask']);
    expect([pictureInputs('planar_tracker'), hasOutput('planar_tracker'), hasMaskInput('planar_tracker'), handedOn('planar_tracker')])
      .toEqual([['input', 'foreground'], true, true, 'input']);
  });

  it('is added among the effects after the node chosen on its picture, and is joined across by its input alone where it is deleted', () => {
    expect(ADDABLE_EFFECTS.at(-1)).toBe('planar_tracker');
    expect(addNode(AT, comp(), 'planar_tracker', null, { x: 10.4, y: 20.6 })).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('planar_tracker', { x: 10, y: 21 }) }]);
    expect(addNode(AT, comp(), 'planar_tracker', 'n3', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('planar_tracker'), links: [link('n3', '', 'input'), link('', 'n4', 'background')],
    }]);
    // Its foreground, which its Corner Pin lays on the plane, is no picture that it hands on.
    const made = comp();
    const pinned: Composition = {
      nodes: [...made.nodes, planar()],
      links: [link('MediaIn1', 'n3'), link('n3', 'n6'), link('n5', 'n6', 'foreground'), link('n6', 'n4', 'background'), link('n4', 'MediaOut1')],
    };
    expect(removeNodes(AT, pinned, ['n6'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n6', links: [link('n3', 'n4', 'background')] }]);
  });
});

describe('the path of a Planar Tracker', () => {
  it('is made of homographies of eight numbers, the last of nine being 1, which act on places from the lower left corner', () => {
    expect(matrixOf(TILTED)).toEqual([[1.02, -0.03, 0.05], [0.02, 0.97, -0.04], [0.08, -0.06, 1]]);
    expect(applyTo(matrixOf(shifted(0.25, 0.125)), [0.5, 0.5])).toEqual([0.75, 0.625]);
    expect(applyTo(IDENTITY, [0.3, 0.4])).toEqual([0.3, 0.4]);
    // A place that a homography sends to no place.
    expect(applyTo([[1, 0, 0], [0, 1, 0], [1, 0, 0]], [0, 0.5])).toBeNull();
    // The frames in their order, whatever order they are written in.
    expect(pathFrames({ 24: STILL, 3: TILTED, 12: shifted(0, 0) }).map(([frame]) => frame)).toEqual([3, 12, 24]);
    expect(pathFrames(undefined)).toEqual([]);
  });

  it('is fitted from four places, and undone', () => {
    const box: [number, number][] = [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]];
    const through = (m: Matrix3) => box.map(place => applyTo(m, place)!);
    near(fitHomography(box, through(matrixOf(TILTED))), matrixOf(TILTED));
    near(fitHomography(box, through(matrixOf(shifted(0.1, -0.2)))), matrixOf(shifted(0.1, -0.2)));
    // Three of the places on one line make no quadrilateral.
    expect(fitHomography([[0, 0], [1, 0], [2, 0], [0, 1]], [[0, 0], [1, 0], [2, 0], [0, 1]])).toBeNull();
    // The inverse takes each place back where it was; a matrix that flattens the plane has none.
    const back = inverse(matrixOf(TILTED))!;
    for (const place of box) nearPlace(applyTo(back, applyTo(matrixOf(TILTED), place)!), place);
    expect(inverse([[0, 0, 0], [0, 0, 0], [0, 0, 1]])).toBeNull();
  });

  it('is read exactly at its frames, between them by the box about the region, and held beyond them, as the engine reads it', () => {
    const moving = { region: newPlanarTracker().region, path: { 10: STILL, 20: shifted(0.2, -0.1), 22: TILTED } };
    expect(homographyAt(moving, 10)).toEqual(IDENTITY);
    expect(homographyAt(moving, 20)).toEqual(matrixOf(shifted(0.2, -0.1)));
    expect(homographyAt(moving, 22)).toEqual(matrixOf(TILTED));
    // Half way between two shifts, the shift half way between them.
    nearPlace(applyTo(homographyAt(moving, 15), [0.5, 0.5]), [0.6, 0.45]);
    // Before the first frame that of the first, after the last that of the last; and none where there is no path or no frame.
    expect([homographyAt(moving, 9), homographyAt(moving, -1000), homographyAt(moving, 30)]).toEqual([IDENTITY, IDENTITY, matrixOf(TILTED)]);
    expect([homographyAt({ ...moving, path: {} }, 7), homographyAt(moving, Number.NaN)]).toEqual([IDENTITY, IDENTITY]);
  });

  it('moves the plane from where it is at the reference frame, at which nothing moves', () => {
    const moving: FusionPlanarTracker = { ...newPlanarTracker(), path: { 0: STILL, 10: shifted(0.25, 0), 20: shifted(0.5, 0.125) } };
    expect(motionAt(moving, 20)).toEqual(matrixOf(shifted(0.5, 0.125)));
    const later = { ...moving, reference: 10 };
    expect(motionAt(later, 10)).toEqual(IDENTITY);
    nearPlace(applyTo(motionAt(later, 20)!, [0.5, 0.5]), [0.75, 0.625]);
    nearPlace(applyTo(motionAt(later, 0)!, [0.5, 0.5]), [0.25, 0.5]);
    // No path, no motion; a homography at the reference frame that flattens the plane cannot be undone.
    expect(motionAt(newPlanarTracker(), 12)).toEqual(IDENTITY);
    const flat = { ...newPlanarTracker(), reference: 5, path: { 0: STILL, 5: [0, 0, 0, 0, 0, 0, 0, 0] as FusionHomography } };
    expect([motionAt(flat, 5), motionAt(flat, 0)]).toEqual([IDENTITY, null]);
  });
});

describe('the settings of a Planar Tracker', () => {
  const region = newPlanarTracker().region;
  const corners = newPlanarTracker().corners;

  it('are set by a patch of those that change, none taking keys', () => {
    const node = planar();
    expect(setPlanar(AT, node, { operation: 'steady', motion: 'perspective', reference: 0 })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n6', params: { operation: 'steady' } }]);
    expect(setPlanar(AT, node, { corners: { ...corners, tl: { x: 0.2, y: 0.75 } } })).toEqual([{
      op: 'set_fusion_node', ...AT, id: 'n6', params: { corners: { ...corners, tl: { x: 0.2, y: 0.75 } } },
    }]);
    expect([setPlanar(AT, node, { corners, reference: 0 }), setPlanar(AT, newNode('blur', null, 'b1'), { operation: 'steady' })]).toEqual([[], []]);
  });

  it('have their region set whole, with the corners of the box about it while the Corner Pin is not what the node does', () => {
    const grown = [...region.slice(0, 2), { x: 0.9, y: 0.5 }, ...region.slice(2)];
    expect(insertRegionPoint(AT, planar(), 1, { x: 0.9, y: 0.5 })).toEqual([{
      op: 'set_fusion_node', ...AT, id: 'n6',
      params: { region: grown, corners: { tl: { x: 0.25, y: 0.75 }, tr: { x: 0.9, y: 0.75 }, br: { x: 0.9, y: 0.25 }, bl: { x: 0.25, y: 0.25 } } },
    }]);
    // With the Corner Pin the corners are the user's: the region alone.
    const pinned = planar({ operation: 'corner_pin' });
    expect(insertRegionPoint(AT, pinned, 1, { x: 0.9, y: 0.5 })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n6', params: { region: grown } }]);
    expect(regionOp(AT, pinned, region)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n6', params: { region } });
    // Each place that was there is the object that it is.
    const own = planar();
    expect(((insertRegionPoint(AT, own, 3, { x: 0, y: 0 })[0] as Extract<Op, { op: 'set_fusion_node' }>).params as { region: unknown[] }).region[0])
      .toBe((own.params as FusionPlanarTracker).region[0]);
    // At most 64 places, and three at the least.
    const full = planar({ region: Array.from({ length: MOST_REGION_POINTS }, (_, index) => ({ x: index / 100, y: 0.5 })) });
    expect([insertRegionPoint(AT, full, 0, { x: 0, y: 0 }), insertRegionPoint(AT, planar(), 4, { x: 0, y: 0 })]).toEqual([[], []]);
    expect(removeRegionPoint(AT, planar(), 0)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n6', params: { region: region.slice(1), corners } }]);
    expect([removeRegionPoint(AT, planar({ region: region.slice(1) }), 0), removeRegionPoint(AT, planar(), 4)]).toEqual([[], []]);
    // What a drag that is taken back sends, and the corners set whole.
    expect(restoredPlanar(AT, planar())).toEqual({ op: 'set_fusion_node', ...AT, id: 'n6', params: { region, corners } });
    expect(cornersOp(AT, planar(), corners)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n6', params: { corners } });
    expect([restoredPlanar(AT, newNode('blur', null, 'b1')), cornersOp(AT, newNode('blur', null, 'b1'), corners)]).toEqual([null, null]);
  });

  it('have their path cleared by one patch that gives each of its frames null', () => {
    expect(clearTrack(AT, planar({ path: { 0: STILL, 12: shifted(0.1, 0) } }))).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n6', params: { path: { 0: null, 12: null } } }]);
    expect([clearTrack(AT, planar()), clearTrack(AT, text())]).toEqual([[], []]);
  });
});

describe('the controls of a Planar Tracker on the picture', () => {
  const REGION = [[600, 810], [1320, 810], [1320, 270], [600, 270]];

  it('are its region, a handle at each of its places, and with the Corner Pin its four corners', () => {
    expect([hasControls('planar_tracker'), hasControls('text_plus')]).toEqual([true, true]);
    const controls = controlsOf(planar(), 0, PICTURE)!;
    expect(controls.lines).toEqual([[...REGION, REGION[0]]]);
    expect(controls.handles).toEqual(REGION.map((at, index) => ({ kind: 'region', index, at })));
    expect(controls.edges).toEqual(REGION);
    expect(overlayOf(controls, [1920, 1080])).toEqual({ canvas: [1920, 1080], lines: [[...REGION, REGION[0]]], dots: [{ kind: 'key', at: REGION }] });
    expect(controls.handles.map(handleName)).toEqual(['region:0', 'region:1', 'region:2', 'region:3']);
    // The corners of the Corner Pin from the upper left one round, after the places of the region: a corner on a place is the one pressed.
    const pinned = controlsOf(planar({ operation: 'corner_pin' }), 0, PICTURE)!;
    expect(pinned.lines[1]).toEqual([[600, 270], [1320, 270], [1320, 810], [600, 810], [600, 270]]);
    expect(pinned.handles.slice(4)).toEqual([[600, 270], [1320, 270], [1320, 810], [600, 810]].map((at, index) => ({ kind: 'corner', index, at })));
    expect(handleName(pinned.handles[4])).toBe('corner:0');
  });

  it('are where the motion of the path puts them at the playhead', () => {
    const moving = planar({ path: { 0: STILL, 10: shifted(0.1, 0) } });
    // A tenth of the width at frame 10 is 144 pixels; at frame 5, half way, 72.
    expect(controlsOf(moving, 10, PICTURE)!.lines[0]).toEqual([[744, 810], [1464, 810], [1464, 270], [744, 270], [744, 810]]);
    expect(controlsOf(moving, 5, PICTURE)!.handles[0].at).toEqual([672, 810]);
    // At the reference frame the region is where it was drawn.
    expect(controlsOf(moving, 0, PICTURE)!.edges).toEqual(REGION);
  });

  it('find the side of the region that a press is on, and where on it', () => {
    const controls = controlsOf(planar(), 0, PICTURE)!;
    expect(edgeAt(controls, [960, 812], 16)).toEqual({ after: 0, at: [960, 810] });
    expect(edgeAt(controls, [1322, 540], 16)).toEqual({ after: 1, at: [1320, 540] });
    expect(edgeAt(controls, [598, 540], 16)).toEqual({ after: 3, at: [600, 540] });
    expect([edgeAt(controls, [960, 540], 16), edgeAt(controls, [960, 830], 16), edgeAt(controlsOf(text(), 0, PICTURE)!, [960, 540], 16)]).toEqual([null, null, null]);
  });

  it('move a place or a corner with the pointer, taken back through the motion to the reference frame', () => {
    const region = newPlanarTracker().region;
    // 144 pixels right and 108 up: a tenth of the width and of the height.
    expect(draggedRegion(planar(), 2, 0, PICTURE, 144, -108)).toEqual([region[0], region[1], { x: 0.85, y: 0.85 }, region[3]]);
    const own = planar();
    expect(draggedRegion(own, 2, 0, PICTURE, 144, -108)![0]).toBe((own.params as FusionPlanarTracker).region[0]);
    // Kept within the range of a place.
    expect(draggedRegion(planar(), 0, 0, PICTURE, 1440 * 40, 0)![0]).toEqual({ x: 16.5, y: 0.25 });
    // Where the path moves the plane a tenth of the width at frame 10, the place goes with the pointer from where it is drawn there.
    const moving = planar({ path: { 0: STILL, 10: shifted(0.1, 0) } });
    expect(draggedRegion(moving, 0, 10, PICTURE, 0, 0)![0]).toEqual({ x: 0.25, y: 0.25 });
    expect(draggedRegion(moving, 0, 10, PICTURE, 144, 0)![0]).toEqual({ x: 0.35, y: 0.25 });
    // A press on a side puts a place there: the place at the reference frame that is drawn where it was pressed.
    expect(regionWithPoint(planar(), 0, 0, PICTURE, [960, 810])).toEqual([region[0], { x: 0.5, y: 0.25 }, ...region.slice(1)]);
    expect(regionWithPoint(moving, 0, 10, PICTURE, [1104, 810])![1]).toEqual({ x: 0.5, y: 0.25 });
    // A corner of the Corner Pin.
    const pinned = planar({ operation: 'corner_pin' });
    expect(draggedCorner(pinned, 0, 0, PICTURE, -144, 0)).toEqual({ ...newPlanarTracker().corners, tl: { x: 0.15, y: 0.75 } });
    // Nothing of a place, a corner or a node that is not there, and nothing for the settings of a node that take keys.
    expect([draggedRegion(planar(), 4, 0, PICTURE, 1, 1), draggedCorner(pinned, 4, 0, PICTURE, 1, 1), draggedRegion(text(), 0, 0, PICTURE, 1, 1)]).toEqual([null, null, null]);
    expect(draggedValues(planar(), { kind: 'region', index: 0 }, 0, PICTURE, 10, 10)).toEqual([]);
  });
});

describe('a Text+', () => {
  /** A Text+ with all of its parts. */
  const whole = () => text({ outline: newOutline(), border: newBorder(), follower: newFollower() });
  const paths = (node: FusionNode) => settingsOf(node).map(setting => setting.path);
  const PLAIN = [
    'size', 'center.x', 'center.y', 'line_spacing', 'tracking', 'angle', 'write_on.start', 'write_on.end', 'transform.offset.x', 'transform.offset.y',
    'transform.angle', 'transform.scale', 'fill.color.r', 'fill.color.g', 'fill.color.b', 'fill.color.a',
  ];

  it('is made with every setting as the engine makes a new one, makes its picture of nothing, and has its effect mask', () => {
    expect(newNode('text_plus', null, 'x1')).toStrictEqual({
      id: 'x1', label: '', pass_through: false, pos: null, kind: 'text_plus',
      params: {
        text: '', font: '', size: 0.08, center: { x: 0.5, y: 0.5 }, justify: 'center', vertical: 'center', line_spacing: 1, tracking: 0, angle: 0,
        write_on: { start: 0, end: 1 }, transform: { select: 'characters', offset: { x: 0, y: 0 }, angle: 0, scale: 1 }, fill: { color: { r: 1, g: 1, b: 1, a: 1 } },
      },
    });
    expect([newOutline(), newBorder(), newFollower()]).toStrictEqual([
      { color: { r: 0, g: 0, b: 0, a: 1 }, thickness: 0.02 },
      { color: { r: 0, g: 0, b: 0, a: 0.6 }, extend: 0.02, round: 0 },
      { order: 'left_to_right', delay: 2, delay_type: 'each', opacity: 1, offset: { x: 0, y: 0 }, angle: 0, scale: 1 },
    ]);
    expect([INPUTS.text_plus, pictureInputs('text_plus'), hasOutput('text_plus'), hasMaskInput('text_plus'), handedOn('text_plus')])
      .toEqual([['mask'], [], true, true, undefined]);
    // Added beside the Background, and with no links, whatever is chosen.
    expect(ADDABLE.indexOf('text_plus')).toBe(ADDABLE.indexOf('background') + 1);
    expect(addNode(AT, comp(), 'text_plus', 'n3', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('text_plus') }]);
  });

  it('has the settings that take keys of the catalog of the engine, with its defaults and ranges', () => {
    const catalog: Record<string, { path: string; default: number; low: number; high: number; page: string }[]> = JSON.parse(
      readFileSync(new URL('../../engine/crates/model/tests/data/catalog.json', import.meta.url), 'utf8'));
    const entries = catalog.fusion_text_plus;
    const byPath = <T extends { path: string }>(list: readonly T[]) => [...list].sort((a, b) => a.path.localeCompare(b.path));
    expect(byPath(FUSION_SETTINGS.text_plus).map(setting => ({ path: setting.path, default: setting.bounds.neutral, low: setting.bounds.low, high: setting.bounds.high })))
      .toEqual(byPath(entries).map(entry => ({ path: entry.path, default: entry.default, low: entry.low, high: entry.high })));
    expect(entries.every(entry => entry.page === 'fusion')).toBe(true);
    // A Text+ with all its parts has every setting at its default.
    for (const setting of settingsOf(whole())) expect(paramOf(whole(), setting), setting.path).toBe(setting.bounds.neutral);
    expect(paths(whole())).toEqual(FUSION_SETTINGS.text_plus.map(setting => setting.path));
  });

  it('has the settings of a part that takes keys only where it has the part, which the engine refuses a key of otherwise', () => {
    expect(paths(text())).toEqual(PLAIN);
    expect(textPartsOf(text())).toEqual([]);
    const outlined = text({ outline: newOutline() });
    expect(textPartsOf(outlined)).toEqual(['outline']);
    expect(paths(outlined)).toEqual([...PLAIN, 'outline.thickness', 'outline.color.r', 'outline.color.g', 'outline.color.b', 'outline.color.a']);
    expect(paths(text({ follower: newFollower() })).slice(PLAIN.length)).toEqual(['follower.opacity', 'follower.offset.x', 'follower.offset.y', 'follower.angle', 'follower.scale']);
    expect([settingOf(text(), 'outline.thickness'), settingOf(text(), 'border.extend'), settingOf(text(), 'follower.opacity')]).toEqual([undefined, undefined, undefined]);
    expect(settingOf(outlined, 'outline.thickness')!.bounds).toEqual({ neutral: 0.02, low: 0, high: 0.5 });
    // The same list each time, so that what is drawn of it is not drawn again.
    expect(settingsOf(text())).toBe(settingsOf(text()));
    // A number of a part set as every number is, by a patch of the node; and a key on a setting of it is a key of the node.
    expect(putValue(AT, outlined, settingOf(outlined, 'outline.thickness')!, 0.05, 0)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { outline: { thickness: 0.05 } } }]);
    expect(hasKeys(text({ size: { keys: [{ frame: 0, value: 0.1, interp: 'linear' }] } }))).toBe(true);
    expect(hasKeys(outlined)).toBe(false);
  });

  it('has its parts turned on as the engine makes them and off by null, which takes them away in the copy', () => {
    const node = text();
    expect(setPart(AT, node, 'outline', true)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { outline: newOutline() } }]);
    expect(setPart(AT, node, 'border', true)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { border: newBorder() } }]);
    expect(setPart(AT, node, 'follower', true)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { follower: newFollower() } }]);
    expect([setPart(AT, node, 'outline', false), setPart(AT, text({ outline: newOutline() }), 'outline', true), setPart(AT, planar(), 'outline', true)]).toEqual([[], [], []]);
    const on = applied(withNode(node), setPart(AT, node, 'border', true)).nodes[2];
    expect(on.params).toStrictEqual({ ...newTextPlus(), border: newBorder() });
    const off = setPart(AT, on, 'border', false);
    expect(off).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { border: null } }]);
    expect(applied(withNode(on), off).nodes[2].params).toStrictEqual(newTextPlus());
  });

  it('has what takes no keys set by a patch that changes it, and never half a part that it has not', () => {
    const node = text();
    expect(setText(AT, node, { text: 'Hello\nWorld' })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { text: 'Hello\nWorld' } }]);
    expect(setText(AT, node, { transform: { select: 'words' } })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { transform: { select: 'words' } } }]);
    expect([setText(AT, node, { justify: 'center' }), setText(AT, node, { follower: { order: 'inside_out' } }), setText(AT, planar(), { text: 'x' })]).toEqual([[], [], []]);
    expect(setText(AT, text({ follower: newFollower() }), { follower: { delay: 5 } })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { follower: { delay: 5 } } }]);
    // Its text is 4096 characters at the most, a character that is written with two units of UTF-16 one of them.
    expect(limitedText('a'.repeat(5000))).toHaveLength(MOST_TEXT);
    expect([...limitedText('😀'.repeat(MOST_TEXT + 1))]).toHaveLength(MOST_TEXT);
    expect(limitedText('short')).toBe('short');
  });

  it('has the handle of its centre on the picture, dragged as the centre of a Transform is', () => {
    const placed = text({ center: { x: 0.25, y: 0.75 } });
    expect(controlsOf(placed, 0, PICTURE)).toEqual({ lines: [], handles: [{ kind: 'center', at: [600, 270] }] });
    expect(draggedValues(text(), { kind: 'center' }, 0, PICTURE, 144, -108).map(({ setting, value }) => [setting.path, value])).toEqual([['center.x', 0.6], ['center.y', 0.6]]);
  });
});
