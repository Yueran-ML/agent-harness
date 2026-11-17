/**
 * The on-screen controls of a node of a composition, worked out apart from the page
 * (`src/studio/fusion/onscreen`; PROTOCOL.md, "The Fusion view"): where a place of a composition is
 * in the frame drawn, the outline and the handles of each kind of node, what the engine is asked to
 * draw, the handle that is pressed, and the values that a drag gives the settings of a handle; and
 * the keys that have the viewer show a node.
 *
 * The picture is that of the worked example of the protocol: a composition of 1440 by 1080 in a
 * timeline of 1920 by 1080, which lies at 240, 0. A width of the picture is 1440 pixels and a height
 * 1080.
 */
import { describe, expect, it } from 'vitest';
import type { Key } from '../src/app/keys';
import type {
  FusionEllipse, FusionMerge, FusionNode, FusionPicture, FusionPoint, FusionPolygon, FusionRectangle, FusionTrackerPattern, FusionTransform, OverlayPoint, Param,
  ViewPointer,
} from '../src/studio/engine';
import {
  fusionKey, newEllipse, newMerge, newNode, newPattern, newPolygon, newRectangle, newTracker, newTransform, startPoints, viewedAfter,
} from '../src/studio/fusion/edits';
import {
  controlsOf, draggedPattern, draggedValues, ELLIPSE_STEPS, fromPixels, handleName, hasControls, hitHandle, overlayOf, reachOf, toPixels, turnOf, type Handle,
} from '../src/studio/fusion/onscreen';
import { MOST_PATH_POINTS, pathSamples, placeOnPath, thinned } from '../src/studio/fusion/tracker';

const PICTURE: FusionPicture = { x: 240, y: 0, width: 1440, height: 1080 };
const rectangle = (params: Partial<FusionRectangle> = {}): FusionNode => ({ ...newNode('rectangle', null, 'n6'), params: { ...newRectangle(), ...params } } as FusionNode);
const ellipse = (params: Partial<FusionEllipse> = {}): FusionNode => ({ ...newNode('ellipse', null, 'n8'), params: { ...newEllipse(), ...params } } as FusionNode);
const polygon = (params: Partial<FusionPolygon> = {}): FusionNode => ({ ...newNode('polygon', null, 'n7'), params: { ...newPolygon(), points: startPoints(), ...params } } as FusionNode);
const merge = (params: Partial<FusionMerge> = {}): FusionNode => ({ ...newNode('merge', null, 'n4'), params: { ...newMerge(), ...params } } as FusionNode);
const transform = (params: Partial<FusionTransform> = {}): FusionNode => ({ ...newNode('transform', null, 'n3'), params: { ...newTransform(), ...params } } as FusionNode);
const keys = (...given: [frame: number, value: number][]): Param => ({ keys: given.map(([frame, value]) => ({ frame, value, interp: 'linear' as const })) });
const handles = (node: FusionNode, keyFrame = 0) => controlsOf(node, keyFrame, PICTURE)!.handles;
const lines = (node: FusionNode, keyFrame = 0) => controlsOf(node, keyFrame, PICTURE)!.lines;
/** The values that a drag of a handle gives, each with the path of its setting. */
const dragged = (node: FusionNode, handle: Pick<Handle, 'kind' | 'index'>, dx: number, dy: number, keyFrame = 0) => draggedValues(node, handle, keyFrame, PICTURE, dx, dy)
  .map(({ setting, value }) => [setting.path, value]);
const key = (given: Partial<Key>): Key => ({ key: '', shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...given });

describe('a place of a composition in the frame drawn', () => {
  it('is its share of the width from the left of the picture and of the height from its bottom, as the protocol works it out', () => {
    expect(toPixels(PICTURE, 0.5, 0.5)).toEqual([960, 540]);
    expect(toPixels(PICTURE, 0.25, 0.75)).toEqual([600, 270]);
    expect(toPixels(PICTURE, 0, 0)).toEqual([240, 1080]);
    expect(toPixels(PICTURE, 1, 1)).toEqual([1680, 0]);
    // And back.
    expect(fromPixels(PICTURE, 960, 540)).toEqual([0.5, 0.5]);
    expect(fromPixels(PICTURE, 600, 270)).toEqual([0.25, 0.75]);
    expect(fromPixels(PICTURE, 240, 1080)).toEqual([0, 0]);
  });

  it('turns by whole quarters exactly', () => {
    expect([turnOf(0), turnOf(90), turnOf(180), turnOf(270), turnOf(360), turnOf(-90), turnOf(450)]).toEqual([[1, 0], [0, 1], [-1, 0], [0, -1], [1, 0], [0, -1], [0, 1]]);
    const [cosine, sine] = turnOf(30);
    expect(cosine).toBeCloseTo(Math.sqrt(3) / 2, 12);
    expect(sine).toBeCloseTo(0.5, 12);
  });
});

