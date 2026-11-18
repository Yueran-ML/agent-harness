/**
 * What the Fusion page asks of the engine, worked out apart from the page (PROTOCOL.md,
 * "Compositions"): the operations of each command of the node editor, the layout of a composition,
 * the settings of the nodes as the engine's catalog has them, and the operations that set them and
 * their keys.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Key } from '../src/app/keys';
import { COLUMN_GAP, linkHalves, linkPath, MARGIN, NODE_HEIGHT, NODE_WIDTH } from '../src/studio/color/graph';
import {
  applyOp, firstComposition, type Composition, type FusionFollow, type FusionLink, type FusionNode, type FusionPoint, type FusionRectangle, type FusionTracker,
  type FusionTrackerPattern, type Keyframe, type Op, type Project,
} from '../src/studio/engine';
import {
  ADDABLE, ADDABLE_EFFECTS, ADDABLE_MASKS, addedNode, addedNodes, addNode, CENTER_KINDS, connect, copyNodes, cutNodes, disconnect, duplicateNodes, followOf,
  followOps, fusionKey, hasCenter, insertOnLink, insertPoint, isFirst, LEAST_POINTS, MOST_NODES, MOST_POINTS, MOST_TRACKERS, moveNodes, newBackground,
  newDeltaKeyer, newMerge, newNode, newPattern, newPolygon, newTracker, newTransform, PASTE_OFFSET, pastedName, pasteNodes, reconnect, relabel, removeComposition,
  removeNodes, removePoint, resetComposition, restoreNodes, roomFor, shownComposition, startPoints, togglePass, withFirst, withFollow,
} from '../src/studio/fusion/edits';
import {
  fusionLinkHalves, fusionLinkNear, fusionLinkPath, handedOn, hasMaskInput, hasOutput, INPUTS, inputPoint, isMaskKind, isMatteInput, isPictureInput,
  layoutComposition, linkEnds, linkName, MASK_INSET, MATTE_INPUTS, MATTE_INSETS, matteInputs, NAME_ROOM, nameWidth, namesOf, nodesIn, outputPoint, pictureInputs,
  placesOf, portAt, portsOf,
} from '../src/studio/fusion/graph';
import {
  addKey, FUSION_SETTINGS, hasKeys, isPointSetting, keyerRefusals, keyOps, keyTimesOf, numbered, paramOf, pathOf, pointPath, putValue, putValues, removeKey,
  settingOf, settingsOf, wholeParam, wholeValue,
} from '../src/studio/fusion/settings';
import {
  addPattern, clearPath, enablePattern, followChoices, followedPlace, followName, patternName, placedPattern, removePattern, renamePattern, restoredPatterns,
  setPatternValue, setTracker,
} from '../src/studio/fusion/tracker';
import { clip, media, timeline } from './timeline-fixture';

const AT = { timeline: 't1', clip: 'c1' };
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
const mediaIn = (id = 'MediaIn1'): FusionNode => newNode('media_in', null, id);
const mediaOut = (id = 'MediaOut1'): FusionNode => newNode('media_out', null, id);
const merge = (id: string): FusionNode => newNode('merge', null, id);
const transform = (id: string): FusionNode => newNode('transform', null, id);
const background = (id: string): FusionNode => newNode('background', null, id);
/** MediaIn1 through the Transform n3 into the background of the Merge n4, the Background n5 its foreground, and the Merge into MediaOut1. */
const comp = (): Composition => ({
  nodes: [mediaIn(), mediaOut(), transform('n3'), merge('n4'), background('n5')],
  links: [link('MediaIn1', 'n3'), link('n3', 'n4', 'background'), link('n5', 'n4', 'foreground'), link('n4', 'MediaOut1')],
});
const key = (given: Partial<Key>): Key => ({ key: '', shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...given });

describe('the composition that a clip is shown with', () => {
  it('is its own, or the first one where it has none, the same object each time', () => {
    const own = comp();
    expect(shownComposition({ fusion: own })).toBe(own);
    expect(shownComposition({})).toEqual(firstComposition());
    expect(shownComposition({})).toBe(shownComposition({}));
    expect(isFirst(shownComposition({}))).toBe(true);
    expect(isFirst(own)).toBe(false);
    // A node of the first one that was dragged is no longer the first composition.
    const dragged = firstComposition();
    dragged.nodes[0] = { ...dragged.nodes[0], pos: { x: 3, y: 4 } };
    expect(isFirst(dragged)).toBe(false);
  });

  it('is sent before the first change of a clip that has none, in the transaction of the change', () => {
    const change: Op[] = [{ op: 'set_fusion_node', ...AT, id: 'MediaIn1', pass_through: true }];
    expect(withFirst(AT, true, change)).toBe(change);
    expect(withFirst(AT, false, change)).toEqual([{ op: 'set_fusion', ...AT, value: firstComposition() }, ...change]);
    // Nothing to do sends nothing, and a whole composition put in place is sent as it is.
    expect(withFirst(AT, false, [])).toEqual([]);
    const whole: Op[] = [{ op: 'set_fusion', ...AT, value: null }];
    expect(withFirst(AT, false, whole)).toBe(whole);
  });
});

