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
  applyOp, firstComposition, type Composition, type FusionLink, type FusionNode, type FusionPoint, type Keyframe, type Op, type Project,
} from '../src/studio/engine';
import {
  ADDABLE, ADDABLE_EFFECTS, ADDABLE_MASKS, addedNode, addedNodes, addNode, connect, copyNodes, cutNodes, disconnect, duplicateNodes, fusionKey, insertOnLink,
  insertPoint, isFirst, LEAST_POINTS, MOST_NODES, MOST_POINTS, moveNodes, newBackground, newMerge, newNode, newPolygon, newTransform, PASTE_OFFSET, pastedName,
  pasteNodes, reconnect, relabel, removeComposition, removeNodes, removePoint, resetComposition, restoreNodes, roomFor, shownComposition, startPoints, togglePass,
  withFirst,
} from '../src/studio/fusion/edits';
import {
  fusionLinkHalves, fusionLinkNear, fusionLinkPath, handedOn, hasMaskInput, hasOutput, INPUTS, inputPoint, isMaskKind, layoutComposition, linkEnds, linkName,
  MASK_INSET, namesOf, nodesIn, outputPoint, pictureInputs, placesOf, portAt, portsOf,
} from '../src/studio/fusion/graph';
import {
  addKey, FUSION_SETTINGS, hasKeys, isPointSetting, keyOps, keyTimesOf, numbered, paramOf, pathOf, pointPath, putValue, removeKey, settingOf, settingsOf,
  wholeParam, wholeValue,
} from '../src/studio/fusion/settings';
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
      rectangle: 'Rectangle', ellipse: 'Ellipse', polygon: 'Polygon', blur: 'Blur', mosaic: 'Mosaic',
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
    expect(ADDABLE_EFFECTS).toEqual(['blur', 'mosaic']);
    expect([...ADDABLE, ...ADDABLE_MASKS].some(kind => kind === 'blur' || kind === 'mosaic')).toBe(false);
    for (const kind of ADDABLE_EFFECTS) {
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