describe('the controls of a node on the picture', () => {
  it('are those of the masks, of a Merge and of a Transform, and of no other kind', () => {
    expect(['rectangle', 'ellipse', 'polygon', 'merge', 'transform'].map(kind => hasControls(kind as FusionNode['kind']))).toEqual([true, true, true, true, true]);
    expect(['media_in', 'media_out', 'background', 'blur', 'mosaic', 'delta_keyer'].map(kind => hasControls(kind as FusionNode['kind']))).toEqual(Array.from({ length: 6 }, () => false));
    for (const kind of ['media_in', 'media_out', 'background', 'blur', 'mosaic', 'delta_keyer'] as const) expect(controlsOf(newNode(kind, null, 'n1'), 0, PICTURE), kind).toBeNull();
  });

  it('are the outline of a Rectangle, its width in widths and its height in heights, and the handles of its centre, its right side and its upper side', () => {
    // A new Rectangle: in the middle, half the picture wide and high: 720 by 540 pixels about 960, 540.
    expect(lines(rectangle())).toEqual([[[600, 810], [1320, 810], [1320, 270], [600, 270], [600, 810]]]);
    expect(handles(rectangle())).toEqual([{ kind: 'center', at: [960, 540] }, { kind: 'width', at: [1320, 540] }, { kind: 'height', at: [960, 270] }]);
    // Elsewhere, and a quarter of the picture high: 720 by 270 about 600, 270.
    const moved = rectangle({ center: { x: 0.25, y: 0.75 }, height: 0.25 });
    expect(lines(moved)).toEqual([[[240, 405], [960, 405], [960, 135], [240, 135], [240, 405]]]);
    expect(handles(moved).map(each => each.at)).toEqual([[600, 270], [960, 270], [600, 135]]);
    // Its corners are drawn straight, whatever its Corner Radius.
    expect(lines(rectangle({ corner_radius: 1 }))).toEqual(lines(rectangle()));
  });

  it('turn a Rectangle counter-clockwise as it is seen, about its centre', () => {
    // A quarter turn: what was its right side is its upper side, 360 above the centre, and what was its upper side is at its left.
    const quarter = rectangle({ angle: 90 });
    expect(lines(quarter)).toEqual([[[1230, 900], [1230, 180], [690, 180], [690, 900], [1230, 900]]]);
    expect(handles(quarter).map(each => each.at)).toEqual([[960, 540], [960, 180], [690, 540]]);
    // By 30°: each corner at 960 + x·cos − y·sin and 540 − (x·sin + y·cos), of its place x, y from the centre, y up.
    const turned = rectangle({ angle: 30 });
    expect(lines(turned)).toEqual([[[783.231, 953.827], [1406.769, 593.827], [1136.769, 126.173], [513.231, 486.173], [783.231, 953.827]]]);
    expect(handles(turned).map(each => each.at)).toEqual([[960, 540], [1271.769, 360], [825, 306.173]]);
  });

  it('are the outline of an Ellipse, its width and its height both in widths, and the handles at the ends of its two axes', () => {
    // A new Ellipse is a circle half the picture wide: of 360 pixels about 960, 540, whatever the shape of the picture.
    const [circle] = lines(ellipse());
    expect(circle).toHaveLength(ELLIPSE_STEPS + 1);
    expect([circle[0], circle[16], circle[32], circle[48], circle[64]]).toEqual([[1320, 540], [960, 180], [600, 540], [960, 900], [1320, 540]]);
    // An eighth of the way round: 360·cos 45° = 254.558 to the right of the centre and above it.
    expect(circle[8]).toEqual([1214.558, 285.442]);
    // Every place of it is 360 pixels from the centre, to the thousandth of a pixel that a place is written to.
    expect(circle.every(([x, y]) => Math.abs(Math.hypot(x - 960, y - 540) - 360) < 0.002)).toBe(true);
    expect(handles(ellipse())).toEqual([{ kind: 'center', at: [960, 540] }, { kind: 'width', at: [1320, 540] }, { kind: 'height', at: [960, 180] }]);
    // Half as high as wide, and a quarter turn: its long axis stands upright, and its short one lies across.
    const [upright] = lines(ellipse({ height: 0.25, angle: 90 }));
    expect([upright[0], upright[16], upright[32], upright[48]]).toEqual([[960, 180], [780, 540], [960, 900], [1140, 540]]);
    expect(handles(ellipse({ height: 0.25, angle: 90 })).map(each => each.at)).toEqual([[960, 540], [960, 180], [780, 540]]);
  });

  it('are the outline of a Polygon through its points, each from the centre in widths, sized and turned, and a handle at each', () => {
    // The four points of a new Polygon, twice their size: 0.2 of the width is 576 pixels then, both ways.
    const large = polygon({ size: 2 });
    expect(lines(large)).toEqual([[[384, 1116], [1536, 1116], [1536, -36], [384, -36], [384, 1116]]]);
    expect(handles(large)).toEqual([
      { kind: 'center', at: [960, 540] }, { kind: 'point', index: 0, at: [384, 1116] }, { kind: 'point', index: 1, at: [1536, 1116] },
      { kind: 'point', index: 2, at: [1536, -36] }, { kind: 'point', index: 3, at: [384, -36] },
    ]);
    // Half their size and a quarter turn: 0.2 of the width is 144 pixels, and the point that was at the lower left is at the lower right.
    const turned = polygon({ size: 0.5, angle: 90 });
    expect(lines(turned)).toEqual([[[1104, 684], [1104, 396], [816, 396], [816, 684], [1104, 684]]]);
    expect(handles(turned).slice(1).map(each => each.at)).toEqual([[1104, 684], [1104, 396], [816, 396], [816, 684]]);
    // About another centre.
    expect(handles(polygon({ center: { x: 0.25, y: 0.75 } })).map(each => each.at)).toEqual([[600, 270], [312, 558], [888, 558], [888, -18], [312, -18]]);
    // No point, and one: no outline. No size: every point at the centre, and no handle but the centre's.
    expect(controlsOf(polygon({ points: [] }), 0, PICTURE)).toEqual({ lines: [], handles: [{ kind: 'center', at: [960, 540] }] });
    expect(controlsOf(polygon({ points: [{ x: 0.1, y: 0 }] }), 0, PICTURE)).toEqual({ lines: [], handles: [{ kind: 'center', at: [960, 540] }, { kind: 'point', index: 0, at: [1104, 540] }] });
    expect(handles(polygon({ size: 0 }))).toEqual([{ kind: 'center', at: [960, 540] }]);
  });

  it('are the centre alone of a Merge and of a Transform', () => {
    expect(controlsOf(merge(), 0, PICTURE)).toEqual({ lines: [], handles: [{ kind: 'center', at: [960, 540] }] });
    expect(controlsOf(transform({ center: { x: 0.25, y: 0.75 } }), 0, PICTURE)).toEqual({ lines: [], handles: [{ kind: 'center', at: [600, 270] }] });
    // A centre beyond the picture is a place beyond it: one width to the right of its right edge.
    expect(handles(merge({ center: { x: 2, y: 0.5 } }))).toEqual([{ kind: 'center', at: [3120, 540] }]);
  });

  it('are where a setting that has keys is at the frame of the keys', () => {
    // The centre goes from a quarter of the width to three quarters over 50 frames: at 0, at 25 and beyond its last key.
    const keyed = rectangle({ center: { x: keys([0, 0.25], [50, 0.75]), y: 0.5 } });
    expect([handles(keyed, 0)[0].at, handles(keyed, 25)[0].at, handles(keyed, 80)[0].at]).toEqual([[600, 540], [960, 540], [1320, 540]]);
    // A number of a point of a Polygon, half way between its keys.
    const points: FusionPoint[] = [{ x: keys([0, -0.2], [10, 0]), y: -0.2 }, { x: 0.2, y: -0.2 }, { x: 0.2, y: 0.2 }];
    expect(handles(polygon({ points }), 5)[1].at).toEqual([816, 828]);
  });

  it('are on the picture wherever it lies in the frame, at its size', () => {
    // The whole frame of a timeline of 1920 by 1080; and a picture half as large, below a bar of 270 pixels.
    expect(handles(rectangle()).length).toBe(3);
    expect(controlsOf(rectangle(), 0, { x: 0, y: 0, width: 1920, height: 1080 })!.handles.map(each => each.at)).toEqual([[960, 540], [1440, 540], [960, 270]]);
    expect(controlsOf(ellipse(), 0, { x: 0, y: 270, width: 960, height: 540 })!.handles.map(each => each.at)).toEqual([[480, 540], [720, 540], [480, 300]]);
  });
});