describe('the commands of the node editor', () => {
  it('add a node with no links where none is chosen, at the place given', () => {
    expect(addNode(AT, comp(), 'merge', null, { x: 40.4, y: 80.6 })).toEqual([
      { op: 'add_fusion_node', ...AT, node: { id: '', kind: 'merge', label: '', pass_through: false, pos: { x: 40, y: 81 }, params: newMerge() } },
    ]);
    expect(addNode(AT, comp(), 'transform', null, null)).toEqual([
      { op: 'add_fusion_node', ...AT, node: { id: '', kind: 'transform', label: '', pass_through: false, pos: null, params: newTransform() } },
    ]);
  });

  it('insert a node after the one chosen: its output into the first input of the new node, and the new node into all that it fed', () => {
    // After the Transform, which feeds the background of the Merge.
    expect(addNode(AT, comp(), 'transform', 'n3', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('transform'), links: [link('n3', '', 'input'), link('', 'n4', 'background')],
    }]);
    // A Merge takes what was chosen as its background.
    expect(addNode(AT, comp(), 'merge', 'n4', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('merge'), links: [link('n4', '', 'background'), link('', 'MediaOut1', 'input')],
    }]);
    // A node that feeds several inputs is put before every one of them.
    const fan: Composition = { nodes: [mediaIn(), mediaOut(), merge('n4')], links: [link('MediaIn1', 'n4', 'background'), link('MediaIn1', 'n4', 'foreground'), link('n4', 'MediaOut1')] };
    expect(addNode(AT, fan, 'transform', 'MediaIn1', null)[0]).toMatchObject({
      links: [link('MediaIn1', '', 'input'), link('', 'n4', 'background'), link('', 'n4', 'foreground')],
    });
    // A node without an input cannot go after another, and nothing goes after the MediaOut: both are added loose.
    expect(addNode(AT, comp(), 'background', 'n3', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('background') }]);
    expect(addNode(AT, comp(), 'transform', 'MediaOut1', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('transform') }]);
    expect(newNode('background').params).toEqual(newBackground());
  });

  it('tell the node that a transaction added', () => {
    expect(addedNode([{ op: 'add_fusion_node', ...AT, node: newNode('merge', null, 'n12') }])).toBe('n12');
    expect(addedNode([{ op: 'set_fusion', ...AT, value: null }])).toBeNull();
    expect(addedNode(null)).toBeNull();
  });

  it('delete nodes, joining what fed each to what it fed where that is a single chain, and leave the MediaOut', () => {
    // The Transform has one link into it: the MediaIn is linked to the background that it fed.
    expect(removeNodes(AT, comp(), ['n3'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4', 'background')] }]);
    // The Merge has two links into it: what it fed is left without a picture.
    expect(removeNodes(AT, comp(), ['n4'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n4' }]);
    // A node that nothing feeds has nothing to join.
    expect(removeNodes(AT, comp(), ['n5'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n5' }]);
    // The MediaOut is not deleted, alone or among others, and a node that is not there is passed over.
    expect(removeNodes(AT, comp(), ['MediaOut1'])).toEqual([]);
    expect(removeNodes(AT, comp(), ['MediaOut1', 'n5', 'gone'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n5' }]);
    // Two nodes of a chain: the second is taken from the composition as the first left it, so the chain is joined across both.
    const chain: Composition = { nodes: [mediaIn(), mediaOut(), transform('n3'), transform('n4')], links: [link('MediaIn1', 'n3'), link('n3', 'n4'), link('n4', 'MediaOut1')] };
    const ops = removeNodes(AT, chain, ['n3', 'n4']);
    expect(ops).toEqual([
      { op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4')] },
      { op: 'remove_fusion_node', ...AT, id: 'n4', links: [link('MediaIn1', 'MediaOut1')] },
    ]);
    // Applied as the copy applies them, they leave the first composition's link.
    let project: Project = { schema: 5, next_id: 10, bins: [], groups: [], media: [media], timelines: [timeline([clip('c1', 0, 100, { fusion: chain })])] };
    for (const op of ops) project = applyOp(project, op);
    const left = (project.timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!;
    expect(left.links).toEqual([link('MediaIn1', 'MediaOut1')]);
  });

  it('move nodes to whole pixels, each a set_fusion_node of its place', () => {
    const placed: Composition = { ...comp(), nodes: comp().nodes.map(node => (node.id === 'n3' ? { ...node, pos: { x: 10, y: 20 } } : node)) };
    expect(moveNodes(AT, placed, { n3: { x: 10.2, y: 19.8 }, n4: { x: 200.6, y: 30.4 } })).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n4', pos: { x: 201, y: 30 } }]);
    // A move of a drag sends every node dragged, whether or not it is where it was: it is the same whatever went before it.
    expect(moveNodes(AT, placed, { n3: { x: 10, y: 20 } }, true)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 10, y: 20 } }]);
    // Taken back, each goes where it was: at its place, or where the editor lays it out.
    expect(restoreNodes(AT, placed, ['n3', 'n4'])).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 10, y: 20 } }, { op: 'set_fusion_node', ...AT, id: 'n4', pos: null },
    ]);
  });

  it('link, unlink and take the end of a link to another input', () => {
    expect(connect(AT, comp(), link('n5', 'n4', 'background'))).toEqual([{ op: 'connect_fusion', ...AT, link: link('n5', 'n4', 'background') }]);
    // The link that is there already sends nothing: alone in a transaction the engine would refuse it as empty.
    expect(connect(AT, comp(), link('n3', 'n4', 'background'))).toEqual([]);
    expect(disconnect(AT, link('n3', 'n4', 'background'))).toEqual([{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'background' }]);
    expect(reconnect(AT, link('n5', 'n4', 'foreground'), { to: 'MediaOut1', input: 'input' })).toEqual([
      { op: 'disconnect_fusion', ...AT, to: 'n4', input: 'foreground' }, { op: 'connect_fusion', ...AT, link: link('n5', 'MediaOut1') },
    ]);
    expect(reconnect(AT, link('n5', 'n4', 'foreground'), { to: 'n4', input: 'foreground' })).toEqual([]);
  });

  it('pass nodes through, rename them, and reset or remove the whole composition', () => {
    const passed: Composition = { ...comp(), nodes: comp().nodes.map(node => (node.id === 'n4' ? { ...node, pass_through: true } : node)) };
    expect(togglePass(AT, passed, ['n3', 'n4'])).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n3', pass_through: true }, { op: 'set_fusion_node', ...AT, id: 'n4', pass_through: false },
    ]);
    expect(relabel(AT, comp(), 'n3', 'Shake')).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n3', label: 'Shake' }]);
    expect(relabel(AT, comp(), 'n3', '')).toEqual([]);
    expect(resetComposition(AT, { fusion: comp() })).toEqual([{ op: 'set_fusion', ...AT, value: firstComposition() }]);
    expect(resetComposition(AT, { fusion: firstComposition() })).toEqual([]);
    expect(resetComposition(AT, {})).toEqual([]);
    expect(removeComposition(AT, { fusion: comp() })).toEqual([{ op: 'set_fusion', ...AT, value: null }]);
    expect(removeComposition(AT, {})).toEqual([]);
  });

  it('are asked for by keys that the transport does not take', () => {
    expect(fusionKey(key({ key: 'Delete' }))).toEqual({ kind: 'delete', key: 'Delete' });
    expect(fusionKey(key({ key: 'Backspace' }))).toEqual({ kind: 'delete', key: 'Backspace' });
    expect(fusionKey(key({ key: 'p', code: 'KeyP', ctrl: true }))).toEqual({ kind: 'passThrough' });
    expect(fusionKey(key({ key: 'a', code: 'KeyA', ctrl: true }))).toEqual({ kind: 'selectAll' });
    expect(fusionKey(key({ key: 'F2' }))).toEqual({ kind: 'rename' });
    expect(fusionKey(key({ key: 'Escape' }))).toEqual({ kind: 'cancel' });
    // The keys of the transport, and the same letters without Ctrl, ask nothing of the editor.
    for (const plain of [' ', 'j', 'k', 'l', 'p', 'a', 'ArrowLeft', 'ArrowRight', 'Home', 'End']) expect(fusionKey(key({ key: plain })), plain).toBeNull();
    expect(fusionKey(key({ key: 'Delete', repeat: true }))).toBeNull();
    expect(fusionKey(key({ key: 'p', code: 'KeyP', ctrl: true, shift: true }))).toBeNull();
    expect(fusionKey(key({ key: 'F2', composing: true }))).toBeNull();
  });
});

describe('the layout of a composition', () => {
  it('puts a node that nothing feeds in the first column and every other one right of what feeds it', () => {
    const layout = layoutComposition(comp());
    const at = (id: string) => { const placed = layout.nodes.find(each => each.node.id === id)!; return [placed.column, placed.row]; };
    expect([at('MediaIn1'), at('n5'), at('n3'), at('n4'), at('MediaOut1')]).toEqual([[0, 0], [0, 1], [1, 0], [2, 0], [3, 0]]);
    const merged = layout.nodes.find(each => each.node.id === 'n4')!;
    expect(merged.x).toBe(MARGIN + 2 * (NODE_WIDTH + COLUMN_GAP));
    expect(layout.width).toBe(MARGIN + 3 * (NODE_WIDTH + COLUMN_GAP) + NODE_WIDTH + MARGIN);
    // The links are in the engine's order: by the node they go into, then by the name of the input.
    expect(layout.links.map(linkName)).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
  });

  it('keeps a node where the project puts it, and finds the ports, the links and the nodes under a point', () => {
    const placed: Composition = { ...comp(), nodes: comp().nodes.map(node => (node.id === 'n5' ? { ...node, pos: { x: 300, y: 200 } } : node)) };
    expect(placesOf(placed)).toEqual({ n5: { x: 300, y: 200 } });
    const layout = layoutComposition(placed, placesOf(placed));
    const five = layout.nodes.find(each => each.node.id === 'n5')!;
    expect([five.x, five.y]).toEqual([300, 200]);
    const four = layout.nodes.find(each => each.node.id === 'n4')!;
    // The background of a Merge is above its foreground; its output is at the middle of its right side.
    expect(inputPoint(four, 'background').y).toBeLessThan(inputPoint(four, 'foreground').y);
    expect(outputPoint(four)).toEqual({ x: four.x + NODE_WIDTH, y: four.y + NODE_HEIGHT / 2 });
    expect(linkEnds(layout, link('n5', 'n4', 'foreground'))).toEqual({ from: outputPoint(five), to: inputPoint(four, 'foreground') });
    expect(linkEnds(layout, link('gone', 'n4', 'foreground'))).toBeNull();
    expect(portAt(layout, inputPoint(four, 'foreground'))).toEqual({ node: 'n4', side: 'in', input: 'foreground' });
    expect(portAt(layout, outputPoint(five), port => port.side === 'in')).toBeNull();
    expect(nodesIn(layout, { x: 290, y: 190 }, { x: 310, y: 210 })).toEqual(['n5']);
    // The pictures of a Merge are the two inputs down its side; its effect mask, the third, lies on its lower edge.
    expect(INPUTS.merge).toEqual(['background', 'foreground', 'mask']);
    expect(pictureInputs('merge')).toEqual(['background', 'foreground']);
    expect([hasOutput('media_out'), hasOutput('media_in')]).toEqual([false, true]);
  });

  it('names a node by its label, or by its kind and its number among those of its kind', () => {
    const kinds = {
      media_in: 'MediaIn', media_out: 'MediaOut', merge: 'Merge', transform: 'Transform', background: 'Background',
      rectangle: 'Rectangle', ellipse: 'Ellipse', polygon: 'Polygon', blur: 'Blur', mosaic: 'Mosaic', delta_keyer: 'Delta Keyer', tracker: 'Tracker',
      planar_tracker: 'Planar Tracker', text_plus: 'Text+',
    };
    const labelled: Composition = { ...comp(), nodes: [...comp().nodes.map(node => (node.id === 'n4' ? { ...node, label: 'Over the sky' } : node)), transform('n6')] };
    expect([...namesOf(labelled, kinds)]).toEqual([
      ['MediaIn1', 'MediaIn 1'], ['MediaOut1', 'MediaOut 1'], ['n3', 'Transform 1'], ['n4', 'Over the sky'], ['n5', 'Background 1'], ['n6', 'Transform 2'],
    ]);
  });

  it('makes a mask of each kind with every setting, as the engine writes a new one, and knows the mask that comes into it', () => {
    const common = { level: 1, soft_edge: 0, border_width: 0, invert: false, solid: true, paint_mode: 'merge', center: { x: 0.5, y: 0.5 } };
    const base = { label: '', pass_through: false, pos: null };
    expect(newNode('rectangle', null, 'r1')).toStrictEqual({ id: 'r1', ...base, kind: 'rectangle', params: { ...common, width: 0.5, height: 0.5, corner_radius: 0, angle: 0 } });
    expect(newNode('ellipse', null, 'e1')).toStrictEqual({ id: 'e1', ...base, kind: 'ellipse', params: { ...common, width: 0.5, height: 0.5, angle: 0 } });
    expect(newNode('polygon', null, 'p1')).toStrictEqual({ id: 'p1', ...base, kind: 'polygon', params: { ...common, size: 1, angle: 0, points: [], fill: 'alternate' } });
    for (const kind of ['rectangle', 'ellipse', 'polygon'] as const) {
      expect(INPUTS[kind]).toEqual(['mask']);
      expect(hasOutput(kind)).toBe(true);
    }
  });
});

interface Entry { path: string; default: number; low: number; high: number; unit: string; page: string; palette: string; clock: string }

describe('the settings of the nodes that take keys', () => {
  it('are those of the catalog of the engine, with its defaults, ranges and units', () => {
    // By its path from the folder that the tests run in, as a test of a page reads a file.
    const catalog: Record<string, Entry[]> = JSON.parse(readFileSync('../engine/crates/model/tests/data/catalog.json', 'utf8'));
    for (const kind of ['merge', 'transform', 'background'] as const) {
      const entries = catalog[`fusion_${kind}`];
      expect(FUSION_SETTINGS[kind].map(setting => ({ path: setting.path, default: setting.bounds.neutral, low: setting.bounds.low, high: setting.bounds.high, unit: setting.unit })), kind)
        .toEqual(entries.map(entry => ({ path: entry.path, default: entry.default, low: entry.low, high: entry.high, unit: entry.unit })));
      expect(entries.every(entry => entry.page === 'fusion' && entry.palette === 'fusion' && entry.clock === 'source_frame')).toBe(true);
    }
    // A new node has every setting at the default of the catalog; a MediaIn and a MediaOut have none.
    for (const node of [merge('n1'), transform('n2'), background('n3')]) {
      for (const setting of settingsOf(node)) expect(paramOf(node, setting), `${node.kind}.${setting.path}`).toBe(setting.bounds.neutral);
    }
    expect(settingsOf(mediaIn())).toEqual([]);
    expect(settingsOf(mediaOut())).toEqual([]);
    expect(settingOf(merge('n1'), 'apply')).toBeUndefined();
  });

  const blend = settingOf(merge('n4'), 'blend')!;
  const centerX = settingOf(merge('n4'), 'center.x')!;
  const keyed = (keys: [number, number][]): FusionNode => ({ ...merge('n4'), params: { ...newMerge(), blend: { keys: keys.map(([frame, value]) => ({ frame, value, interp: 'linear' as const })) } } } as FusionNode);

  it('are set as one number until they have a key, and then as a key at the frame', () => {
    expect(pathOf(merge('n4'), centerX)).toBe('n4.center.x');
    expect(putValue(AT, merge('n4'), centerX, 0.25, 12)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n4', params: { center: { x: 0.25 } } }]);
    expect(putValue(AT, merge('n4'), centerX, 0.5, 12)).toEqual([]);
    const node = keyed([[10, 0.25], [30, 0.75]]);
    expect(hasKeys(node)).toBe(true);
    expect(hasKeys(merge('n4'))).toBe(false);
    // A value at a frame that has a key changes the key; at another frame it puts a key there, among the others in order.
    expect(putValue(AT, node, blend, 0.4, 30)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 1, remove: 1, keys: [{ frame: 30, value: 0.4, interp: 'linear' }] }]);
    expect(putValue(AT, node, blend, 0.4, 20)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 1, remove: 0, keys: [{ frame: 20, value: 0.4, interp: 'linear' }] }]);
    expect(putValue(AT, node, blend, 0.75, 30)).toEqual([]);
  });

  it('take a key at the playhead, of the value that they have there, and lose it again', () => {
    // The first key of a setting is spliced in at 0: the number becomes the keys.
    expect(addKey(AT, merge('n4'), blend, 12)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 0, remove: 0, keys: [{ frame: 12, value: 1, interp: 'linear' }] }]);
    const node = keyed([[10, 0.25], [30, 0.75]]);
    expect(addKey(AT, node, blend, 20)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 1, remove: 0, keys: [{ frame: 20, value: 0.5, interp: 'linear' }] }]);
    expect(addKey(AT, node, blend, 10)).toEqual([]);
    expect(removeKey(AT, node, blend, 10)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 0, remove: 1, keys: [] }]);
    expect(removeKey(AT, node, blend, 11)).toEqual([]);
    // The last key leaves its value, as one number.
    expect(removeKey(AT, keyed([[10, 0.25]]), blend, 10)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'n4.blend', param: 0.25 }]);
  });

  it('are made what a curve editor leaves them, by the keys that differ', () => {
    const node = keyed([[10, 0.25], [30, 0.75]]);
    const moved = { keys: [{ frame: 10, value: 0.25, interp: 'linear' as const }, { frame: 40, value: 0.75, interp: 'linear' as const }] };
    expect(keyOps(AT, node, blend, moved)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 1, remove: 1, keys: [moved.keys[1]] }]);
    expect(keyOps(AT, node, blend, paramOf(node, blend))).toEqual([]);
    expect(keyOps(AT, node, blend, 0.3)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'n4.blend', param: 0.3 }]);
    expect(keyOps(AT, node, blend, { keys: [] })).toEqual([]);
  });

  it('are set whole at each move of a drag, from what they were as it began, and put back whole', () => {
    expect(wholeValue(AT, merge('n4'), blend, 1, 0.6, 12)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n4', params: { blend: 0.6 } });
    const initial = paramOf(keyed([[10, 0.25], [30, 0.75]]), blend);
    expect(wholeValue(AT, merge('n4'), blend, initial, 0.6, 30)).toEqual({ op: 'set_fusion_keys', ...AT, path: 'n4.blend', param: { keys: [
      { frame: 10, value: 0.25, interp: 'linear' }, { frame: 30, value: 0.6, interp: 'linear' },
    ] } });
    expect(wholeValue(AT, merge('n4'), blend, initial, 0.6, 20)).toEqual({ op: 'set_fusion_keys', ...AT, path: 'n4.blend', param: { keys: [
      { frame: 10, value: 0.25, interp: 'linear' }, { frame: 20, value: 0.6, interp: 'linear' }, { frame: 30, value: 0.75, interp: 'linear' },
    ] } });
    expect(wholeParam(AT, merge('n4'), blend, initial)).toEqual({ op: 'set_fusion_keys', ...AT, path: 'n4.blend', param: initial });
    expect(wholeParam(AT, merge('n4'), blend, 1)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n4', params: { blend: 1 } });
  });

  it('have their keys at the frames of the timeline at which the clip shows their pictures', () => {
    // The clip begins at frame 100 of the timeline and at the first picture of its media: the key of picture 10 is at frame 110.
    const shown = clip('c1', 100, 50);
    const param = paramOf(keyed([[10, 0.25], [30, 0.75], [90, 0.1]]), blend);
    expect(keyTimesOf(shown, timeline(), [media], param)).toEqual([110, 130]);
    expect(keyTimesOf(shown, timeline(), [media], 0.5)).toEqual([]);
  });
});

// The masks (PROTOCOL.md, "Masks"): a Rectangle, an Ellipse and a Polygon, and the `mask` input of the nodes that have one.
const rectangle = (id: string): FusionNode => newNode('rectangle', null, id);
const ellipse = (id: string): FusionNode => newNode('ellipse', null, id);
/** A Polygon of these points. */
const polygon = (points: FusionPoint[], id = 'p1'): FusionNode => ({ ...newNode('polygon', null, id), params: { ...newPolygon(), points } } as FusionNode);
/** `comp` with the Rectangle n6 linked into the mask of the Merge n4. */
const masked = (): Composition => ({ nodes: [...comp().nodes, rectangle('n6')], links: [...comp().links, link('n6', 'n4', 'mask')] });
/** A composition of a MediaIn into a MediaOut and one more node, which is its third. */
const withNode = (node: FusionNode): Composition => ({ nodes: [mediaIn(), mediaOut(), node], links: [link('MediaIn1', 'MediaOut1')] });
/** A composition as the copy leaves it once it has applied operations to the one clip that has it. */
const applied = (fusion: Composition, ops: Op[]): Composition => {
  const project: Project = { schema: 5, next_id: 20, bins: [], groups: [], media: [media], timelines: [timeline([clip('c1', 0, 100, { fusion })])] };
  return (ops.reduce(applyOp, project).timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!;
};
const keyAt = (frame: number, value: number): Keyframe => ({ frame, value, interp: 'linear' });
/** The points that an operation sets a Polygon to. */
const pointsOf = (ops: Op[]): FusionPoint[] => ((ops[0] as Extract<Op, { op: 'set_fusion_node' }>).params as unknown as { points: FusionPoint[] }).points;
const paths = (node: FusionNode) => settingsOf(node).map(setting => setting.path);

describe('the masks in the node editor', () => {
  it('are added in a group of their own: linked into the mask input of the node chosen where that is free, and loose otherwise', () => {
    expect(ADDABLE_MASKS).toEqual(['rectangle', 'ellipse', 'polygon']);
    expect(ADDABLE.some(isMaskKind)).toBe(false);
    // None chosen: with no links, at the place given.
    expect(addNode(AT, comp(), 'rectangle', null, { x: 10.4, y: 20.6 })).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('rectangle', { x: 10, y: 21 }) }]);
    // A Merge, a Transform and a Background chosen, with nothing in their masks: the new mask goes into it, in the one operation.
    expect(addNode(AT, comp(), 'rectangle', 'n4', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('rectangle'), links: [link('', 'n4', 'mask')] }]);
    expect(addNode(AT, comp(), 'ellipse', 'n3', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n3', 'mask')] }]);
    expect(addNode(AT, comp(), 'ellipse', 'n5', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n5', 'mask')] }]);
    // A mask chosen: the new one is the mask that comes into it.
    expect(addNode(AT, masked(), 'ellipse', 'n6', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n6', 'mask')] }]);
    // The mask input of the node chosen has a link: the new mask is added loose, and the link stays.
    const taken = addNode(AT, masked(), 'ellipse', 'n4', null);
    expect(taken).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse') }]);
    expect(taken[0]).not.toHaveProperty('links');
    // A node chosen that has no mask input, or that is not there: loose.
    for (const chosen of ['MediaIn1', 'MediaOut1', 'gone']) {
      expect(addNode(AT, comp(), 'rectangle', chosen, null), chosen).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('rectangle') }]);
    }
  });

  it('begin a Polygon with a square of four points about its centre, where the engine makes one without points', () => {
    expect(addNode(AT, comp(), 'polygon', 'n4', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: { ...newNode('polygon'), params: { ...newPolygon(), points: startPoints() } }, links: [link('', 'n4', 'mask')],
    }]);
    expect(startPoints()).toEqual([{ x: -0.2, y: -0.2 }, { x: 0.2, y: -0.2 }, { x: 0.2, y: 0.2 }, { x: -0.2, y: 0.2 }]);
    // Each polygon has a list of its own, and the engine's new one stays empty.
    expect(startPoints()).not.toBe(startPoints());
    expect(newPolygon().points).toEqual([]);
  });

  it('are never put into the chain of the picture, and a node of a picture never into a mask input', () => {
    // A Background has a mask input alone: chosen after a node, it is added loose, as before.
    expect(addNode(AT, comp(), 'background', 'n4', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('background') }]);
    // A Merge after a Merge takes it as its background and feeds what it fed.
    expect(addNode(AT, masked(), 'merge', 'n4', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('merge'), links: [link('n4', '', 'background'), link('', 'MediaOut1', 'input')],
    }]);
    // A Transform after the Rectangle that feeds the mask of the Merge goes between them: its picture is the mask.
    expect(addNode(AT, masked(), 'transform', 'n6', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('transform'), links: [link('n6', '', 'input'), link('', 'n4', 'mask')],
    }]);
  });

  it('are linked, unlinked and taken to another mask input as any link is, and kept in the order of the engine', () => {
    const loose: Composition = { ...comp(), nodes: [...comp().nodes, rectangle('n6')] };
    const linked = connect(AT, loose, link('n6', 'n4', 'mask'));
    expect(linked).toEqual([{ op: 'connect_fusion', ...AT, link: link('n6', 'n4', 'mask') }]);
    // The mask of a node comes after its pictures: background, foreground, input, mask.
    expect(applied(loose, linked).links.map(linkName)).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n6>n4:mask']);
    expect(connect(AT, masked(), link('n6', 'n4', 'mask'))).toEqual([]);
    // Any output goes into a mask input: one that has a link gives it up to the new one.
    const other = applied(masked(), connect(AT, masked(), link('n5', 'n4', 'mask'))).links.map(linkName);
    expect([other.includes('n5>n4:mask'), other.includes('n6>n4:mask')]).toEqual([true, false]);
    expect(disconnect(AT, link('n6', 'n4', 'mask'))).toEqual([{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'mask' }]);
    const moved = reconnect(AT, link('n6', 'n4', 'mask'), { to: 'n3', input: 'mask' });
    expect(moved).toEqual([{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'mask' }, { op: 'connect_fusion', ...AT, link: link('n6', 'n3', 'mask') }]);
    expect(applied(masked(), moved).links.map(linkName)).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n6>n3:mask', 'n3>n4:background', 'n5>n4:foreground']);
  });

  it('are not counted among the pictures of a node that is deleted, but the mask that comes into a mask is what it hands on', () => {
    // The Rectangle n6 masks the Merge, the Ellipse n7 comes into n6, the Rectangle n8 masks the Transform and n9 the Background.
    const all: Composition = {
      nodes: [...comp().nodes, rectangle('n6'), ellipse('n7'), rectangle('n8'), rectangle('n9')],
      links: [...comp().links, link('n6', 'n4', 'mask'), link('n7', 'n6', 'mask'), link('n8', 'n3', 'mask'), link('n9', 'n5', 'mask')],
    };
    // A Transform with a mask is joined across as one without.
    expect(removeNodes(AT, all, ['n3'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4', 'background')] }]);
    // A Background makes its picture of nothing: its mask is not handed on to what it fed.
    expect(removeNodes(AT, all, ['n5'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n5' }]);
    // A mask between two: the one that came into it goes on into what it masked.
    const joined = removeNodes(AT, all, ['n6']);
    expect(joined).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n6', links: [link('n7', 'n4', 'mask')] }]);
    expect(applied(all, joined).links.map(linkName)).toContain('n7>n4:mask');
    expect(removeNodes(AT, all, ['n7'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n7' }]);
    // A Merge with both its pictures is joined to nothing, with a mask or without.
    expect(removeNodes(AT, all, ['n4'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n4' }]);
    expect((['merge', 'transform', 'media_out', 'rectangle', 'background', 'media_in'] as const).map(handedOn))
      .toEqual(['background', 'input', 'input', 'mask', undefined, undefined]);
  });

  it('have their input on the lower edge of a node, which leaves the inputs of its pictures where they were', () => {
    expect([INPUTS.merge, INPUTS.transform, INPUTS.background, INPUTS.rectangle, INPUTS.media_in, INPUTS.media_out])
      .toEqual([['background', 'foreground', 'mask'], ['input', 'mask'], ['mask'], ['mask'], [], ['input']]);
    expect([pictureInputs('transform'), pictureInputs('background'), pictureInputs('polygon')]).toEqual([['input'], [], []]);
    expect((['media_in', 'media_out', 'merge', 'transform', 'background', 'rectangle', 'ellipse', 'polygon'] as const).filter(hasMaskInput))
      .toEqual(['merge', 'transform', 'background', 'rectangle', 'ellipse', 'polygon']);
    const layout = layoutComposition(masked());
    const placed = (id: string) => layout.nodes.find(each => each.node.id === id)!;
    const [three, four, six] = [placed('n3'), placed('n4'), placed('n6')];
    // The Rectangle feeds the Merge through its mask: it lies in a column left of it, as a node that feeds a picture does.
    expect([six.column, four.column]).toEqual([0, 2]);
    // The pictures: spread down the left side as though there were no mask input.
    expect(inputPoint(four, 'background')).toEqual({ x: four.x, y: four.y + NODE_HEIGHT / 3 });
    expect(inputPoint(four, 'foreground')).toEqual({ x: four.x, y: four.y + (NODE_HEIGHT * 2) / 3 });
    expect(inputPoint(three, 'input')).toEqual({ x: three.x, y: three.y + NODE_HEIGHT / 2 });
    // The mask: on the lower edge, a quarter of the way along, of every kind that has one.
    expect(MASK_INSET).toBe(NODE_WIDTH / 4);
    for (const each of [three, four, six]) expect(inputPoint(each, 'mask'), each.node.id).toEqual({ x: each.x + MASK_INSET, y: each.y + NODE_HEIGHT });
    expect(portAt(layout, inputPoint(four, 'mask'))).toEqual({ node: 'n4', side: 'in', input: 'mask' });
    expect(portAt(layout, inputPoint(four, 'foreground'))).toEqual({ node: 'n4', side: 'in', input: 'foreground' });
    const ports = (id: string) => portsOf(layout).filter(each => each.port.node === id).map(each => [each.port.side, each.port.input]);
    expect(ports('n4')).toEqual([['in', 'background'], ['in', 'foreground'], ['in', 'mask'], ['out', null]]);
    expect(ports('n6')).toEqual([['in', 'mask'], ['out', null]]);
    expect([ports('MediaIn1'), ports('MediaOut1')]).toEqual([[['out', null]], [['in', 'input']]]);
    expect(linkEnds(layout, link('n6', 'n4', 'mask'))).toEqual({ from: outputPoint(six), to: inputPoint(four, 'mask') });
  });

  it('have their links drawn into the mask input from below, and every other link as the Color page draws it', () => {
    const from = { x: 0, y: 0 };
    const to = { x: 100, y: 40 };
    expect(fusionLinkPath(from, to, 'input')).toBe(linkPath(from, to));
    expect(fusionLinkPath(from, to, null)).toBe(linkPath(from, to));
    expect(fusionLinkHalves(from, to, 'foreground')).toEqual(linkHalves(from, to));
    // Out of the output to the right, and up into the input: the last handle lies straight below it.
    expect(fusionLinkPath(from, to, 'mask')).toBe('M 0 0 C 50 0, 100 64, 100 40');
    // The halves meet at the middle of that curve.
    expect(fusionLinkHalves(from, to, 'mask')).toEqual(['M 0 0 C 25 0, 50 16, 68.75 29', 'M 68.75 29 C 87.5 42, 100 52, 100 40']);
  });
});

describe('the settings of the masks', () => {
  it('are those of the catalog of the engine, the two numbers of a point written once for every point', () => {
    const catalog: Record<string, Entry[]> = JSON.parse(readFileSync('../engine/crates/model/tests/data/catalog.json', 'utf8'));
    for (const kind of ['rectangle', 'ellipse', 'polygon'] as const) {
      const entries = catalog[`fusion_${kind}`];
      expect(FUSION_SETTINGS[kind].map(setting => ({ path: setting.path, default: setting.bounds.neutral, low: setting.bounds.low, high: setting.bounds.high, unit: setting.unit })), kind)
        .toEqual(entries.map(entry => ({ path: entry.path, default: entry.default, low: entry.low, high: entry.high, unit: entry.unit })));
      expect(entries.every(entry => entry.page === 'fusion' && entry.palette === 'fusion' && entry.clock === 'source_frame')).toBe(true);
    }
    expect(FUSION_SETTINGS.polygon.filter(isPointSetting).map(setting => setting.path)).toEqual(['points.*.x', 'points.*.y']);
    // A new mask has every setting at the default of the catalog; the points of a Polygon are where they were put.
    for (const node of [rectangle('r1'), ellipse('e1'), polygon(startPoints())]) {
      for (const setting of settingsOf(node).filter(each => !isPointSetting(each))) expect(paramOf(node, setting), `${node.kind}.${setting.path}`).toBe(setting.bounds.neutral);
    }
  });

  it('of a Polygon are those of its kind and then the two numbers of each of its points, by the place of the point', () => {
    const shape = polygon(startPoints());
    expect(paths(polygon([]))).toEqual(['level', 'soft_edge', 'border_width', 'center.x', 'center.y', 'size', 'angle']);
    expect(paths(shape).slice(7)).toEqual(['points.0.x', 'points.0.y', 'points.1.x', 'points.1.y', 'points.2.x', 'points.2.y', 'points.3.x', 'points.3.y']);
    // The same list for every polygon of as many points, so that what is drawn from it is not drawn again.
    expect(settingsOf(shape)).toBe(settingsOf(polygon(startPoints(), 'p2')));
    const x2 = settingOf(shape, pointPath(2, 'x'))!;
    expect(x2).toEqual({ path: 'points.2.x', bounds: { neutral: 0, low: -16, high: 16 }, unit: 'widths' });
    expect([isPointSetting(x2), isPointSetting(settingOf(shape, 'soft_edge')!)]).toEqual([true, false]);
    // A point that the polygon has not, the path as the catalog writes it, and the list itself are no settings.
    expect([settingOf(shape, 'points.4.x'), settingOf(shape, 'points.*.x'), settingOf(shape, 'points')]).toEqual([undefined, undefined, undefined]);
    expect([paramOf(shape, x2), pathOf(shape, x2)]).toEqual([0.2, 'p1.points.2.x']);
    // The points are counted from 1 where they are named.
    expect([numbered('Point {n} X', 2), numbered('Point {n}', 0)]).toEqual(['Point 3 X', 'Point 1']);
    expect(hasKeys(shape)).toBe(false);
    expect(hasKeys(polygon(startPoints().map((point, index) => (index === 3 ? { ...point, y: { keys: [keyAt(0, 0.2)] } } : point))))).toBe(true);
    // A mask of any kind has the badge of keys once one of its settings has a key.
    const box = rectangle('r1');
    expect(hasKeys(box)).toBe(false);
    expect(hasKeys(applied(withNode(box), addKey(AT, box, settingOf(box, 'soft_edge')!, 0)).nodes[2])).toBe(true);
  });

  it('are set as the engine takes them: a number of a mask by a patch of its node, a number of a point by its path', () => {
    const box = rectangle('r1');
    const of = (path: string) => settingOf(box, path)!;
    const patch = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'r1', params }];
    expect(putValue(AT, box, of('soft_edge'), 0.1, 12)).toEqual(patch({ soft_edge: 0.1 }));
    expect(putValue(AT, box, of('border_width'), -0.5, 12)).toEqual(patch({ border_width: -0.5 }));
    expect(putValue(AT, box, of('center.y'), 0.25, 12)).toEqual(patch({ center: { y: 0.25 } }));
    expect(putValue(AT, box, of('corner_radius'), 1, 12)).toEqual(patch({ corner_radius: 1 }));
    expect(putValue(AT, box, of('angle'), -90, 12)).toEqual(patch({ angle: -90 }));
    expect(putValue(AT, box, of('width'), 0.5, 12)).toEqual([]);
    expect(addKey(AT, box, of('level'), 12)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'r1.level', at: 0, remove: 0, keys: [keyAt(12, 1)] }]);
    // A patch puts a list in place of a list, whole, and cannot name one point: its number is set by `set_fusion_keys`, which takes a number too.
    const shape = polygon(startPoints());
    const x2 = settingOf(shape, 'points.2.x')!;
    const set = putValue(AT, shape, x2, 0.3, 12);
    expect(set).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'p1.points.2.x', param: 0.3 }]);
    expect(putValue(AT, shape, x2, 0.2, 12)).toEqual([]);
    expect(wholeValue(AT, shape, x2, 0.2, 0.4, 12)).toEqual({ op: 'set_fusion_keys', ...AT, path: 'p1.points.2.x', param: 0.4 });
    expect(wholeParam(AT, shape, x2, 0.2)).toEqual({ op: 'set_fusion_keys', ...AT, path: 'p1.points.2.x', param: 0.2 });
    const keyed = addKey(AT, shape, x2, 12);
    expect(keyed).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'p1.points.2.x', at: 0, remove: 0, keys: [keyAt(12, 0.2)] }]);
    // The copy takes them as the engine does: the one number of the one point, and then its keys.
    const numbered3 = applied(withNode(shape), set).nodes[2];
    expect(paths(numbered3).slice(7).map(path => paramOf(numbered3, settingOf(numbered3, path)!))).toEqual([-0.2, -0.2, 0.2, -0.2, 0.3, 0.2, -0.2, 0.2]);
    const after = applied(withNode(shape), keyed).nodes[2];
    const there = settingOf(after, 'points.2.x')!;
    expect(paramOf(after, there)).toEqual({ keys: [keyAt(12, 0.2)] });
    expect(hasKeys(after)).toBe(true);
    // Its last key taken away leaves its value, as one number.
    expect(removeKey(AT, after, there, 12)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'p1.points.2.x', param: 0.2 }]);
    expect(keyOps(AT, after, there, 0.5)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'p1.points.2.x', param: 0.5 }]);
  });

  it('keep the keys of a point with it where a point is put into the list of points or taken out of it', () => {
    const keys = { keys: [keyAt(0, 0.2), keyAt(20, 0.4)] };
    const points: FusionPoint[] = [{ x: -0.2, y: -0.2 }, { x: keys, y: -0.2 }, { x: 0.2, y: 0.2 }, { x: -0.2, y: 0.2 }];
    const shape = polygon(points);
    // After the last: the list set whole, the new point in the middle of the side that closes the outline, the others the objects that they were.
    const last = insertPoint(AT, shape, null, 0);
    expect(last).toEqual([{ op: 'set_fusion_node', ...AT, id: 'p1', params: { points: [...points, { x: -0.2, y: 0 }] } }]);
    pointsOf(last).slice(0, 4).forEach((point, index) => expect(point, `point ${index}`).toBe(points[index]));
    // After the first, at the frame of the second key of the point after it, where that point is at 0.4: the middle of -0.2 and 0.4.
    const second = insertPoint(AT, shape, 0, 20);
    expect(pointsOf(second)).toEqual([points[0], { x: 0.1, y: -0.2 }, points[1], points[2], points[3]]);
    expect(pointsOf(second)[2]).toBe(points[1]);
    // A place that the polygon has not is after the last.
    expect(pointsOf(insertPoint(AT, shape, 9, 0))).toEqual([...points, { x: -0.2, y: 0 }]);
    // A polygon without points takes one at its centre, and one of one point takes the second beside it.
    expect(pointsOf(insertPoint(AT, polygon([]), null, 0))).toEqual([{ x: 0, y: 0 }]);
    expect(pointsOf(insertPoint(AT, polygon([{ x: 0.1, y: 0.3 }]), 0, 0))).toEqual([{ x: 0.1, y: 0.3 }, { x: 0.2, y: 0.3 }]);
    // A polygon takes 128 points and no more; and only a Polygon has points.
    expect([MOST_POINTS, LEAST_POINTS]).toEqual([128, 3]);
    expect(insertPoint(AT, polygon(Array.from({ length: MOST_POINTS }, (_, index) => ({ x: index / 1000, y: 0 }))), null, 0)).toEqual([]);
    expect(pointsOf(insertPoint(AT, polygon(Array.from({ length: MOST_POINTS - 1 }, (_, index) => ({ x: index / 1000, y: 0 }))), null, 0))).toHaveLength(MOST_POINTS);
    expect(insertPoint(AT, rectangle('r1'), null, 0)).toEqual([]);
    // A point taken away: the list without it, the others the objects that they were.
    const without = removePoint(AT, shape, 0);
    expect(without).toEqual([{ op: 'set_fusion_node', ...AT, id: 'p1', params: { points: [points[1], points[2], points[3]] } }]);
    expect(pointsOf(without)[0]).toBe(points[1]);
    expect([removePoint(AT, shape, 4), removePoint(AT, shape, -1), removePoint(AT, rectangle('r1'), 0)]).toEqual([[], [], []]);
    // In the copy the keys are named by the new place of their point: they were at points.1.x.
    const left = applied(withNode(shape), without).nodes[2];
    expect(paths(left).slice(7)).toEqual(['points.0.x', 'points.0.y', 'points.1.x', 'points.1.y', 'points.2.x', 'points.2.y']);
    expect([paramOf(left, settingOf(left, 'points.0.x')!), paramOf(left, settingOf(left, 'points.1.x')!)]).toEqual([keys, 0.2]);
    const more = applied(withNode(shape), second).nodes[2];
    expect([paramOf(more, settingOf(more, 'points.1.x')!), paramOf(more, settingOf(more, 'points.2.x')!)]).toEqual([0.1, keys]);
    expect(pathOf(more, settingOf(more, 'points.2.x')!)).toBe('p1.points.2.x');
  });
});