describe('what the engine is asked to draw for the controls', () => {
  it('is the outline, the centre in the colour of the place at the playhead, and the other handles as points', () => {
    expect(overlayOf(controlsOf(rectangle(), 0, PICTURE)!, [1920, 1080])).toEqual({
      canvas: [1920, 1080], lines: [[[600, 810], [1320, 810], [1320, 270], [600, 270], [600, 810]]],
      dots: [{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1320, 540], [960, 270]] }],
    });
    // A Merge has its centre alone: no line, and no kind of dot that has none.
    expect(overlayOf(controlsOf(merge(), 0, PICTURE)!, [1920, 1080])).toEqual({ canvas: [1920, 1080], lines: [], dots: [{ kind: 'current', at: [[960, 540]] }] });
  });

  it('has the handle that is dragged on top, in its own colour', () => {
    const controls = controlsOf(rectangle(), 0, PICTURE)!;
    expect(overlayOf(controls, [1920, 1080], 0).dots).toEqual([{ kind: 'key', at: [[1320, 540], [960, 270]] }, { kind: 'active', at: [[960, 540]] }]);
    expect(overlayOf(controls, [1920, 1080], 2).dots).toEqual([{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1320, 540]] }, { kind: 'active', at: [[960, 270]] }]);
    // A handle that the node has not is none.
    expect(overlayOf(controls, [1920, 1080], 7)).toEqual(overlayOf(controls, [1920, 1080]));
    expect(overlayOf(controls, [1920, 1080], -1)).toEqual(overlayOf(controls, [1920, 1080], null));
  });

  it('names a handle by what it sets, and a point by its place among the points', () => {
    expect(handles(polygon()).map(handleName)).toEqual(['center', 'point:0', 'point:1', 'point:2', 'point:3']);
    expect(handles(rectangle()).map(handleName)).toEqual(['center', 'width', 'height']);
  });
});