// A Blur and a Mosaic (PROTOCOL.md, "Blur and Mosaic"): each has a picture, `input`, and an effect mask.
describe('a Blur and a Mosaic', () => {
  it('are made with every setting, as the engine writes a new one, and have their picture and their mask', () => {
    const base = { label: '', pass_through: false, pos: null };
    expect(newNode('blur', null, 'b1')).toStrictEqual({ id: 'b1', ...base, kind: 'blur', params: { filter: 'gaussian', lock_xy: true, size: { x: 5, y: 5 }, blend: 1 } });
    expect(newNode('mosaic', null, 'm1')).toStrictEqual({
      id: 'm1', ...base, kind: 'mosaic', params: { frequency: 32, aspect: 1, offset: { x: 0, y: 0 }, whole_cells_only: false },
    });
    for (const kind of ['blur', 'mosaic'] as const) {
      expect(INPUTS[kind]).toEqual(['input', 'mask']);
      expect(pictureInputs(kind)).toEqual(['input']);
      expect([hasOutput(kind), hasMaskInput(kind), isMaskKind(kind), handedOn(kind)]).toEqual([true, true, false, 'input']);
    }
  });

  it('have the settings of the catalog of the engine, with its defaults, ranges and units', () => {
    const catalog: Record<string, Entry[]> = JSON.parse(readFileSync('../engine/crates/model/tests/data/catalog.json', 'utf8'));
    for (const kind of ['blur', 'mosaic'] as const) {
      const entries = catalog[`fusion_${kind}`];
      expect(FUSION_SETTINGS[kind].map(setting => ({ path: setting.path, default: setting.bounds.neutral, low: setting.bounds.low, high: setting.bounds.high, unit: setting.unit })), kind)
        .toEqual(entries.map(entry => ({ path: entry.path, default: entry.default, low: entry.low, high: entry.high, unit: entry.unit })));
      expect(entries.every(entry => entry.page === 'fusion' && entry.palette === 'fusion' && entry.clock === 'source_frame')).toBe(true);
    }
    // A new node has every setting at the default of the catalog; what is no number is no setting.
    for (const node of [newNode('blur', null, 'b1'), newNode('mosaic', null, 'm1')]) {
      for (const setting of settingsOf(node)) expect(paramOf(node, setting), `${node.kind}.${setting.path}`).toBe(setting.bounds.neutral);
    }
    expect(paths(newNode('blur', null, 'b1'))).toEqual(['size.x', 'size.y', 'blend']);
    expect(paths(newNode('mosaic', null, 'm1'))).toEqual(['frequency', 'aspect', 'offset.x', 'offset.y']);
    expect(settingOf(newNode('blur', null, 'b1'), 'lock_xy')).toBeUndefined();
    expect(settingOf(newNode('mosaic', null, 'm1'), 'whole_cells_only')).toBeUndefined();
  });

  it('are added in a group of their own: loose where no node is chosen, and after the node chosen on their picture, in the one operation', () => {
    expect(ADDABLE_EFFECTS).toEqual(['blur', 'mosaic', 'delta_keyer', 'tracker', 'planar_tracker']);
    expect([...ADDABLE, ...ADDABLE_MASKS].some(kind => kind === 'blur' || kind === 'mosaic' || kind === 'delta_keyer' || kind === 'tracker')).toBe(false);
    // A Tracker begins with a pattern of its own, which the engine's new one has not: it is held below, with the Tracker.
    for (const kind of ['blur', 'mosaic', 'delta_keyer'] as const) {
      // None chosen: with no links, at the place given, or where the editor lays it out.
      expect(addNode(AT, comp(), kind, null, { x: 10.4, y: 20.6 }), kind).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode(kind, { x: 10, y: 21 }) }]);
      const loose = addNode(AT, comp(), kind, null, null);
      expect(loose, kind).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode(kind) }]);
      expect(loose[0], kind).not.toHaveProperty('links');
      // After the Transform, which feeds the background of the Merge: the Transform into its `input`, and it into the background.
      expect(addNode(AT, comp(), kind, 'n3', null), kind).toEqual([{
        op: 'add_fusion_node', ...AT, node: newNode(kind), links: [link('n3', '', 'input'), link('', 'n4', 'background')],
      }]);
      // After the Merge, before the MediaOut; and after a node that feeds several inputs, before every one of them.
      expect(addNode(AT, comp(), kind, 'n4', null), kind).toEqual([{
        op: 'add_fusion_node', ...AT, node: newNode(kind), links: [link('n4', '', 'input'), link('', 'MediaOut1', 'input')],
      }]);
      const fan: Composition = { nodes: [mediaIn(), mediaOut(), merge('n4')], links: [link('MediaIn1', 'n4', 'background'), link('MediaIn1', 'n4', 'foreground'), link('n4', 'MediaOut1')] };
      expect(addNode(AT, fan, kind, 'MediaIn1', null)[0], kind).toMatchObject({
        links: [link('MediaIn1', '', 'input'), link('', 'n4', 'background'), link('', 'n4', 'foreground')],
      });
      // After the Rectangle that feeds the mask of the Merge it goes between them, as a Transform does: its picture is the mask.
      expect(addNode(AT, masked(), kind, 'n6', null), kind).toEqual([{
        op: 'add_fusion_node', ...AT, node: newNode(kind), links: [link('n6', '', 'input'), link('', 'n4', 'mask')],
      }]);
      // Nothing goes after the MediaOut, nor after a node that is not there: loose.
      for (const chosen of ['MediaOut1', 'gone']) expect(addNode(AT, comp(), kind, chosen, null), `${kind} after ${chosen}`).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode(kind) }]);
    }
    // It is never linked into a mask input, and a mask added with it chosen goes into its effect mask.
    const blurred: Composition = { nodes: [...comp().nodes, newNode('blur', null, 'n6')], links: [...comp().links] };
    expect(addNode(AT, blurred, 'ellipse', 'n6', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n6', 'mask')] }]);
    // In the copy, once the engine has given the node its id: the one operation puts the Blur between the Transform and the Merge.
    const [added] = addNode(AT, comp(), 'blur', 'n3', null) as Extract<Op, { op: 'add_fusion_node' }>[];
    const own = (end: string) => (end === '' ? 'n9' : end);
    const named: Op = { ...added, node: newNode('blur', null, 'n9'), links: (added.links ?? []).map(each => ({ ...each, from: own(each.from), to: own(each.to) })) };
    const after = applied(comp(), [named]);
    expect(after.nodes.map(node => node.id)).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n9']);
    expect(after.links.map(linkName)).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n9>n4:background', 'n5>n4:foreground', 'n3>n9:input']);
    // Taken away again, the Transform feeds the Merge as it did.
    expect(removeNodes(AT, after, ['n9'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n9', links: [link('n3', 'n4', 'background')] }]);
  });

  it('have their numbers set by a patch of the node, which leaves the other number of a pair, and their keys by the path of the setting', () => {
    const blur = newNode('blur', null, 'b1');
    const mosaic = newNode('mosaic', null, 'm1');
    const of = (node: FusionNode, path: string) => settingOf(node, path)!;
    const patch = (id: string, params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id, params }];
    const sized = putValue(AT, blur, of(blur, 'size.x'), 12, 0);
    expect(sized).toEqual(patch('b1', { size: { x: 12 } }));
    expect(applied(withNode(blur), sized).nodes[2].params).toEqual({ filter: 'gaussian', lock_xy: true, size: { x: 12, y: 5 }, blend: 1 });
    expect(putValue(AT, blur, of(blur, 'size.y'), 7.5, 0)).toEqual(patch('b1', { size: { y: 7.5 } }));
    expect(putValue(AT, blur, of(blur, 'blend'), 0.5, 0)).toEqual(patch('b1', { blend: 0.5 }));
    expect(putValue(AT, blur, of(blur, 'size.y'), 5, 0)).toEqual([]);
    expect(putValue(AT, mosaic, of(mosaic, 'frequency'), 64, 0)).toEqual(patch('m1', { frequency: 64 }));
    expect(putValue(AT, mosaic, of(mosaic, 'aspect'), 2, 0)).toEqual(patch('m1', { aspect: 2 }));
    const moved = putValue(AT, mosaic, of(mosaic, 'offset.y'), -40, 0);
    expect(moved).toEqual(patch('m1', { offset: { y: -40 } }));
    expect(applied(withNode(mosaic), moved).nodes[2].params).toEqual({ frequency: 32, aspect: 1, offset: { x: 0, y: -40 }, whole_cells_only: false });
    expect(wholeValue(AT, blur, of(blur, 'size.x'), 5, 20, 0)).toEqual({ op: 'set_fusion_node', ...AT, id: 'b1', params: { size: { x: 20 } } });
    expect(wholeParam(AT, mosaic, of(mosaic, 'offset.x'), 0)).toEqual({ op: 'set_fusion_node', ...AT, id: 'm1', params: { offset: { x: 0 } } });
    // A key is put by the path of the setting, of the value that it has; the last key taken away leaves its value, as one number.
    const keyed = addKey(AT, blur, of(blur, 'size.x'), 12);
    expect(keyed).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'b1.size.x', at: 0, remove: 0, keys: [keyAt(12, 5)] }]);
    expect(addKey(AT, mosaic, of(mosaic, 'offset.y'), 3)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'm1.offset.y', at: 0, remove: 0, keys: [keyAt(3, 0)] }]);
    expect(addKey(AT, mosaic, of(mosaic, 'frequency'), 3)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'm1.frequency', at: 0, remove: 0, keys: [keyAt(3, 32)] }]);
    const after = applied(withNode(blur), keyed).nodes[2];
    expect(paramOf(after, of(after, 'size.x'))).toEqual({ keys: [keyAt(12, 5)] });
    expect([hasKeys(blur), hasKeys(mosaic), hasKeys(after)]).toEqual([false, false, true]);
    expect(putValue(AT, after, of(after, 'size.x'), 8, 12)).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'b1.size.x', at: 0, remove: 1, keys: [keyAt(12, 8)] }]);
    expect(removeKey(AT, after, of(after, 'size.x'), 12)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'b1.size.x', param: 5 }]);
    expect(keyOps(AT, after, of(after, 'size.x'), 9)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'b1.size.x', param: 9 }]);
  });
});

// A Delta Keyer (PROTOCOL.md, "The Delta Keyer"): it has a picture, `input`, a garbage matte, a solid matte and an effect mask.
const keyer = (id: string): FusionNode => newNode('delta_keyer', null, id);
/**
 * MediaIn1 keyed by the Delta Keyer n6 and laid over the Background n5 by the Merge n4, into MediaOut1: the Ellipse n7 is the
 * garbage matte of the keyer, the Rectangle n8 its solid matte and the Rectangle n9 its effect mask.
 */