describe('the handle that is pressed', () => {
  const controls = controlsOf(rectangle(), 0, PICTURE)!;
  const at = (x: number, y: number): OverlayPoint => [x, y];

  it('is the nearest within reach, and none beyond it', () => {
    expect(hitHandle(controls, at(966, 540), 16)).toBe(0);
    expect(hitHandle(controls, at(960, 556), 16)).toBe(0);
    expect(hitHandle(controls, at(977, 540), 16)).toBe(-1);
    expect(hitHandle(controls, at(1310, 545), 16)).toBe(1);
    expect(hitHandle(controls, at(950, 262), 16)).toBe(2);
    expect(hitHandle(controls, at(100, 100), 16)).toBe(-1);
    // Between two handles that are both within reach, the nearer.
    const near = controlsOf(rectangle({ width: 0.02 }), 0, PICTURE)!;
    expect(near.handles.map(each => each.at)).toEqual([[960, 540], [974.4, 540], [960, 270]]);
    expect([hitHandle(near, at(962, 540), 16), hitHandle(near, at(972, 540), 16)]).toEqual([0, 1]);
  });

  it('is the handle of the width where it lies on the centre, so that a shape of no width can be drawn out again', () => {
    const flat = controlsOf(rectangle({ width: 0 }), 0, PICTURE)!;
    expect(flat.handles.map(each => each.at)).toEqual([[960, 540], [960, 540], [960, 270]]);
    expect(hitHandle(flat, at(960, 540), 16)).toBe(1);
  });

  it('is within eight pixels of the screen, however large the picture is shown', () => {
    const shown = (width: number): Pick<ViewPointer, 'presentation'> => ({ presentation: { serial: 1, tag: 1, map: { surface: [width, 540], picture: [1920, 1080], dest: [0, 0, width, 540] } } });
    expect([reachOf(shown(960)), reachOf(shown(1920)), reachOf(shown(3840))]).toEqual([16, 8, 4]);
    // Where the view has not said how large it shows the picture, eight pixels of the frame.
    expect([reachOf({}), reachOf({ presentation: null }), reachOf({ presentation: { serial: 1, tag: null, map: null } })]).toEqual([8, 8, 8]);
  });
});

describe('the values that a drag gives the settings of a handle', () => {
  it('move the centre with the pointer: across by its share of the width, and up by its share of the height', () => {
    // 144 pixels right and 108 up: a tenth of the width and of the height.
    expect(dragged(rectangle(), { kind: 'center' }, 144, -108)).toEqual([['center.x', 0.6], ['center.y', 0.6]]);
    expect(dragged(rectangle(), { kind: 'center' }, -360, 270)).toEqual([['center.x', 0.25], ['center.y', 0.25]]);
    expect(dragged(rectangle(), { kind: 'center' }, 0, 0)).toEqual([['center.x', 0.5], ['center.y', 0.5]]);
    // The same of every kind that has a centre, and the angle of a shape does not turn the way of its centre.
    for (const node of [ellipse(), polygon(), merge(), transform(), rectangle({ angle: 30 })]) {
      expect(dragged(node, { kind: 'center' }, 144, -108), node.kind).toEqual([['center.x', 0.6], ['center.y', 0.6]]);
    }
  });

  it('are kept within the range of each setting', () => {
    // The centre of a mask from -10 to 10, of a Merge and of a Transform from -15.5 to 16.5.
    expect(dragged(rectangle(), { kind: 'center' }, 1440 * 20, 1080 * 20)).toEqual([['center.x', 10], ['center.y', -10]]);
    expect(dragged(merge(), { kind: 'center' }, 1440 * 20, 1080 * 20)).toEqual([['center.x', 16.5], ['center.y', -15.5]]);
    expect(dragged(transform(), { kind: 'center' }, -1440 * 20, -1080 * 20)).toEqual([['center.x', -15.5], ['center.y', 16.5]]);
    // A width and a height from 0 to 10, and a number of a point from -16 to 16.
    expect(dragged(rectangle(), { kind: 'width' }, 1440 * 20, 0)).toEqual([['width', 10]]);
    expect(dragged(ellipse(), { kind: 'height' }, 0, -1440 * 20)).toEqual([['height', 10]]);
    expect(dragged(polygon(), { kind: 'point', index: 0 }, -1440 * 40, -1440 * 40)).toEqual([['points.0.x', -16], ['points.0.y', 16]]);
  });

  it('widen and heighten a Rectangle and an Ellipse about their centre: twice what the pointer goes along the axis', () => {
    // 72 pixels to the right: 144 wider, a tenth of the width. Up or down, the handle of the width does nothing.
    expect(dragged(rectangle(), { kind: 'width' }, 72, 0)).toEqual([['width', 0.6]]);
    expect(dragged(rectangle(), { kind: 'width' }, 0, 99)).toEqual([['width', 0.5]]);
    expect(dragged(rectangle(), { kind: 'width' }, -72, 40)).toEqual([['width', 0.4]]);
    // To the centre, the shape has no width; through it, it comes out on the other side.
    expect(dragged(rectangle(), { kind: 'width' }, -360, 0)).toEqual([['width', 0]]);
    expect(dragged(rectangle(), { kind: 'width' }, -720, 0)).toEqual([['width', 0.5]]);
    // 54 pixels up: 108 higher, a tenth of the height of the picture, in which the height of a Rectangle is counted.
    expect(dragged(rectangle(), { kind: 'height' }, 0, -54)).toEqual([['height', 0.6]]);
    expect(dragged(rectangle(), { kind: 'height' }, 50, 54)).toEqual([['height', 0.4]]);
    // The height of an Ellipse is counted in widths: 72 pixels up are 144 higher, a tenth of the width.
    expect(dragged(ellipse(), { kind: 'height' }, 0, -72)).toEqual([['height', 0.6]]);
    expect(dragged(ellipse(), { kind: 'width' }, 72, 0)).toEqual([['width', 0.6]]);
  });

  it('follow the axes of a shape that is turned', () => {
    // A quarter turn: the handle of the width is above the centre, and is dragged up and down; across, it does nothing.
    const quarter = rectangle({ angle: 90 });
    expect(dragged(quarter, { kind: 'width' }, 0, -72)).toEqual([['width', 0.6]]);
    expect(dragged(quarter, { kind: 'width' }, 72, 0)).toEqual([['width', 0.5]]);
    // And the handle of the height is at the left: 54 pixels further left are a tenth of the height more.
    expect(dragged(quarter, { kind: 'height' }, -54, 0)).toEqual([['height', 0.6]]);
    // By 30°, along the axis: 72 pixels along it are 72·cos 30° across and 72·sin 30° up.
    const [cosine, sine] = turnOf(30);
    expect(dragged(rectangle({ angle: 30 }), { kind: 'width' }, 72 * cosine, -72 * sine)).toEqual([['width', 0.6]]);
  });

  it('move a point of a Polygon with the pointer, the mapping of the engine inverted: turned back by the Angle and divided by the Size', () => {
    // Not turned, at its own size: 144 pixels are a tenth of the width, and up is more.
    expect(dragged(polygon(), { kind: 'point', index: 2 }, 144, -144)).toEqual([['points.2.x', 0.3], ['points.2.y', 0.3]]);
    // Twice its size: the same way of the pointer is half as much of its numbers.
    expect(dragged(polygon({ size: 2 }), { kind: 'point', index: 2 }, 144, -144)).toEqual([['points.2.x', 0.25], ['points.2.y', 0.25]]);
    // Half its size and a quarter turn: the point at 816, 396 taken 72 right and 144 down. Across the screen is down its own y, and down the screen is back along its own x.
    const turned = polygon({ size: 0.5, angle: 90 });
    expect(dragged(turned, { kind: 'point', index: 2 }, 72, 144)).toEqual([['points.2.x', 0], ['points.2.y', 0.1]]);
    const after = polygon({ size: 0.5, angle: 90, points: startPoints().map((point, index) => (index === 2 ? { x: 0, y: 0.1 } : point)) });
    expect(handles(after)[3].at).toEqual([888, 540]);
  });

  it('put a point of a Polygon where the pointer took it, whatever its Size, its Angle and its centre', () => {
    const shape: Partial<FusionPolygon> = { center: { x: 0.4, y: 0.6 }, size: 0.8, angle: 30 };
    const before = handles(polygon(shape))[2].at;
    const [[, x], [, y]] = dragged(polygon(shape), { kind: 'point', index: 1 }, 50, -30) as [string, number][];
    const after = handles(polygon({ ...shape, points: startPoints().map((point, index) => (index === 1 ? { x, y } : point)) }))[2].at;
    expect(after[0]).toBeCloseTo(before[0] + 50, 1);
    expect(after[1]).toBeCloseTo(before[1] - 30, 1);
  });

  it('are worked out from the value that a setting with keys has at the frame of the keys', () => {
    const keyed = rectangle({ center: { x: keys([0, 0.25], [50, 0.75]), y: 0.5 } });
    expect(dragged(keyed, { kind: 'center' }, 144, 0, 0)).toEqual([['center.x', 0.35], ['center.y', 0.5]]);
    expect(dragged(keyed, { kind: 'center' }, 144, 0, 25)).toEqual([['center.x', 0.6], ['center.y', 0.5]]);
  });

  it('are none for a handle that the node has not', () => {
    expect(dragged(merge(), { kind: 'width' }, 72, 0)).toEqual([]);
    expect(dragged(polygon(), { kind: 'width' }, 72, 0)).toEqual([]);
    expect(dragged(polygon(), { kind: 'point', index: 9 }, 72, 0)).toEqual([]);
    expect(dragged(polygon(), { kind: 'point' }, 72, 0)).toEqual([]);
    expect(dragged(polygon({ size: 0 }), { kind: 'point', index: 0 }, 72, 0)).toEqual([]);
    expect(dragged(rectangle(), { kind: 'point', index: 0 }, 72, 0)).toEqual([]);
    expect(dragged(newNode('blur', null, 'n9'), { kind: 'center' }, 72, 0)).toEqual([]);
    // Nor on a picture that has no size.
    expect(draggedValues(rectangle(), { kind: 'center' }, 0, { x: 0, y: 0, width: 0, height: 0 }, 72, 0)).toEqual([]);
  });
});