const keying = (): Composition => ({
  nodes: [mediaIn(), mediaOut(), merge('n4'), background('n5'), keyer('n6'), ellipse('n7'), rectangle('n8'), rectangle('n9')],
  links: [
    link('MediaIn1', 'n6'), link('n7', 'n6', 'garbage_matte'), link('n8', 'n6', 'solid_matte'), link('n9', 'n6', 'mask'),
    link('n5', 'n4', 'background'), link('n6', 'n4', 'foreground'), link('n4', 'MediaOut1'),
  ],
});
/** `comp` and a Delta Keyer, n6, that nothing is linked to or from. */
const looseKeyer = (): Composition => ({ nodes: [...comp().nodes, keyer('n6')], links: comp().links });

describe('a Delta Keyer', () => {
  it('is made with every setting, as the engine writes a new one, and has its picture, its two mattes and its mask', () => {
    const base = { label: '', pass_through: false, pos: null };
    expect(newNode('delta_keyer', null, 'k1')).toStrictEqual({
      id: 'k1', ...base, kind: 'delta_keyer', params: {
        screen: { r: 0, g: 1, b: 0 }, gain: 1, balance: 0.5, pre_blur: 0, low: 0, high: 1, erode_dilate: 0, matte_blur: 0, clean_foreground: 0, clean_background: 0,
        replace: 'soft_color', replace_color: { r: 0.5, g: 0.5, b: 0.5 }, garbage_invert: false, spill_method: 'medium', spill_suppression: 1, view: 'final_result',
      },
    });
    expect(newNode('delta_keyer').params).toStrictEqual(newDeltaKeyer());
    // The fields of its settings, in the order that the engine writes them in; and each new one has colours of its own.
    expect(Object.keys(newDeltaKeyer())).toEqual([
      'screen', 'gain', 'balance', 'pre_blur', 'low', 'high', 'erode_dilate', 'matte_blur', 'clean_foreground', 'clean_background',
      'replace', 'replace_color', 'garbage_invert', 'spill_method', 'spill_suppression', 'view',
    ]);
    expect(newDeltaKeyer().screen).not.toBe(newDeltaKeyer().screen);
    expect(INPUTS.delta_keyer).toEqual(['input', 'garbage_matte', 'solid_matte', 'mask']);
    // Its one picture is its `input`: a matte is no picture, as its effect mask is none.
    expect(pictureInputs('delta_keyer')).toEqual(['input']);
    expect(matteInputs('delta_keyer')).toEqual(['garbage_matte', 'solid_matte']);
    expect([hasOutput('delta_keyer'), hasMaskInput('delta_keyer'), isMaskKind('delta_keyer'), handedOn('delta_keyer')]).toEqual([true, true, false, 'input']);
    expect([...MATTE_INPUTS]).toEqual(['garbage_matte', 'solid_matte']);
    expect((['input', 'background', 'foreground', 'mask', 'garbage_matte', 'solid_matte'] as const).map(input => [isMatteInput(input), isPictureInput(input)]))
      .toEqual([[false, true], [false, true], [false, true], [false, false], [true, false], [true, false]]);
    expect(isMatteInput(null)).toBe(false);
    // No other kind has a matte, and the pictures of the other kinds are what they were.
    expect((Object.keys(INPUTS) as (keyof typeof INPUTS)[]).filter(kind => matteInputs(kind).length > 0)).toEqual(['delta_keyer']);
    expect([pictureInputs('merge'), pictureInputs('blur'), pictureInputs('rectangle'), pictureInputs('media_out')]).toEqual([['background', 'foreground'], ['input'], [], ['input']]);
  });

  it('has the settings of the catalog of the engine, with its defaults, ranges and units', () => {
    const catalog: Record<string, Entry[]> = JSON.parse(readFileSync('../engine/crates/model/tests/data/catalog.json', 'utf8'));
    const entries = catalog.fusion_delta_keyer;
    expect(entries).toHaveLength(16);
    expect(FUSION_SETTINGS.delta_keyer.map(setting => ({ path: setting.path, default: setting.bounds.neutral, low: setting.bounds.low, high: setting.bounds.high, unit: setting.unit })))
      .toEqual(entries.map(entry => ({ path: entry.path, default: entry.default, low: entry.low, high: entry.high, unit: entry.unit })));
    expect(entries.every(entry => entry.page === 'fusion' && entry.palette === 'fusion' && entry.clock === 'source_frame')).toBe(true);
    // A new keyer has every number at the default of the catalog; its modes, its switch and a colour whole are no settings.
    const node = keyer('k1');
    for (const setting of settingsOf(node)) expect(paramOf(node, setting), setting.path).toBe(setting.bounds.neutral);
    expect(paths(node)).toEqual([
      'screen.r', 'screen.g', 'screen.b', 'gain', 'balance', 'pre_blur', 'low', 'high', 'erode_dilate', 'matte_blur', 'clean_foreground', 'clean_background',
      'replace_color.r', 'replace_color.g', 'replace_color.b', 'spill_suppression',
    ]);
    for (const path of ['replace', 'garbage_invert', 'spill_method', 'view', 'screen', 'replace_color']) expect(settingOf(node, path), path).toBeUndefined();
    // Its lengths are counted as a Blur Size is, and a channel of a colour may be as much as a half float holds.
    expect(['pre_blur', 'erode_dilate', 'matte_blur'].map(path => settingOf(node, path)!.unit)).toEqual(['pixels1080', 'pixels1080', 'pixels1080']);
    expect(settingOf(node, 'screen.g')).toEqual({ path: 'screen.g', bounds: { neutral: 1, low: 0, high: 65504 }, unit: 'number' });
    expect(settingOf(node, 'erode_dilate')!.bounds).toEqual({ neutral: 0, low: -100, high: 100 });
  });

  it('is added in the group of the effects: loose where no node is chosen, and after the node chosen on its picture, never on a matte', () => {
    expect(ADDABLE_EFFECTS).toContain('delta_keyer');
    expect(addNode(AT, comp(), 'delta_keyer', null, { x: 10.4, y: 20.6 })).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer', { x: 10, y: 21 }) }]);
    // After the Transform, which feeds the background of the Merge: the Transform into its `input`, and it into the background.
    expect(addNode(AT, comp(), 'delta_keyer', 'n3', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('delta_keyer'), links: [link('n3', '', 'input'), link('', 'n4', 'background')],
    }]);
    // A Blur after the keyer goes between it and the foreground that it feeds.
    expect(addNode(AT, keying(), 'blur', 'n6', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('blur'), links: [link('n6', '', 'input'), link('', 'n4', 'foreground')],
    }]);
    // A Transform after the Ellipse that feeds the garbage matte goes between them: its picture is the matte.
    expect(addNode(AT, keying(), 'transform', 'n7', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: newNode('transform'), links: [link('n7', '', 'input'), link('', 'n6', 'garbage_matte')],
    }]);
    // A mask added with the keyer chosen goes into its effect mask where that is free, though its mattes are free too; and loose where it is not.
    expect(addNode(AT, looseKeyer(), 'rectangle', 'n6', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('rectangle'), links: [link('', 'n6', 'mask')] }]);
    const unmasked: Composition = { ...keying(), links: keying().links.filter(each => each.input !== 'mask') };
    expect(addNode(AT, unmasked, 'ellipse', 'n6', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n6', 'mask')] }]);
    const unmatted: Composition = { ...keying(), links: keying().links.filter(each => !isMatteInput(each.input)) };
    const taken = addNode(AT, unmatted, 'ellipse', 'n6', null);
    expect(taken).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse') }]);
    expect(taken[0]).not.toHaveProperty('links');
  });

  it('has its mattes linked, unlinked and taken to the other matte as any link is, and kept in the order of the engine', () => {
    // The inputs of a node by their names: the garbage matte before the picture, and the solid matte after the mask.
    expect(layoutComposition(keying()).links.map(linkName)).toEqual([
      'n4>MediaOut1:input', 'n5>n4:background', 'n6>n4:foreground', 'n7>n6:garbage_matte', 'MediaIn1>n6:input', 'n9>n6:mask', 'n8>n6:solid_matte',
    ]);
    const unmatted: Composition = { ...keying(), links: keying().links.filter(each => !isMatteInput(each.input)) };
    const linked = connect(AT, unmatted, link('n7', 'n6', 'garbage_matte'));
    expect(linked).toEqual([{ op: 'connect_fusion', ...AT, link: link('n7', 'n6', 'garbage_matte') }]);
    expect(applied(unmatted, linked).links.map(linkName)).toEqual([
      'n4>MediaOut1:input', 'n5>n4:background', 'n6>n4:foreground', 'n7>n6:garbage_matte', 'MediaIn1>n6:input', 'n9>n6:mask',
    ]);
    // The link that is there already sends nothing; any output goes into a matte, and one that has a link gives it up to the new one.
    expect(connect(AT, keying(), link('n7', 'n6', 'garbage_matte'))).toEqual([]);
    const other = applied(keying(), connect(AT, keying(), link('n5', 'n6', 'solid_matte'))).links.map(linkName);
    expect([other.includes('n5>n6:solid_matte'), other.includes('n8>n6:solid_matte')]).toEqual([true, false]);
    expect(disconnect(AT, link('n8', 'n6', 'solid_matte'))).toEqual([{ op: 'disconnect_fusion', ...AT, to: 'n6', input: 'solid_matte' }]);
    // The end of the link of the garbage matte taken to the solid matte: the old link goes and the new one comes.
    const moved = reconnect(AT, link('n7', 'n6', 'garbage_matte'), { to: 'n6', input: 'solid_matte' });
    expect(moved).toEqual([{ op: 'disconnect_fusion', ...AT, to: 'n6', input: 'garbage_matte' }, { op: 'connect_fusion', ...AT, link: link('n7', 'n6', 'solid_matte') }]);
    const after = applied(keying(), moved).links.map(linkName);
    expect([after.includes('n7>n6:solid_matte'), after.includes('n8>n6:solid_matte'), after.includes('n7>n6:garbage_matte')]).toEqual([true, false, false]);
  });

  it('is joined across by its picture alone where it is deleted: its mattes and its mask are not counted', () => {
    // With all four of its inputs linked, one link comes into its picture: the MediaIn is linked to the foreground that it fed.
    const removed = removeNodes(AT, keying(), ['n6']);
    expect(removed).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n6', links: [link('MediaIn1', 'n4', 'foreground')] }]);
    expect(applied(keying(), removed).links.map(linkName)).toEqual(['n4>MediaOut1:input', 'n5>n4:background', 'MediaIn1>n4:foreground']);
    // With its mattes linked and its picture not, what it fed is left without a picture: a matte is not handed on.
    const unfed: Composition = { ...keying(), links: keying().links.filter(each => each.to !== 'n6' || each.input !== 'input') };
    expect(removeNodes(AT, unfed, ['n6'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n6' }]);
    // A mask that feeds a matte, which nothing feeds, has nothing to join; a Transform between the two is joined across, into the matte.
    expect(removeNodes(AT, keying(), ['n7'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n7' }]);
    const through: Composition = {
      nodes: [...keying().nodes, transform('n10')],
      links: [...keying().links.filter(each => each.input !== 'garbage_matte'), link('n7', 'n10'), link('n10', 'n6', 'garbage_matte')],
    };
    expect(removeNodes(AT, through, ['n10'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n10', links: [link('n7', 'n6', 'garbage_matte')] }]);
  });

  it('takes a node dropped on the link into its picture, and none on a link into a matte or into its mask', () => {
    const withBlur: Composition = { nodes: [...keying().nodes, newNode('blur', null, 'n10')], links: keying().links };
    expect(insertOnLink(AT, withBlur, 'n10', link('n7', 'n6', 'garbage_matte'))).toEqual([]);
    expect(insertOnLink(AT, withBlur, 'n10', link('n8', 'n6', 'solid_matte'))).toEqual([]);
    expect(insertOnLink(AT, withBlur, 'n10', link('n9', 'n6', 'mask'))).toEqual([]);
    expect(insertOnLink(AT, withBlur, 'n10', link('MediaIn1', 'n6'))).toEqual([
      { op: 'connect_fusion', ...AT, link: link('MediaIn1', 'n10', 'input') }, { op: 'connect_fusion', ...AT, link: link('n10', 'n6', 'input') },
    ]);
    // A keyer that has no links goes into a link of a picture by its `input`; one that has a link, be it into a matte alone, does not.
    expect(insertOnLink(AT, looseKeyer(), 'n6', link('n3', 'n4', 'background'))).toEqual([
      { op: 'connect_fusion', ...AT, link: link('n3', 'n6', 'input') }, { op: 'connect_fusion', ...AT, link: link('n6', 'n4', 'background') },
    ]);
    const matted: Composition = { nodes: [...looseKeyer().nodes, ellipse('n7')], links: [...looseKeyer().links, link('n7', 'n6', 'solid_matte')] };
    expect(insertOnLink(AT, matted, 'n6', link('n3', 'n4', 'background'))).toEqual([]);
  });

  it('has its matte inputs on the upper edge of its card, which leaves its picture, its mask and its output where those of a Blur are', () => {
    const layout = layoutComposition(keying());
    const placed = (id: string) => layout.nodes.find(each => each.node.id === id)!;
    const six = placed('n6');
    // What feeds a matte lies in a column left of the keyer, as what feeds its picture does.
    expect([placed('n7').column, placed('n8').column, placed('n9').column, six.column, placed('n4').column]).toEqual([0, 0, 0, 1, 2]);
    expect(inputPoint(six, 'input')).toEqual({ x: six.x, y: six.y + NODE_HEIGHT / 2 });
    expect(inputPoint(six, 'mask')).toEqual({ x: six.x + MASK_INSET, y: six.y + NODE_HEIGHT });
    expect(outputPoint(six)).toEqual({ x: six.x + NODE_WIDTH, y: six.y + NODE_HEIGHT / 2 });
    // The garbage matte a quarter of the way along the upper edge, over the mask input, and the solid matte three quarters.
    expect(MATTE_INSETS).toEqual({ garbage_matte: NODE_WIDTH / 4, solid_matte: (NODE_WIDTH * 3) / 4 });
    expect(inputPoint(six, 'garbage_matte')).toEqual({ x: six.x + NODE_WIDTH / 4, y: six.y });
    expect(inputPoint(six, 'solid_matte')).toEqual({ x: six.x + (NODE_WIDTH * 3) / 4, y: six.y });
    const ports = (id: string) => portsOf(layout).filter(each => each.port.node === id).map(each => [each.port.side, each.port.input]);
    expect(ports('n6')).toEqual([['in', 'input'], ['in', 'garbage_matte'], ['in', 'solid_matte'], ['in', 'mask'], ['out', null]]);
    expect(portAt(layout, inputPoint(six, 'garbage_matte'))).toEqual({ node: 'n6', side: 'in', input: 'garbage_matte' });
    expect(portAt(layout, inputPoint(six, 'solid_matte'))).toEqual({ node: 'n6', side: 'in', input: 'solid_matte' });
    expect(linkEnds(layout, link('n8', 'n6', 'solid_matte'))).toEqual({ from: outputPoint(placed('n8')), to: inputPoint(six, 'solid_matte') });
  });

  it('has the links into its mattes drawn into them from above, and found on that curve', () => {
    const from = { x: 0, y: 0 };
    const to = { x: 100, y: 40 };
    // Out of the output to the right, and down into the input: the last handle lies straight above it.
    expect(fusionLinkPath(from, to, 'garbage_matte')).toBe('M 0 0 C 50 0, 100 16, 100 40');
    expect(fusionLinkPath(from, to, 'solid_matte')).toBe(fusionLinkPath(from, to, 'garbage_matte'));
    expect(fusionLinkHalves(from, to, 'solid_matte')).toEqual(['M 0 0 C 25 0, 50 4, 68.75 11', 'M 68.75 11 C 87.5 18, 100 28, 100 40']);
    // A link into a mask comes from below still, and a link of a picture is as the Color page draws it.
    expect(fusionLinkPath(from, to, 'mask')).toBe('M 0 0 C 50 0, 100 64, 100 40');
    expect(fusionLinkPath(from, to, 'input')).toBe(linkPath(from, to));
    const layout = layoutComposition(keying());
    const matte = link('n7', 'n6', 'garbage_matte');
    const ends = linkEnds(layout, matte)!;
    const [x, y] = fusionLinkHalves(ends.from, ends.to, matte.input)[0].split(', ').at(-1)!.split(' ').map(Number);
    expect(fusionLinkNear(layout, { x, y })).toEqual(matte);
    // The editor asks for the links of pictures alone where a node is dragged over a link: a matte is none.
    expect(fusionLinkNear(layout, { x, y }, each => isPictureInput(each.input))).toBeNull();
  });

  it('has a number set by a patch of the node and keyed by its path, and a colour set as one by one patch', () => {
    const node = keyer('k1');
    const of = (path: string) => settingOf(node, path)!;
    const patch = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'k1', params }];
    expect(putValue(AT, node, of('gain'), 2.5, 0)).toEqual(patch({ gain: 2.5 }));
    expect(putValue(AT, node, of('replace_color.b'), 0.2, 0)).toEqual(patch({ replace_color: { b: 0.2 } }));
    expect(putValue(AT, node, of('erode_dilate'), -3.5, 0)).toEqual(patch({ erode_dilate: -3.5 }));
    expect(putValue(AT, node, of('high'), 1, 0)).toEqual([]);
    expect(pathOf(node, of('replace_color.b'))).toBe('k1.replace_color.b');
    const keyed = addKey(AT, node, of('screen.g'), 12);
    expect(keyed).toEqual([{ op: 'splice_fusion_keys', ...AT, path: 'k1.screen.g', at: 0, remove: 0, keys: [keyAt(12, 1)] }]);
    const after = applied(withNode(node), keyed).nodes[2];
    expect([hasKeys(node), hasKeys(after)]).toEqual([false, true]);
    expect(removeKey(AT, after, of('screen.g'), 12)).toEqual([{ op: 'set_fusion_keys', ...AT, path: 'k1.screen.g', param: 1 }]);
    // A colour as one, from green to blue: the two channels that change in one patch, so that the engine never sees the grey between them.
    const colour = (red: number, green: number, blue: number) => [
      { setting: of('screen.r'), value: red }, { setting: of('screen.g'), value: green }, { setting: of('screen.b'), value: blue },
    ];
    const blue = putValues(AT, node, colour(0, 0, 1), 0);
    expect(blue).toEqual(patch({ screen: { g: 0, b: 1 } }));
    const set = applied(withNode(node), blue).nodes[2];
    expect(['screen.r', 'screen.g', 'screen.b'].map(path => paramOf(set, of(path)))).toEqual([0, 0, 1]);
    expect(putValues(AT, node, colour(0, 1, 0), 0)).toEqual([]);
    // A channel that has keys takes its value as a key at the frame, by its path, after the patch of the others.
    expect(putValues(AT, after, colour(0, 0, 1), 12)).toEqual([
      ...patch({ screen: { b: 1 } }), { op: 'splice_fusion_keys', ...AT, path: 'k1.screen.g', at: 0, remove: 1, keys: [keyAt(12, 0)] },
    ]);
    expect(putValues(AT, after, colour(0, 0, 0), 20)).toEqual([
      { op: 'splice_fusion_keys', ...AT, path: 'k1.screen.g', at: 1, remove: 0, keys: [keyAt(20, 0)] },
    ]);
    // The two refusals that are of no one number are named by their reason and what the engine says they are of.
    expect(keyerRefusals({ keyerScreen: 'screen', keyerThreshold: 'threshold' }))
      .toEqual({ 'out_of_range:delta_keyer.screen': 'screen', 'out_of_range:delta_keyer.low': 'threshold' });
  });
});

// Copy, Cut, Paste and Duplicate, and a node dropped onto a link: done with the operations that the node editor has already.
type Added = Extract<Op, { op: 'add_fusion_node' }>;
/**
 * Operations as the engine does them (PROTOCOL.md, "Names of a transaction"): a node added by a name of the transaction is given
 * the next id, and the links that name it, its own and those of the nodes added after it, name that id.
 */
const done = (ops: Op[], next = 20): Op[] => {
  const ids = new Map<string, string>();
  const real = (end: string) => ids.get(end) ?? end;
  return ops.map((op): Op => {
    if (op.op !== 'add_fusion_node') return op;
    const id = `n${next + ids.size}`;
    ids.set(op.node.id, id);
    return { ...op, node: { ...op.node, id } as FusionNode, ...(op.links ? { links: op.links.map(each => ({ ...each, from: real(each.from), to: real(each.to) })) } : {}) };
  });
};
const blendKeys = { keys: [keyAt(10, 0.25), keyAt(30, 0.75)] };
/** `comp` with the Transform passed through, the Merge labelled and its Blend keyed, and the Background dragged to 300,200. */
const rich = (): Composition => {
  const made = comp();
  return {
    ...made,
    nodes: made.nodes.map((node): FusionNode => {
      if (node.id === 'n3') return { ...node, pass_through: true };
      if (node.id === 'n4') return { ...node, label: 'Over the sky', params: { ...newMerge(), blend: blendKeys } } as FusionNode;
      if (node.id === 'n5') return { ...node, pos: { x: 300, y: 200 } };
      return node;
    }),
  };
};

describe('copying, cutting, pasting and duplicating nodes', () => {
  it('copy the nodes chosen whole, where the editor draws them, with the links among them, and never the MediaOut', () => {
    const from = rich();
    const copied = copyNodes(from, ['n5', 'MediaOut1', 'n4', 'gone', 'n3'])!;
    // In the order of the composition, each at the place where it is drawn: where the editor lays it out, or where it was dragged to.
    expect(copied.nodes.map(node => [node.id, node.pos])).toEqual([['n3', { x: 176, y: 28 }], ['n4', { x: 324, y: 28 }], ['n5', { x: 300, y: 200 }]]);
    // Whole: Pass Through, the label, the settings and their keys.
    expect(copied.nodes[0]).toEqual({ ...from.nodes[2], pos: { x: 176, y: 28 } });
    expect(copied.nodes[1]).toEqual({ ...from.nodes[3], pos: { x: 324, y: 28 } });
    expect([copied.nodes[0].pass_through, copied.nodes[1].label, (copied.nodes[1].params as unknown as { blend: unknown }).blend]).toEqual([true, 'Over the sky', blendKeys]);
    // A copy of its own: what is done to the composition afterwards is not done to it.
    expect(copied.nodes[1].params).not.toBe(from.nodes[3].params);
    // The links that both begin and end among them, in the order of the engine; those to the MediaIn and the MediaOut are left.
    expect(copied.links).toEqual([link('n3', 'n4', 'background'), link('n5', 'n4', 'foreground')]);
    expect(copyNodes(from, ['n4'])!.links).toEqual([]);
    // A MediaIn may be copied; the MediaOut never is, alone or among others; and nothing chosen is nothing to copy.
    expect(copyNodes(from, ['MediaIn1', 'MediaOut1'])!.nodes.map(node => node.kind)).toEqual(['media_in']);
    expect([copyNodes(from, ['MediaOut1']), copyNodes(from, []), copyNodes(from, ['gone'])]).toEqual([null, null, null]);
  });

  it('paste them as new nodes down and right of where they were, linked among themselves and to nothing else', () => {
    const from = rich();
    const copied = copyNodes(from, ['n3', 'n4', 'n5'])!;
    const ops = pasteNodes(AT, from, copied);
    expect([PASTE_OFFSET, pastedName(2)]).toEqual([{ x: 24, y: 24 }, '$p2']);
    // Each is named by the transaction, and a link goes with the later of the two nodes that it joins.
    expect(ops).toEqual([
      { op: 'add_fusion_node', ...AT, node: { ...copied.nodes[0], id: '$p0', pos: { x: 200, y: 52 } } },
      { op: 'add_fusion_node', ...AT, node: { ...copied.nodes[1], id: '$p1', pos: { x: 348, y: 52 } }, links: [link('$p0', '$p1', 'background')] },
      { op: 'add_fusion_node', ...AT, node: { ...copied.nodes[2], id: '$p2', pos: { x: 324, y: 224 } }, links: [link('$p2', '$p1', 'foreground')] },
    ]);
    expect(ops[0]).not.toHaveProperty('links');
    // What was copied is as it was, to be pasted again.
    expect(copied.nodes.map(node => [node.id, node.pos])).toEqual([['n3', { x: 176, y: 28 }], ['n4', { x: 324, y: 28 }], ['n5', { x: 300, y: 200 }]]);
    // As the engine does them: three nodes more, the two links among them, and no link to what was there.
    const made = done(ops);
    expect(addedNodes(made)).toEqual(['n20', 'n21', 'n22']);
    expect([addedNodes(null), addedNodes([{ op: 'set_fusion', ...AT, value: null }])]).toEqual([[], []]);
    const after = applied(from, made);
    expect(after.nodes.map(node => node.id)).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n20', 'n21', 'n22']);
    expect(after.links.map(linkName).sort()).toEqual([...from.links.map(linkName), 'n20>n21:background', 'n22>n21:foreground'].sort());
    // Pass Through, the label, the settings and the keys are kept: the keys as they are, at the frames that they counted.
    expect([after.nodes[5].pass_through, after.nodes[6].label, after.nodes[6].params, after.nodes[7].params])
      .toEqual([true, 'Over the sky', from.nodes[3].params, from.nodes[4].params]);
    // Pasted again into what that left, they lie further down and right, clear of the ones pasted before.
    expect(pasteNodes(AT, after, copied).map(op => (op as Added).node.pos)).toEqual([{ x: 224, y: 76 }, { x: 372, y: 76 }, { x: 348, y: 248 }]);
    // At a point, from the menu: the corner of the box about them lies there, the others as they lay about it.
    expect(pasteNodes(AT, from, copied, { x: 500.4, y: 300.6 }).map(op => (op as Added).node.pos)).toEqual([{ x: 500, y: 301 }, { x: 648, y: 301 }, { x: 624, y: 473 }]);
    // Into the composition of another clip, which has none yet: the same nodes, after the first composition.
    const other = { timeline: 't1', clip: 'c2' };
    const into = pasteNodes(other, firstComposition(), copied);
    expect(into.map(op => [(op as Added).clip, (op as Added).node.id, (op as Added).node.pos]))
      .toEqual([['c2', '$p0', { x: 200, y: 52 }], ['c2', '$p1', { x: 348, y: 52 }], ['c2', '$p2', { x: 324, y: 224 }]]);
    expect((into[1] as Added).node.params).toEqual(from.nodes[3].params);
    expect(withFirst(other, false, into)[0]).toEqual({ op: 'set_fusion', ...other, value: firstComposition() });
    // Nothing copied is nothing to paste.
    expect(pasteNodes(AT, from, null)).toEqual([]);
  });

  it('paste nothing into a composition that has no room for it: 256 nodes at the most', () => {
    expect(MOST_NODES).toBe(256);
    const full: Composition = {
      nodes: [mediaIn(), mediaOut(), ...Array.from({ length: MOST_NODES - 3 }, (_, index) => transform(`t${index}`))], links: [link('MediaIn1', 'MediaOut1')],
    };
    expect(full.nodes).toHaveLength(255);
    expect([roomFor(full, 1), roomFor(full, 2), roomFor(comp(), 251), roomFor(comp(), 252)]).toEqual([true, false, true, false]);
    expect(pasteNodes(AT, full, copyNodes(full, ['t0']))).toHaveLength(1);
    expect(pasteNodes(AT, full, copyNodes(full, ['t0', 't1']))).toEqual([]);
    expect(duplicateNodes(AT, full, ['t0', 't1'])).toEqual([]);
    expect(duplicateNodes(AT, full, ['t1'])).toHaveLength(1);
  });

  it('cut as Delete takes nodes away, keeping what Copy keeps, and duplicate as Copy and Paste in one', () => {
    const cut = cutNodes(AT, comp(), ['n3', 'MediaOut1']);
    expect(cut.copied!.nodes.map(node => node.id)).toEqual(['n3']);
    expect(cut.ops).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4', 'background')] }]);
    // Two of a chain: kept with the link between them, and taken away in the order of the composition.
    const two = cutNodes(AT, comp(), ['n4', 'n3']);
    expect([two.copied!.links, two.ops]).toEqual([[link('n3', 'n4', 'background')], removeNodes(AT, comp(), ['n3', 'n4'])]);
    // The MediaOut is neither kept nor taken away.
    expect(cutNodes(AT, comp(), ['MediaOut1'])).toEqual({ copied: null, ops: [] });
    const twice = duplicateNodes(AT, comp(), ['n3', 'n4']);
    expect(twice).toEqual(pasteNodes(AT, comp(), copyNodes(comp(), ['n3', 'n4'])));
    expect(twice.map(op => [(op as Added).node.id, (op as Added).node.kind, (op as Added).node.pos, (op as Added).links])).toEqual([
      ['$p0', 'transform', { x: 200, y: 52 }, undefined], ['$p1', 'merge', { x: 348, y: 52 }, [link('$p0', '$p1', 'background')]],
    ]);
    expect([duplicateNodes(AT, comp(), ['MediaOut1']), duplicateNodes(AT, comp(), [])]).toEqual([[], []]);
  });

  it('are asked for by Ctrl or Cmd with C, X, V and D, and Select Tool by Shift+Space', () => {
    for (const [letter, kind] of [['c', 'copy'], ['x', 'cut'], ['v', 'paste'], ['d', 'duplicate']] as const) {
      const code = `Key${letter.toUpperCase()}`;
      expect(fusionKey(key({ key: letter, code, ctrl: true })), letter).toEqual({ kind });
      expect(fusionKey(key({ key: letter, code, meta: true })), letter).toEqual({ kind });
      // Not with both, with Shift or Alt more, held down, or alone.
      for (const more of [{ ctrl: true, meta: true }, { ctrl: true, shift: true }, { ctrl: true, alt: true }, { ctrl: true, repeat: true }, {}]) {
        expect(fusionKey(key({ key: letter, code, ...more })), `${letter} ${JSON.stringify(more)}`).toBeNull();
      }
    }
    // Pass Through and Select All stay with Ctrl alone.
    expect([fusionKey(key({ key: 'p', code: 'KeyP', meta: true })), fusionKey(key({ key: 'a', code: 'KeyA', meta: true }))]).toEqual([null, null]);
    expect(fusionKey(key({ key: ' ', code: 'Space', shift: true }))).toEqual({ kind: 'selectTool' });
    // Space alone is the transport's; and Shift+Space is taken once, with nothing else held.
    expect(fusionKey(key({ key: ' ', code: 'Space' }))).toBeNull();
    for (const more of [{ ctrl: true }, { meta: true }, { alt: true }, { repeat: true }, { composing: true }]) {
      expect(fusionKey(key({ key: ' ', code: 'Space', shift: true, ...more })), JSON.stringify(more)).toBeNull();
    }
  });
});

describe('a node dropped onto a link', () => {
  /**
   * `comp`, and nodes that nothing is linked to or from: the Blur n6, the Merge n7, the Rectangle n8 and the Background n10; and
   * the Rectangle n9, into the mask of the Merge n4.
   */
  const loose = (): Composition => ({
    nodes: [...comp().nodes, newNode('blur', null, 'n6'), merge('n7'), rectangle('n8'), rectangle('n9'), background('n10')],
    links: [...comp().links, link('n9', 'n4', 'mask')],
  });

  it('goes into the link: what fed the link feeds its first picture, and it feeds what the link fed', () => {
    const ops = insertOnLink(AT, loose(), 'n6', link('n3', 'n4', 'background'));
    expect(ops).toEqual([{ op: 'connect_fusion', ...AT, link: link('n3', 'n6', 'input') }, { op: 'connect_fusion', ...AT, link: link('n6', 'n4', 'background') }]);
    // In the copy: the second link takes the place of the one that the node was dropped on.
    expect(applied(loose(), ops).links.map(linkName).sort())
      .toEqual(['MediaIn1>n3:input', 'n3>n6:input', 'n6>n4:background', 'n5>n4:foreground', 'n4>MediaOut1:input', 'n9>n4:mask'].sort());
    // A Merge takes what fed the link as its background.
    expect(insertOnLink(AT, loose(), 'n7', link('n4', 'MediaOut1'))).toEqual([
      { op: 'connect_fusion', ...AT, link: link('n4', 'n7', 'background') }, { op: 'connect_fusion', ...AT, link: link('n7', 'MediaOut1') },
    ]);
  });

  it('does not go into a link into a mask, nor where it has a link itself, no input of a picture or no output', () => {
    expect(insertOnLink(AT, loose(), 'n6', link('n9', 'n4', 'mask'))).toEqual([]);
    // A mask, a Background and a MediaIn have no input of a picture, and the MediaOut has no output.
    for (const id of ['n8', 'n10', 'MediaIn1', 'MediaOut1']) expect(insertOnLink(AT, loose(), id, link('n3', 'n4', 'background')), id).toEqual([]);
    // A node that has a link, be it one into its effect mask alone.
    expect(insertOnLink(AT, loose(), 'n3', link('n5', 'n4', 'foreground'))).toEqual([]);
    const maskedBlur: Composition = { ...loose(), links: [...loose().links, link('n8', 'n6', 'mask')] };
    expect(insertOnLink(AT, maskedBlur, 'n6', link('n3', 'n4', 'background'))).toEqual([]);
    // A link that the composition has not, and a node that it has not.
    expect(insertOnLink(AT, loose(), 'n6', link('n5', 'n4', 'background'))).toEqual([]);
    expect(insertOnLink(AT, loose(), 'gone', link('n3', 'n4', 'background'))).toEqual([]);
  });

  it('is over a link where the pointer is near the curve of the link, as in the Color page\'s editor', () => {
    const layout = layoutComposition(loose());
    /** The middle of the curve of a link, where its two halves meet. */
    const middleOf = (of: FusionLink) => {
      const ends = linkEnds(layout, of)!;
      const [x, y] = fusionLinkHalves(ends.from, ends.to, of.input)[0].split(', ').at(-1)!.split(' ').map(Number);
      return { x, y };
    };
    const picture = link('n3', 'n4', 'background');
    const mask = link('n9', 'n4', 'mask');
    expect(fusionLinkNear(layout, middleOf(picture))).toEqual(picture);
    expect(fusionLinkNear(layout, { x: middleOf(picture).x + 2, y: middleOf(picture).y - 2 })).toEqual(picture);
    // Out of reach of every link: none.
    expect(fusionLinkNear(layout, { x: middleOf(picture).x, y: middleOf(picture).y - 20 })).toBeNull();
    expect(fusionLinkNear(layout, { x: 600, y: 400 })).toBeNull();
    // A link into a mask is found on its own curve, which comes into the input from below; the editor asks for the links of pictures alone.
    expect(fusionLinkNear(layout, middleOf(mask))).toEqual(mask);
    expect(fusionLinkNear(layout, middleOf(mask), each => each.input !== 'mask')).toBeNull();
    expect(fusionLinkNear(layout, middleOf(picture), each => each.input !== 'background')).toBeNull();
  });
});

// A Tracker (PROTOCOL.md, "The Tracker" and "Following a Tracker"): its patterns, each with its path, and the centres that follow it.
interface OpsExample { name: string; done: Op }
/** A pattern put at a quarter of the width and half the height, named, with a path of two frames: from where it was put at frame 0 to 0.75, 0.25 at frame 8. */
const left = (): FusionTrackerPattern => ({ ...newPattern(0.25, 0.5), name: 'Left', path: { 0: [0.25, 0.5], 8: [0.75, 0.25] } });
/** A Tracker of these patterns, the node `id`, with what else of it is given. */
const trackerNode = (trackers: FusionTrackerPattern[], id = 'n6', more: Partial<FusionTracker> = {}): FusionNode => (
  { ...newNode('tracker', null, id), params: { ...newTracker(), ...more, trackers } } as FusionNode);
/** The place of the pattern `tracker` of the node `node`, as a centre follows it. */
const ofPattern = (tracker: number, node = 'n6'): FusionFollow => ({ node, output: 'tracker', tracker });
/** The patterns that an operation sets a Tracker to, whole. */
const listOf = (ops: Op[], at = 0): FusionTrackerPattern[] => ((ops[at] as Extract<Op, { op: 'set_fusion_node' }>).params as unknown as { trackers: FusionTrackerPattern[] }).trackers;
/** The nodes that operations add. */
const nodesOf = (ops: Op[]): FusionNode[] => ops.filter((op): op is Added => op.op === 'add_fusion_node').map(op => op.node);

describe('a Tracker', () => {
  it('is made with every setting as the engine writes a new one, has no setting that takes keys, and has its picture, its foreground and its mask', () => {
    // By its path from the folder that the tests run in, as the catalog is read above.
    const examples: OpsExample[] = JSON.parse(readFileSync('../engine/crates/model/tests/data/ops-examples.json', 'utf8'));
    const added = examples.find(each => each.name === 'add_fusion_node of a tracker with two patterns and their paths')!.done as Added;
    expect(added.node.kind).toBe('tracker');
    const written = added.node.params as FusionTracker;
    // The example match moves two patterns, named and tracked: every other setting is as the engine makes a new Tracker.
    expect({ ...written, operation: 'none', trackers: [] }).toStrictEqual(newTracker());
    expect(Object.keys(newTracker())).toEqual(['operation', 'merge', 'position', 'rotation', 'scaling', 'pivot', 'pivot_tracker', 'pivot_point', 'reference', 'trackers']);
    expect(Object.keys(newTracker()).sort()).toEqual(Object.keys(written).sort());
    // A pattern as the engine makes one, but for where it was put, its name and its path.
    const [first, second] = written.trackers;
    expect(first).toStrictEqual({ ...newPattern(0.25, 0.5), name: 'Left', path: first.path });
    expect(Object.keys(newPattern()).sort()).toEqual(Object.keys(second).sort());
    expect(newPattern()).toStrictEqual({
      name: '', enabled: true, pattern: { x: 0.5, y: 0.5, width: 0.05, height: 0.05 }, search: { width: 0.125, height: 0.125 }, offset: { x: 0, y: 0 }, path: {},
    });
    expect(newNode('tracker', null, 't1')).toStrictEqual({ id: 't1', label: '', pass_through: false, pos: null, kind: 'tracker', params: newTracker() });
    expect(newTracker().trackers).not.toBe(newTracker().trackers);
    // None of its settings takes keys: the catalog of the engine has no entry for it.
    const catalog: Record<string, Entry[]> = JSON.parse(readFileSync('../engine/crates/model/tests/data/catalog.json', 'utf8'));
    expect(catalog.fusion_tracker).toBeUndefined();
    expect([FUSION_SETTINGS.tracker, settingsOf(newNode('tracker', null, 't1'))]).toEqual([[], []]);
    expect(hasKeys(trackerNode([left()]))).toBe(false);
    // Its picture first, which it hands on, then the foreground that it makes follow its patterns, then its effect mask.
    expect(INPUTS.tracker).toEqual(['input', 'foreground', 'mask']);
    expect(pictureInputs('tracker')).toEqual(['input', 'foreground']);
    expect([hasOutput('tracker'), hasMaskInput('tracker'), isMaskKind('tracker'), handedOn('tracker'), matteInputs('tracker')]).toEqual([true, true, false, 'input', []]);
  });

  it('is added in the group of the effects with one pattern in the middle of the picture, after the node chosen on its picture', () => {
    const start = { ...newNode('tracker'), params: { ...newTracker(), trackers: [newPattern()] } };
    expect(ADDABLE_EFFECTS).toContain('tracker');
    expect(addNode(AT, comp(), 'tracker', null, { x: 10.4, y: 20.6 })).toEqual([{ op: 'add_fusion_node', ...AT, node: { ...start, pos: { x: 10, y: 21 } } }]);
    expect(addNode(AT, comp(), 'tracker', 'n3', null)).toEqual([{
      op: 'add_fusion_node', ...AT, node: start, links: [link('n3', '', 'input'), link('', 'n4', 'background')],
    }]);
    // The engine's new one has none, and each new one has a list of its own.
    expect(newTracker().trackers).toEqual([]);
    // A mask added with it chosen goes into its effect mask; its foreground is linked by hand.
    const withTracker: Composition = { nodes: [...comp().nodes, trackerNode([])], links: comp().links };
    expect(addNode(AT, withTracker, 'ellipse', 'n6', null)).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n6', 'mask')] }]);
  });

  it('has its patterns set whole, the others and every path the objects that they are', () => {
    const one = left();
    const two: FusionTrackerPattern = { ...newPattern(0.75, 0.5), name: 'Right', path: { 4: [0.75, 0.5] } };
    const node = trackerNode([one, two]);
    const moved = setPatternValue(AT, node, 0, 'pattern.x', 0.3);
    expect(moved).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n6', params: { trackers: [{ ...one, pattern: { ...one.pattern, x: 0.3 } }, two] } }]);
    expect(listOf(moved)[1]).toBe(two);
    expect(listOf(moved)[0].path).toBe(one.path);
    // Each number within its range; a number that is what it is already sends nothing; nor does a pattern or a node that is not there.
    expect(listOf(setPatternValue(AT, node, 1, 'search.width', 2))[1].search).toEqual({ width: 1, height: 0.125 });
    expect(listOf(setPatternValue(AT, node, 0, 'offset.y', -20))[0].offset).toEqual({ x: 0, y: -16 });
    expect(listOf(setPatternValue(AT, node, 0, 'pattern.y', 99))[0].pattern).toEqual({ x: 0.25, y: 16.5, width: 0.05, height: 0.05 });
    expect([
      setPatternValue(AT, node, 0, 'pattern.width', 0.05), setPatternValue(AT, node, 2, 'pattern.x', 0.3), setPatternValue(AT, newNode('blur', null, 'b1'), 0, 'pattern.x', 0.3),
    ]).toEqual([[], [], []]);
    // Renamed, turned off, and its path cleared: the other pattern is the object that it was.
    const renamed = renamePattern(AT, node, 1, 'Eye');
    expect(listOf(renamed)).toEqual([one, { ...two, name: 'Eye' }]);
    expect(listOf(renamed)[0]).toBe(one);
    expect(renamePattern(AT, node, 1, 'Right')).toEqual([]);
    expect(listOf(enablePattern(AT, node, 0, false))).toEqual([{ ...one, enabled: false }, two]);
    expect(enablePattern(AT, node, 0, true)).toEqual([]);
    const cleared = clearPath(AT, node, 0);
    expect(listOf(cleared)).toEqual([{ ...one, path: {} }, two]);
    expect(listOf(cleared)[1]).toBe(two);
    expect(clearPath(AT, trackerNode([newPattern()]), 0)).toEqual([]);
    // Add Tracker: a pattern in the middle of the picture after the others, up to 16.
    const more = addPattern(AT, node);
    expect(listOf(more)).toEqual([one, two, newPattern()]);
    expect([listOf(more)[0], listOf(more)[1]]).toEqual([one, two]);
    expect(listOf(more)[0]).toBe(one);
    expect(MOST_TRACKERS).toBe(16);
    expect(addPattern(AT, trackerNode(Array.from({ length: MOST_TRACKERS }, () => newPattern())))).toEqual([]);
    expect(listOf(addPattern(AT, trackerNode(Array.from({ length: MOST_TRACKERS - 1 }, () => newPattern()))))).toHaveLength(MOST_TRACKERS);
    // A pattern dragged on the picture: where it was put, whatever it was before, and the list put back whole where the drag is taken back.
    expect(placedPattern(AT, node, 0, 0.25, 0.5)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n6', params: { trackers: [one, two] } });
    const dragged = placedPattern(AT, node, 1, 0.5, 0.625)!;
    expect(listOf([dragged])).toEqual([one, { ...two, pattern: { ...two.pattern, x: 0.5, y: 0.625 } }]);
    expect(listOf([dragged])[1].path).toBe(two.path);
    expect([placedPattern(AT, node, 3, 0.5, 0.5), restoredPatterns(AT, newNode('blur', null, 'b1'))]).toEqual([null, null]);
    expect(restoredPatterns(AT, node)).toEqual({ op: 'set_fusion_node', ...AT, id: 'n6', params: { trackers: [one, two] } });
    // In the copy, as the engine does it: the list in the place of the list, whole.
    const after = applied(withNode(node), moved).nodes[2];
    expect(after.params).toEqual({ ...newTracker(), trackers: listOf(moved) });
  });

  it('takes a pattern away: the pivot and the centres that name a later pattern name it by its new place, in the same transaction', () => {
    const [one, two, three] = [left(), newPattern(0.75, 0.5), newPattern(0.5, 0.75)];
    const node = trackerNode([one, two, three], 'n6', { pivot: 'selected_tracker', pivot_tracker: 2 });
    const composition: Composition = {
      nodes: [
        mediaIn(), mediaOut(), node, withFollow(merge('n7'), ofPattern(0)), withFollow(rectangle('n8'), ofPattern(2)),
        withFollow(transform('n9'), { node: 'n6', output: 'steady_position', tracker: 0 }), withFollow(ellipse('n10'), ofPattern(1, 'n11')),
      ],
      links: [link('MediaIn1', 'MediaOut1')],
    };
    // The first: the pivot comes to the second place, the centre that followed it lets go, and the one that followed the third follows the second.
    const removed = removePattern(AT, composition, node, 0);
    expect(removed).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n6', params: { pivot_tracker: 1, trackers: [two, three] } },
      { op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: null } } },
      { op: 'set_fusion_node', ...AT, id: 'n8', params: { center: { follow: ofPattern(1) } } },
    ]);
    expect(listOf(removed)[0]).toBe(two);
    // The last: nothing comes after it, and the pivot, which named it, names it still.
    expect(removePattern(AT, composition, node, 2)).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n6', params: { trackers: [one, two] } },
      { op: 'set_fusion_node', ...AT, id: 'n8', params: { center: { follow: null } } },
    ]);
    expect([removePattern(AT, composition, node, 3), removePattern(AT, composition, merge('n7'), 0)]).toEqual([[], []]);
    // In the copy, as the engine does them.
    const after = applied(composition, removed);
    expect(after.nodes.map(followOf)).toEqual([null, null, null, null, ofPattern(1), { node: 'n6', output: 'steady_position', tracker: 0 }, ofPattern(1, 'n11')]);
    expect((after.nodes[2].params as FusionTracker).pivot_tracker).toBe(1);
  });

  it('has the settings of its Operation tab set by one patch of those that change', () => {
    const node = trackerNode([left()]);
    expect(setTracker(AT, node, { operation: 'match_move', merge: 'bg_only', reference: 12 })).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n6', params: { operation: 'match_move', reference: 12 } },
    ]);
    expect(setTracker(AT, node, { pivot_point: { x: 0.25, y: 0.5 }, rotation: false })).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n6', params: { pivot_point: { x: 0.25, y: 0.5 }, rotation: false } },
    ]);
    expect([setTracker(AT, node, { pivot_point: { x: 0.5, y: 0.5 }, pivot: 'tracker_average' }), setTracker(AT, newNode('blur', null, 'b1'), { operation: 'match_move' })])
      .toEqual([[], []]);
    const after = applied(withNode(node), setTracker(AT, node, { merge: 'fg_over_bg', pivot: 'manual', pivot_point: { x: 0.25, y: 0.75 } })).nodes[2];
    expect(after.params).toEqual({ ...newTracker(), merge: 'fg_over_bg', pivot: 'manual', pivot_point: { x: 0.25, y: 0.75 }, trackers: [left()] });
  });

  it('is followed by the centre of a Merge, a Transform or a mask, set and let go of by a patch of the centre that keeps its own place', () => {
    const box = rectangle('n7');
    const composition: Composition = { nodes: [mediaIn(), mediaOut(), trackerNode([left(), newPattern()]), box], links: [link('MediaIn1', 'MediaOut1')] };
    const follow = ofPattern(1);
    const set = followOps(AT, box, follow);
    expect(set).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: { node: 'n6', output: 'tracker', tracker: 1 } } } }]);
    const followed = applied(composition, set);
    const following = followed.nodes[3];
    expect((following.params as FusionRectangle).center).toEqual({ x: 0.5, y: 0.5, follow });
    expect(followOf(following)).toEqual(follow);
    // What it follows already sends nothing; another output of the same Tracker is sent whole, its pattern written.
    expect(followOps(AT, following, follow)).toEqual([]);
    const steady: FusionFollow = { node: 'n6', output: 'steady_position', tracker: 0 };
    expect(followOps(AT, following, steady)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: steady } } }]);
    // Disconnect: `follow: null`, and the centre is its own place again, with no `follow` written.
    const letGo = followOps(AT, following, null);
    expect(letGo).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: null } } }]);
    const free = applied(followed, letGo).nodes[3];
    expect((free.params as FusionRectangle).center).toStrictEqual({ x: 0.5, y: 0.5 });
    expect([followOf(free), followOps(AT, free, null)]).toEqual([null, []]);
    // Five kinds have a centre; a Tracker has none, and so follows nothing.
    expect(CENTER_KINDS).toEqual(['merge', 'transform', 'rectangle', 'ellipse', 'polygon', 'text_plus']);
    expect((['blur', 'tracker', 'background', 'media_in', 'delta_keyer'] as const).map(hasCenter)).toEqual([false, false, false, false, false]);
    expect([followOps(AT, trackerNode([]), follow), followOf(trackerNode([]))]).toEqual([[], null]);
    expect(withFollow(trackerNode([]), follow)).toEqual(trackerNode([]));
    // What a centre may follow: of each Tracker, where each of its patterns is, then its Steady and its Unsteady Position.
    const choices = followChoices(composition);
    expect(choices).toEqual([ofPattern(0), ofPattern(1), steady, { node: 'n6', output: 'unsteady_position', tracker: 0 }]);
    expect(followChoices(comp())).toEqual([]);
    const words = { unnamed: 'Tracker {n}', steady: 'Steady Position', unsteady: 'Unsteady Position' };
    expect(choices.map(each => followName(composition, new Map([['n6', 'Tracker 1']]), each, words)))
      .toEqual(['Tracker 1 › Left', 'Tracker 1 › Tracker 2', 'Tracker 1 › Steady Position', 'Tracker 1 › Unsteady Position']);
    expect([patternName({ name: '' }, 2, 'Tracker {n}'), patternName({ name: 'Eye' }, 2, 'Tracker {n}'), patternName(undefined, 0, 'Tracker {n}')])
      .toEqual(['Tracker 3', 'Eye', 'Tracker 1']);
  });

  it('publishes where a pattern is on its path at a frame, and its offset, for a centre that follows it', () => {
    // On the line between frames 0 and 8: at frame 2 a quarter of the way, at 0.375, 0.4375; and with its offset, right and up.
    const placed = trackerNode([{ ...left(), offset: { x: 0.125, y: -0.25 } }]);
    const where: Composition = { nodes: [mediaIn(), mediaOut(), placed], links: [link('MediaIn1', 'MediaOut1')] };
    expect(followedPlace(where, ofPattern(0), 2)).toEqual([0.5, 0.1875]);
    // Before the first frame where the first has it, and after the last where the last has it.
    expect(followedPlace(where, ofPattern(0), -5)).toEqual([0.375, 0.25]);
    expect(followedPlace(where, ofPattern(0), 40)).toEqual([0.875, 0]);
    // Nothing of a pattern that is not there or has no path, of a Tracker that is not there or is passed through, and of the two positions.
    const pathless = { ...where, nodes: [mediaIn(), mediaOut(), trackerNode([newPattern()])] };
    const passed = { ...where, nodes: [mediaIn(), mediaOut(), { ...placed, pass_through: true }] };
    expect([
      followedPlace(where, ofPattern(1), 2), followedPlace(pathless, ofPattern(0), 2), followedPlace(where, ofPattern(0, 'n9'), 2), followedPlace(passed, ofPattern(0), 2),
      followedPlace(where, { node: 'n6', output: 'steady_position', tracker: 0 }, 2), followedPlace(where, { node: 'n6', output: 'unsteady_position', tracker: 0 }, 2),
    ]).toEqual([null, null, null, null, null, null]);
  });

  it('lets go of every centre that follows it before it is deleted or cut, in the same transaction, and is joined across by its picture alone', () => {
    // MediaIn1 into the Tracker n6, the Background n5 its foreground, and the Tracker into the background of the Merge n4; the
    // Rectangle n7 follows the first pattern of the Tracker, and the Transform n8 its Steady Position.
    const tracked = (): Composition => ({
      nodes: [
        mediaIn(), mediaOut(), merge('n4'), background('n5'), trackerNode([left()]), withFollow(rectangle('n7'), ofPattern(0)),
        withFollow(transform('n8'), { node: 'n6', output: 'steady_position', tracker: 0 }),
      ],
      links: [link('MediaIn1', 'n6'), link('n5', 'n6', 'foreground'), link('n6', 'n4', 'background'), link('n4', 'MediaOut1')],
    });
    const ops = removeNodes(AT, tracked(), ['n6']);
    expect(ops).toEqual([
      { op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: null } } },
      { op: 'set_fusion_node', ...AT, id: 'n8', params: { center: { follow: null } } },
      { op: 'remove_fusion_node', ...AT, id: 'n6', links: [link('MediaIn1', 'n4', 'background')] },
    ]);
    // In the copy: the Tracker is gone, its foreground is joined to nothing, and no centre follows anything.
    const after = applied(tracked(), ops);
    expect(after.nodes.map(node => node.id)).toEqual(['MediaIn1', 'MediaOut1', 'n4', 'n5', 'n7', 'n8']);
    expect(after.nodes.map(followOf)).toEqual([null, null, null, null, null, null]);
    expect(after.links.map(linkName).sort()).toEqual(['MediaIn1>n4:background', 'n4>MediaOut1:input'].sort());
    // Cut takes it away as Delete does, and keeps it whole.
    const cut = cutNodes(AT, tracked(), ['n6']);
    expect(cut.ops).toEqual(ops);
    expect(cut.copied!.nodes.map(node => [node.id, node.kind])).toEqual([['n6', 'tracker']]);
    // A centre that follows it, deleted alone, lets go of nothing; deleted with it, it lets go first all the same.
    expect(removeNodes(AT, tracked(), ['n7'])).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n7' }]);
    expect(removeNodes(AT, tracked(), ['n7', 'n6']).slice(0, 2)).toEqual(ops.slice(0, 2));
  });

  it('is followed by the copy of a centre that is pasted with it, by its name in the transaction, and by none that is pasted where it is not', () => {
    const tracked = (): Composition => ({
      nodes: [mediaIn(), mediaOut(), trackerNode([left()]), withFollow(rectangle('n7'), ofPattern(0))], links: [link('MediaIn1', 'MediaOut1')],
    });
    // The Tracker pasted before the centre: the centre follows its name as it is added.
    const both = pasteNodes(AT, comp(), copyNodes(tracked(), ['n6', 'n7']));
    expect(both).toHaveLength(2);
    expect(nodesOf(both).map(node => [node.id, node.kind, followOf(node)])).toEqual([['$p0', 'tracker', null], ['$p1', 'rectangle', ofPattern(0, '$p0')]]);
    // The centre pasted before the Tracker: it is added following nothing, and follows the Tracker once that is added.
    const backwards: Composition = {
      nodes: [mediaIn(), mediaOut(), withFollow(rectangle('n7'), ofPattern(0, 'n9')), trackerNode([left()], 'n9')], links: [link('MediaIn1', 'MediaOut1')],
    };
    const later = pasteNodes(AT, comp(), copyNodes(backwards, ['n9', 'n7']));
    expect(nodesOf(later).map(node => [node.id, node.kind, followOf(node)])).toEqual([['$p0', 'rectangle', null], ['$p1', 'tracker', null]]);
    expect(later.slice(2)).toEqual([{ op: 'set_fusion_node', ...AT, id: '$p0', params: { center: { follow: ofPattern(0, '$p1') } } }]);
    expect((nodesOf(later)[0].params as FusionRectangle).center).toStrictEqual({ x: 0.5, y: 0.5 });
    // The centre alone: into a composition that has the Tracker it follows it still, and into one that has not it follows nothing.
    const alone = copyNodes(tracked(), ['n7']);
    expect(followOf(nodesOf(pasteNodes(AT, tracked(), alone))[0])).toEqual(ofPattern(0));
    expect(followOf(nodesOf(pasteNodes(AT, comp(), alone))[0])).toBeNull();
    const notTracker: Composition = { ...comp(), nodes: [...comp().nodes, merge('n6')] };
    expect(followOf(nodesOf(pasteNodes(AT, notTracker, alone))[0])).toBeNull();
    // What was copied is as it was, to be pasted again.
    expect(followOf(alone!.nodes[0])).toEqual(ofPattern(0));
  });
});

describe('the width of the name of a node', () => {
  it('counts a character of the CJK scripts or a full-width one as an em, and any other as a little more than half of one', () => {
    expect(nameWidth('MediaIn1')).toBeCloseTo(8 * 6.2, 6);
    expect(nameWidth('键控器')).toBe(33);
    expect(nameWidth('ＡＢ')).toBe(22);
    expect(nameWidth('Delta 键控器 1')).toBeCloseTo(8 * 6.2 + 33, 6);
    // The names that the page gives in Chinese fit their cards but the Delta Keyer's, which is pressed into the room.
    for (const fits of ['媒体输入 1', '媒体输出 1', '跟踪器 1', '椭圆 1', '多边形 1', 'Merge 12']) expect(nameWidth(fits)).toBeLessThanOrEqual(NAME_ROOM);
    expect(nameWidth('Delta 键控器 1')).toBeGreaterThan(NAME_ROOM);
  });
});