describe('the node that the viewer shows', () => {
  const composition = { nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), rectangle()], links: [] };

  it('is the node asked for, and the output again for the node that is viewed already, for the MediaOut and for a node that is not there', () => {
    expect(viewedAfter(composition, null, 'n6')).toBe('n6');
    expect(viewedAfter(composition, 'MediaIn1', 'n6')).toBe('n6');
    expect(viewedAfter(composition, 'n6', 'n6')).toBeNull();
    expect(viewedAfter(composition, null, 'MediaOut1')).toBeNull();
    expect(viewedAfter(composition, 'n6', 'MediaOut1')).toBeNull();
    expect(viewedAfter(composition, null, 'gone')).toBeNull();
  });

  it('is asked for by the key 1, and the output by the key 0, each alone', () => {
    expect(fusionKey(key({ key: '1', code: 'Digit1' }))).toEqual({ kind: 'view' });
    expect(fusionKey(key({ key: '1', code: 'Numpad1' }))).toEqual({ kind: 'view' });
    expect(fusionKey(key({ key: '0', code: 'Digit0' }))).toEqual({ kind: 'viewOutput' });
    // A keyboard whose row of digits types something else without Shift: the key is known by where it lies.
    expect(fusionKey(key({ key: '&', code: 'Digit1' }))).toEqual({ kind: 'view' });
    for (const more of [{ ctrl: true }, { meta: true }, { alt: true }, { shift: true }, { repeat: true }, { composing: true }]) {
      expect(fusionKey(key({ key: '1', code: 'Digit1', ...more })), JSON.stringify(more)).toBeNull();
      expect(fusionKey(key({ key: '0', code: 'Digit0', ...more })), JSON.stringify(more)).toBeNull();
    }
    expect(fusionKey(key({ key: '2', code: 'Digit2' }))).toBeNull();
  });
});

// A Tracker (PROTOCOL.md, "The Tracker"): its patterns on the picture, and the centres that follow it.
/** A Tracker of these patterns. */
const trackerOf = (trackers: FusionTrackerPattern[]): FusionNode => ({ ...newNode('tracker', null, 'n9'), params: { ...newTracker(), trackers } } as FusionNode);
/** A pattern put at a quarter of the width and half the height, with a path from there at frame 0 to 0.75, 0.25 at frame 8. */
const tracked = (): FusionTrackerPattern => ({ ...newPattern(0.25, 0.5), name: 'Left', path: { 0: [0.25, 0.5], 8: [0.75, 0.25] } });

describe('the path of a pattern of a Tracker', () => {
  const moving: FusionTrackerPattern['path'] = { 10: [0.25, 0.5], 20: [0.5, 0.25], 22: [0.75, 0.25] };

  it('is where a pattern is on the line between two of its frames, and where its first or its last has it beyond them, as the engine reads it', () => {
    // Where the path has it.
    expect([placeOnPath(moving, 10), placeOnPath(moving, 20), placeOnPath(moving, 22)]).toEqual([[0.25, 0.5], [0.5, 0.25], [0.75, 0.25]]);
    // Between two frames of it, whole and not.
    expect([placeOnPath(moving, 15), placeOnPath(moving, 21), placeOnPath(moving, 12.5)]).toEqual([[0.375, 0.375], [0.625, 0.25], [0.3125, 0.4375]]);
    // Before its first frame and after its last.
    for (const frame of [9, 0, -1000, -Infinity]) expect(placeOnPath(moving, frame), String(frame)).toEqual([0.25, 0.5]);
    for (const frame of [22.5, 23, 1e9, Infinity]) expect(placeOnPath(moving, frame), String(frame)).toEqual([0.75, 0.25]);
    // One frame is every frame; no path is no place; and a frame that is no number.
    for (const frame of [-3, 7, 7.5, 400]) expect(placeOnPath({ 7: [0.125, 0.875] }, frame), String(frame)).toEqual([0.125, 0.875]);
    expect([placeOnPath({}, 7), placeOnPath(moving, Number.NaN)]).toEqual([null, null]);
  });

  it('has its frames in their order as numbers, below 0 too, whatever order they are written in', () => {
    expect(pathSamples({ 10: [0.5, 0.5], '-2': [0.25, 0.5], 9: [0.75, 0.5] }).map(([frame]) => frame)).toEqual([-2, 9, 10]);
    expect(placeOnPath({ 10: [0.5, 0.5], '-2': [0.25, 0.5], 9: [0.75, 0.5] }, 9.5)).toEqual([0.625, 0.5]);
  });

  it('is thinned to at most a few hundred places, the first and the last always among them', () => {
    const many = Array.from({ length: 1000 }, (_, index) => index);
    const few = thinned(many, MOST_PATH_POINTS);
    expect([MOST_PATH_POINTS, few.length, few[0], few.at(-1)]).toEqual([256, 256, 0, 999]);
    expect(few.every((each, index) => index === 0 || each > few[index - 1])).toBe(true);
    // As few as that, or fewer: all of them, in a list of its own.
    const short = [1, 2, 3];
    expect(thinned(short, MOST_PATH_POINTS)).toEqual(short);
    expect(thinned(short, MOST_PATH_POINTS)).not.toBe(short);
    expect(thinned(many, 2)).toEqual([0, 999]);
  });
});

describe('the controls of a Tracker on the picture', () => {
  it('are of each pattern its box and the box that it is searched for in, about its middle, and a handle there', () => {
    expect(hasControls('tracker')).toBe(true);
    // A new pattern in the middle: 0.05 of the picture wide and high is 72 by 54 pixels, and its search area, an eighth, is 180 by 135.
    expect(controlsOf(trackerOf([newPattern()]), 0, PICTURE)).toEqual({
      lines: [
        [[924, 567], [996, 567], [996, 513], [924, 513], [924, 567]],
        [[870, 607.5], [1050, 607.5], [1050, 472.5], [870, 472.5], [870, 607.5]],
      ],
      handles: [{ kind: 'pattern', index: 0, at: [960, 540] }],
      marks: [],
    });
    expect(handleName({ kind: 'pattern', index: 2 })).toBe('pattern:2');
    // A Tracker without patterns has nothing to draw.
    expect(controlsOf(trackerOf([]), 0, PICTURE)).toEqual({ lines: [], handles: [], marks: [] });
  });

  it('are about where a pattern is on its path at the playhead, with its path as a line through its frames and a mark of the playhead there', () => {
    // At frame 2, a quarter of the way from 0.25, 0.5 to 0.75, 0.25: at 0.375, 0.4375, which is 780, 607.5.
    const controls = controlsOf(trackerOf([tracked()]), 2, PICTURE)!;
    expect(controls.lines).toEqual([
      [[744, 634.5], [816, 634.5], [816, 580.5], [744, 580.5], [744, 634.5]],
      [[690, 675], [870, 675], [870, 540], [690, 540], [690, 675]],
      [[600, 540], [1320, 810]],
    ]);
    expect([controls.handles, controls.marks]).toEqual([[{ kind: 'pattern', index: 0, at: [780, 607.5] }], [[780, 607.5]]]);
    // Drawn, the handle is a point as the handles of a mask are, and the mark of the playhead lies over it.
    expect(overlayOf(controls, [1920, 1080])).toEqual({
      canvas: [1920, 1080], lines: controls.lines, dots: [{ kind: 'key', at: [[780, 607.5]] }, { kind: 'current', at: [[780, 607.5]] }],
    });
    expect(overlayOf(controls, [1920, 1080], 0).dots).toEqual([{ kind: 'current', at: [[780, 607.5]] }, { kind: 'active', at: [[780, 607.5]] }]);
    // Before its first frame where the first has it, after its last where the last has it.
    expect(controlsOf(trackerOf([tracked()]), -4, PICTURE)!.handles[0].at).toEqual([600, 540]);
    expect(controlsOf(trackerOf([tracked()]), 30, PICTURE)!.handles[0].at).toEqual([1320, 810]);
  });

  it('leave out a pattern that is off, and draw one that is dragged where the drag has put it', () => {
    // The first is off: the second alone, by its place in the list.
    const two = controlsOf(trackerOf([{ ...newPattern(), enabled: false }, tracked()]), 0, PICTURE)!;
    expect(two.handles).toEqual([{ kind: 'pattern', index: 1, at: [600, 540] }]);
    expect(two.lines).toHaveLength(3);
    // Dragged to the middle: its boxes and its handle there, its path and the mark of the playhead where they were.
    const placed = controlsOf(trackerOf([tracked()]), 2, PICTURE, { placing: { index: 0, place: [0.5, 0.5] } })!;
    expect(placed.lines[0]).toEqual([[924, 567], [996, 567], [996, 513], [924, 513], [924, 567]]);
    expect([placed.handles[0].at, placed.lines[2], placed.marks]).toEqual([[960, 540], [[600, 540], [1320, 810]], [[780, 607.5]]]);
  });

  it('draw a long path through a few hundred of its places, the first and the last among them', () => {
    // A thousand frames, each a pixel to the right of the one before.
    const path = Object.fromEntries(Array.from({ length: 1000 }, (_, index) => [String(index), [0.25 + index / 1440, 0.5] as [number, number]]));
    const [, , line] = controlsOf(trackerOf([{ ...newPattern(0.25, 0.5), path }]), 0, PICTURE)!.lines;
    expect(line).toHaveLength(MOST_PATH_POINTS);
    expect([line[0], line.at(-1)]).toEqual([[600, 540], [1599, 540]]);
    expect(line.every(([x, y], index) => y === 540 && (index === 0 || x > line[index - 1][0]))).toBe(true);
  });

  it('are dragged by the place where a pattern is put: from where its handle was drawn, as a centre is, and never by its path', () => {
    const node = trackerOf([newPattern(), tracked()]);
    // 144 pixels right and 108 up: a tenth of the width and of the height, from where it was put.
    expect(draggedPattern(node, 0, 0, PICTURE, 144, -108)).toEqual([0.6, 0.6]);
    // A pattern with a path, from where the path has it at the frame of the press.
    expect(draggedPattern(node, 1, 2, PICTURE, 144, 0)).toEqual([0.475, 0.4375]);
    // Within the range of a place.
    expect(draggedPattern(node, 0, 0, PICTURE, 1440 * 20, 1080 * 20)).toEqual([16.5, -15.5]);
    // Nothing of a pattern that is not there, of another kind, or on a picture that has no size; and no setting that takes keys.
    expect([
      draggedPattern(node, 2, 0, PICTURE, 10, 10), draggedPattern(newNode('blur', null, 'b1'), 0, 0, PICTURE, 10, 10),
      draggedPattern(node, 0, 0, { x: 0, y: 0, width: 0, height: 0 }, 10, 10),
    ]).toEqual([null, null, null]);
    expect(draggedValues(node, { kind: 'pattern', index: 0 }, 0, PICTURE, 144, 0)).toEqual([]);
    expect(draggedValues(node, { kind: 'center' }, 0, PICTURE, 144, 0)).toEqual([]);
  });
});

describe('a centre that follows a Tracker, on the picture', () => {
  it('is drawn where it follows the Tracker to, and is not taken hold of', () => {
    // A Merge whose centre follows a pattern that is at a quarter of the width and three quarters of the height.
    const merged = controlsOf(merge(), 0, PICTURE, { follows: { place: [0.25, 0.75] } })!;
    expect(merged).toEqual({ lines: [], handles: [{ kind: 'center', at: [600, 270], fixed: true }] });
    expect(overlayOf(merged, [1920, 1080]).dots).toEqual([{ kind: 'current', at: [[600, 270]] }]);
    expect(hitHandle(merged, [600, 270], 16)).toBe(-1);
    // A Rectangle: its outline and its other handles about the place that it follows, and those are taken hold of still.
    const box = controlsOf(rectangle(), 0, PICTURE, { follows: { place: [0.25, 0.75] } })!;
    expect(box.lines).toEqual([[[240, 540], [960, 540], [960, 0], [240, 0], [240, 540]]]);
    expect(box.handles).toEqual([{ kind: 'center', at: [600, 270], fixed: true }, { kind: 'width', at: [960, 270] }, { kind: 'height', at: [600, 0] }]);
    expect([hitHandle(box, [600, 270], 16), hitHandle(box, [960, 270], 16)]).toEqual([-1, 1]);
    // Where the page does not work out what it follows (a Steady Position), at its own place; and held within the range of the centre of a mask.
    expect(controlsOf(rectangle(), 0, PICTURE, { follows: { place: null } })!.handles[0]).toEqual({ kind: 'center', at: [960, 540], fixed: true });
    expect(controlsOf(rectangle(), 0, PICTURE, { follows: { place: [12, 0.5] } })!.handles[0].at).toEqual([14640, 540]);
    // A centre that follows nothing is as it was.
    expect(controlsOf(merge(), 0, PICTURE, { follows: null })).toEqual(controlsOf(merge(), 0, PICTURE));
  });
});
