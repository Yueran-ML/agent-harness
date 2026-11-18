// @vitest-environment jsdom
/**
 * The Effects tab of the Inspector of the Edit page for a clip of picture (DaVinci Resolve's Effects tab, beside Video, Audio and
 * File): a Blur, a Mosaic and a 3D Keyer, each a node of the composition of the clip (PROTOCOL.md, "Compositions"). Which
 * effects are listed, and in what order: those on the chain of the picture that ends in the MediaOut; adding one right before
 * the MediaOut, with the first composition before it where the clip has none, in one transaction; turning one off, which passes
 * it through; deleting one, joined across as the node editor's Delete joins a node; its settings, which send what the Inspector
 * of the Fusion page sends; titles and colours, which have no 3D Keyer; several clips; a locked track; and Open in Fusion.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useEffect, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { en } from '../src/messages/en';
import {
  applyOp, emptyGrade, firstComposition, ProjectCopy, type Clip, type Composition, type EngineEvent, type FusionLink, type FusionNode, type Op, type Project,
  type Timeline, type TransportState,
} from '../src/studio/engine';
import { newNode, removeNodes } from '../src/studio/fusion/edits';
import { addEffect, chainEffects, effectsFor, enableEffect, hasMoreNodes, outputChain, removeEffect } from '../src/studio/inspector/effectChain';
import { Inspector } from '../src/studio/inspector/Inspector';
import { Studio } from '../src/studio/Studio';
import { likeABrowser } from './mounted';
import { clip, media } from './timeline-fixture';

const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; });

const labels = en.label.studio.inspector;
const said = en.hint.studio.inspector;
const AT = { timeline: 't1', clip: 'c1' };
const MEDIA = [media];
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
/** The MediaIn and the MediaOut that every composition has, and the nodes given after them. */
const nodes = (...made: FusionNode[]): FusionNode[] => [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), ...made];
const passed = (node: FusionNode): FusionNode => ({ ...node, pass_through: true }) as FusionNode;
/** MediaIn1 through the Mosaic n6, the Blur n7 (passed through) and the Delta Keyer n8 into MediaOut1, and the Blur n9 that nothing is linked to or from. */
const chained = (): Composition => ({
  nodes: nodes(newNode('mosaic', null, 'n6'), passed(newNode('blur', null, 'n7')), newNode('delta_keyer', null, 'n8'), newNode('blur', null, 'n9')),
  links: [link('MediaIn1', 'n6'), link('n6', 'n7'), link('n7', 'n8'), link('n8', 'MediaOut1')],
});
/** MediaIn1 through the Blur n6 into MediaOut1, and nothing else. */
const plain = (): Composition => ({ nodes: nodes(newNode('blur', null, 'n6')), links: [link('MediaIn1', 'n6'), link('n6', 'MediaOut1')] });
/** MediaIn1 through the Blur n6 and the Transform n7 into MediaOut1; n7 feeds the foreground of the Merge n5 too. */
const branched = (): Composition => ({
  nodes: nodes(newNode('merge', null, 'n5'), newNode('blur', null, 'n6'), newNode('transform', null, 'n7')),
  links: [link('MediaIn1', 'n6'), link('n6', 'n7'), link('n7', 'MediaOut1'), link('n7', 'n5', 'foreground')],
});
const titled = (id = 'c1', start = 0): Clip => clip(id, start, 100, {
  source: { kind: 'title', text: 'Hello', style: { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } },
});
const colored = (id = 'c1', start = 0): Clip => clip(id, start, 100, { source: { kind: 'color', color: '#ff0000ff' } });
const timelineOf = (clips: Clip[], locked = false): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});

/**
 * An engine that keeps the timeline, applies the operations of a transaction as the copy does, and gives a node added without an
 * id the next one, its links with it, as the real engine does; whatever draws the Inspector is told of each change.
 */
function engineOf(first: Timeline) {
  let project: Project = { schema: 5, next_id: 20, bins: [], groups: [], media: [media], timelines: [first] };
  let revision = 17;
  const watchers = new Set<() => void>();
  const call = vi.fn(async (method: string, params?: Record<string, unknown>, _context?: unknown) => {
    if (method !== 'transaction.apply') return { ok: true, result: {} };
    const done = (params!.ops as Op[]).map((op): Op => {
      if (op.op !== 'add_fusion_node' || op.node.id !== '') return op;
      const id = `n${project.next_id}`;
      project = { ...project, next_id: project.next_id + 1 };
      const own = (end: string) => (end === '' ? id : end);
      return { ...op, node: { ...op.node, id } as FusionNode, ...(op.links ? { links: op.links.map(each => ({ ...each, from: own(each.from), to: own(each.to) })) } : {}) };
    });
    for (const op of done) project = applyOp(project, op);
    revision++;
    watchers.forEach(watcher => watcher());
    return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
  });
  return {
    call,
    bridge: { call, onEvent: () => () => {} } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>,
    timeline: () => project.timelines[0],
    revision: () => revision,
    /** The composition of the clip `c1` as the engine has it. */
    composition: () => (project.timelines[0].tracks[0] as { clips: Clip[] }).clips.find(each => each.id === 'c1')!.fusion,
    watch: (watcher: () => void) => { watchers.add(watcher); return () => { watchers.delete(watcher); }; },
  };
}
type Engine = ReturnType<typeof engineOf>;

/** The Inspector of the Edit page on the engine's timeline, drawn again as the engine changes it. */
function Kept({ engine, clips, onFusion }: { engine: Engine; clips: string[]; onFusion?: () => void }) {
  const [, redraw] = useState(0);
  useEffect(() => engine.watch(() => redraw(count => count + 1)), [engine]);
  return <Inspector bridge={engine.bridge} session={1} revision={engine.revision()} timeline={engine.timeline()} media={MEDIA} clips={clips}
    transition={null} onSpeed={() => {}} onFusion={onFusion} />;
}
function mount(clips: Clip[], options: { chosen?: string[]; locked?: boolean; onFusion?: () => void } = {}) {
  const engine = engineOf(timelineOf(clips, options.locked));
  render(<Kept engine={engine} clips={options.chosen ?? clips.map(each => each.id)} onFusion={options.onFusion} />);
  return engine;
}
/** The operations of each transaction sent. */
const sent = (engine: Engine) => engine.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => (params as { ops: Op[] }).ops);
const byTest = (id: string) => document.querySelector<HTMLElement>(`[data-test="${id}"]`);
const openEffects = async () => { fireEvent.click(screen.getByRole('tab', { name: 'Effects' })); await settle(); };
const menuItems = () => [...document.querySelectorAll<HTMLElement>('[data-test="color-menu"] [role="menuitem"]')].map(each => each.getAttribute('aria-label'));
/** Add Effect, and the effect of that kind from its menu. */
const add = async (kind: string) => {
  fireEvent.click(byTest('add-clip-effect')!);
  fireEvent.click(byTest(`menu-effect-${kind}`)!);
  await settle();
};
const effectOf = (id: string) => document.querySelector<HTMLElement>(`[data-test="clip-effects"] [data-effect="${id}"]`)!;
/** The effects listed: the id of each node, its kind, the name it is shown by, and whether it is on. */
const listed = () => [...document.querySelectorAll<HTMLElement>('[data-test="clip-effects"] [data-effect]')]
  .map(each => [each.dataset.effect, each.dataset.kind, each.querySelector('[data-test="effect-name"]')!.textContent, each.dataset.enabled === 'true']);
const switchOf = (id: string) => effectOf(id).querySelector<HTMLInputElement>('[data-test="effect-enabled"]')!;
const fieldsIn = (id: string) => [...effectOf(id).querySelectorAll<HTMLElement>('[data-field]')].map(each => each.dataset.field);
const rowIn = (id: string, name: string) => effectOf(id).querySelector<HTMLElement>(`[data-field="${name}"]`)!;
const boxIn = (id: string, name: string) => rowIn(id, name).querySelector<HTMLInputElement>('input.inspector-number')!;
const typeIn = (id: string, name: string, value: string) => {
  fireEvent.change(boxIn(id, name), { target: { value } });
  fireEvent.keyDown(boxIn(id, name), { key: 'Enter' });
};
const keysIn = (id: string, name: string) => [...effectOf(id).querySelectorAll<HTMLButtonElement>(`[data-keys="${name}"] button`)];
const node = (op: Partial<Extract<Op, { op: 'set_fusion_node' }>> & { id: string }) => ({ op: 'set_fusion_node', ...AT, ...op });

describe('the effects of a clip, as its composition has them', () => {
  it('are the Blurs, Mosaics and Delta Keyers on the chain of the picture that ends in the MediaOut, from the MediaIn side', () => {
    expect(outputChain(chained()).map(each => each.id)).toEqual(['MediaIn1', 'n6', 'n7', 'n8']);
    expect(chainEffects(chained()).map(each => each.id)).toEqual(['n6', 'n7', 'n8']);
    // The Blur n9 is off the chain: the composition has more than its effects.
    expect(hasMoreNodes(chained())).toBe(true);
    expect([chainEffects(plain()).map(each => each.id), hasMoreNodes(plain())]).toEqual([['n6'], false]);
    // The chain goes back through a Transform, and through a Merge by its background: neither is an effect, and both are more.
    expect(outputChain(branched()).map(each => each.id)).toEqual(['MediaIn1', 'n6', 'n7']);
    expect([chainEffects(branched()).map(each => each.id), hasMoreNodes(branched())]).toEqual([['n6'], true]);
    const merged: Composition = {
      nodes: nodes(newNode('blur', null, 'n6'), newNode('merge', null, 'n4'), newNode('background', null, 'n5'), newNode('mosaic', null, 'n7')),
      links: [link('MediaIn1', 'n6'), link('n6', 'n4', 'background'), link('n5', 'n4', 'foreground'), link('n4', 'n7'), link('n7', 'MediaOut1')],
    };
    expect(chainEffects(merged).map(each => each.id)).toEqual(['n6', 'n7']);
    // The first composition has none, and nothing more; a MediaOut that nothing feeds has no chain, and its MediaIn is more.
    expect([outputChain(firstComposition()).map(each => each.id), chainEffects(firstComposition()), hasMoreNodes(firstComposition())]).toEqual([['MediaIn1'], [], false]);
    const loose: Composition = { nodes: nodes(), links: [] };
    expect([outputChain(loose), hasMoreNodes(loose)]).toEqual([[], true]);
  });

  it('are added right before the MediaOut, with the first composition before them where the clip has none, and moving no other link', () => {
    expect(addEffect(AT, clip('c1'), 'blur')).toEqual([
      { op: 'set_fusion', ...AT, value: firstComposition() },
      { op: 'add_fusion_node', ...AT, node: newNode('blur'), links: [link('MediaIn1', ''), link('', 'MediaOut1')] },
    ]);
    // After the Transform that reaches the MediaOut: its link into the foreground of the Merge stays where it is.
    const added = addEffect(AT, clip('c1', 0, 100, { fusion: branched() }), 'delta_keyer');
    expect(added).toEqual([{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer'), links: [link('n7', ''), link('', 'MediaOut1')] }]);
    // A title and a colour have a Blur and a Mosaic, and no 3D Keyer.
    expect([effectsFor(clip('c1')), effectsFor(titled()), effectsFor(colored())]).toEqual([['blur', 'mosaic', 'delta_keyer'], ['blur', 'mosaic'], ['blur', 'mosaic']]);
    expect(addEffect(AT, titled(), 'delta_keyer')).toEqual([]);
    expect(addEffect(AT, colored(), 'mosaic')).toEqual([
      { op: 'set_fusion', ...AT, value: firstComposition() },
      { op: 'add_fusion_node', ...AT, node: newNode('mosaic'), links: [link('MediaIn1', ''), link('', 'MediaOut1')] },
    ]);
    // A composition of 256 nodes has no room for one more.
    const full: Composition = { nodes: [...nodes(), ...Array.from({ length: 254 }, (_, index) => newNode('background', null, `b${index}`))], links: [link('MediaIn1', 'MediaOut1')] };
    expect(addEffect(AT, clip('c1', 0, 100, { fusion: full }), 'blur')).toEqual([]);
  });

  it('are deleted as the node editor deletes a node, and turned off by Pass Through', () => {
    const had = clip('c1', 0, 100, { fusion: chained() });
    expect(removeEffect(AT, had, 'n7')).toEqual(removeNodes(AT, chained(), ['n7']));
    expect(removeEffect(AT, had, 'n7')).toEqual([{ op: 'remove_fusion_node', ...AT, id: 'n7', links: [link('n6', 'n8')] }]);
    expect(removeEffect(AT, clip('c1'), 'n7')).toEqual([]);
    expect(enableEffect(AT, had, 'n7', true)).toEqual([node({ id: 'n7', pass_through: false })]);
    expect(enableEffect(AT, had, 'n6', false)).toEqual([node({ id: 'n6', pass_through: true })]);
    expect([enableEffect(AT, had, 'n6', true), enableEffect(AT, had, 'n7', false), enableEffect(AT, had, 'n99', false)]).toEqual([[], [], []]);
  });
});

describe('the Effects tab of the Inspector of the Edit page', () => {
  it('is beside Video for a clip of picture, and adds each effect to a clip without a composition, the first one with it, in one transaction', async () => {
    const engine = mount([clip('c1', 0, 100)]);
    expect(screen.getAllByRole('tab').map(tab => [tab.textContent, tab.getAttribute('aria-selected')])).toEqual([['Video', 'true'], ['Effects', 'false']]);
    await openEffects();
    expect([byTest('effects-none')?.textContent, byTest('effects-composition'), byTest('clip-effect-list')]).toEqual([said.noEffects, null, null]);
    fireEvent.click(byTest('add-clip-effect')!);
    expect(menuItems()).toEqual(['Blur', 'Mosaic', '3D Keyer']);
    fireEvent.click(byTest('menu-effect-blur')!);
    await settle();
    expect(sent(engine)).toEqual([[
      { op: 'set_fusion', ...AT, value: firstComposition() },
      { op: 'add_fusion_node', ...AT, node: newNode('blur'), links: [link('MediaIn1', ''), link('', 'MediaOut1')] },
    ]]);
    expect(listed()).toEqual([['n20', 'blur', 'Blur 1', true]]);
    // The clip has a composition now: each effect goes in after the last one, before the MediaOut.
    engine.call.mockClear();
    await add('mosaic');
    await add('delta_keyer');
    expect(sent(engine)).toEqual([
      [{ op: 'add_fusion_node', ...AT, node: newNode('mosaic'), links: [link('n20', ''), link('', 'MediaOut1')] }],
      [{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer'), links: [link('n21', ''), link('', 'MediaOut1')] }],
    ]);
    expect(listed()).toEqual([['n20', 'blur', 'Blur 1', true], ['n21', 'mosaic', 'Mosaic 1', true], ['n22', 'delta_keyer', '3D Keyer 1', true]]);
    expect(outputChain(engine.composition()!).map(each => each.id)).toEqual(['MediaIn1', 'n20', 'n21', 'n22']);
    // The effect mask of each is left free, and the composition is the effects alone.
    expect(engine.composition()!.links.filter(each => each.input === 'mask')).toEqual([]);
    expect([byTest('effects-none'), byTest('effects-composition')]).toEqual([null, null]);
  });

  it('adds an effect to a clip with a chain after whatever reaches the MediaOut, and lists only the effects, saying that the composition has more', async () => {
    const engine = mount([clip('c1', 0, 100, { fusion: branched() })]);
    await openEffects();
    expect(listed()).toEqual([['n6', 'blur', 'Blur 1', true]]);
    expect(byTest('effects-composition')!.querySelector('p')!.textContent).toBe(said.effectsComposition);
    await add('mosaic');
    expect(sent(engine)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('mosaic'), links: [link('n7', ''), link('', 'MediaOut1')] }]]);
    expect(listed()).toEqual([['n6', 'blur', 'Blur 1', true], ['n20', 'mosaic', 'Mosaic 1', true]]);
    // In the engine's order of links, by the node that each goes into: n20 before n5.
    expect(engine.composition()!.links.filter(each => each.from === 'n7')).toEqual([link('n7', 'n20'), link('n7', 'n5', 'foreground')]);
  });

  it('lists the effects on the chain in order, each by the name of the Edit page, with its switch, its settings that open and close, and Delete', async () => {
    mount([clip('c1', 0, 100, { fusion: chained() })]);
    await openEffects();
    // The Blur n9, off the chain, is not listed; it counts among the Blurs, after n7.
    expect(listed()).toEqual([['n6', 'mosaic', 'Mosaic 1', true], ['n7', 'blur', 'Blur 1', false], ['n8', 'delta_keyer', '3D Keyer 1', true]]);
    expect(['n6', 'n7', 'n8'].map(id => switchOf(id).checked)).toEqual([true, false, true]);
    expect(['n6', 'n7', 'n8'].map(id => effectOf(id).querySelector('[data-test="remove-effect"]')!.getAttribute('aria-label')))
      .toEqual(['Delete effect: Mosaic 1', 'Delete effect: Blur 1', 'Delete effect: 3D Keyer 1']);
    expect(byTest('effects-composition')).not.toBeNull();
    // The settings are those of the Inspector of the Fusion page, without the name of the node.
    expect(fieldsIn('n7')).toEqual(['filter', 'lock_xy', 'size.x', 'blend']);
    expect(fieldsIn('n6')).toEqual(['frequency', 'aspect', 'offset.x', 'offset.y', 'whole_cells_only']);
    expect(fieldsIn('n8')[0]).toBe('view');
    const name = effectOf('n8').querySelector<HTMLButtonElement>('[data-test="effect-name"]')!;
    fireEvent.click(name);
    expect([name.getAttribute('aria-expanded'), effectOf('n8').querySelector('[data-test="clip-effect-settings"]')]).toEqual(['false', null]);
    fireEvent.click(name);
    expect([name.getAttribute('aria-expanded'), fieldsIn('n8')[0]]).toEqual(['true', 'view']);
  });

  it('turns an effect on and off by Pass Through, and deletes one, joining what fed it to what it fed', async () => {
    const engine = mount([clip('c1', 0, 100, { fusion: chained() })]);
    await openEffects();
    fireEvent.click(switchOf('n7')); await settle();
    fireEvent.click(switchOf('n6')); await settle();
    expect(sent(engine)).toEqual([[node({ id: 'n7', pass_through: false })], [node({ id: 'n6', pass_through: true })]]);
    expect(listed().map(each => each[3])).toEqual([false, true, true]);
    engine.call.mockClear();
    fireEvent.click(effectOf('n7').querySelector('[data-test="remove-effect"]')!); await settle();
    expect(sent(engine)).toEqual([[{ op: 'remove_fusion_node', ...AT, id: 'n7', links: [link('n6', 'n8')] }]]);
    expect(listed().map(each => each[0])).toEqual(['n6', 'n8']);
    expect(outputChain(engine.composition()!).map(each => each.id)).toEqual(['MediaIn1', 'n6', 'n8']);
  });

  it('sets the settings of an effect as the Inspector of the Fusion page does, with their keys', async () => {
    const engine = mount([clip('c1', 0, 100, { fusion: chained() })]);
    await openEffects();
    fireEvent.change(rowIn('n7', 'filter').querySelector('select')!, { target: { value: 'box' } }); await settle();
    typeIn('n7', 'size.x', '12.5'); await settle();
    typeIn('n6', 'frequency', '64'); await settle();
    fireEvent.change(rowIn('n8', 'view').querySelector('select')!, { target: { value: 'matte' } }); await settle();
    typeIn('n8', 'gain', '2'); await settle();
    expect(sent(engine)).toEqual([
      [node({ id: 'n7', params: { filter: 'box' } })], [node({ id: 'n7', params: { size: { x: 12.5 } } })], [node({ id: 'n6', params: { frequency: 64 } })],
      [node({ id: 'n8', params: { view: 'matte' } })], [node({ id: 'n8', params: { gain: 2 } })],
    ]);
    expect([boxIn('n7', 'size.x').value, boxIn('n6', 'frequency').value]).toEqual(['12.5', '64.0']);
    // A key at the playhead, by the path of the setting, of the value that it has there.
    engine.call.mockClear();
    expect(keysIn('n7', 'blend').map(each => each.getAttribute('aria-label'))).toEqual(['Blend: Previous keyframe', 'Blend: Add keyframe', 'Blend: Next keyframe']);
    fireEvent.click(keysIn('n7', 'blend')[1]); await settle();
    expect(sent(engine)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n7.blend', at: 0, remove: 0, keys: [{ frame: 0, value: 1, interp: 'linear' }] }]]);
    expect(keysIn('n7', 'blend')[1].getAttribute('aria-pressed')).toBe('true');
  });

  it('offers a title and a colour a Blur and a Mosaic, and no 3D Keyer', async () => {
    const engine = mount([titled()]);
    // A title opens on its own tab, with Video and Effects beside it.
    expect(screen.getAllByRole('tab').map(tab => tab.textContent)).toEqual(['Title', 'Video', 'Effects']);
    await openEffects();
    fireEvent.click(byTest('add-clip-effect')!);
    expect(menuItems()).toEqual(['Blur', 'Mosaic']);
    fireEvent.click(byTest('menu-effect-blur')!);
    await settle();
    expect(sent(engine)).toEqual([[
      { op: 'set_fusion', ...AT, value: firstComposition() },
      { op: 'add_fusion_node', ...AT, node: newNode('blur'), links: [link('MediaIn1', ''), link('', 'MediaOut1')] },
    ]]);
    expect(listed()).toEqual([['n20', 'blur', 'Blur 1', true]]);
    cleanup();
    mount([colored()]);
    await openEffects();
    fireEvent.click(byTest('add-clip-effect')!);
    expect(menuItems()).toEqual(['Blur', 'Mosaic']);
  });

  it('says to choose one clip where several are chosen', async () => {
    const engine = mount([clip('c1', 0, 100, { fusion: plain() }), clip('c2', 100, 100)]);
    await openEffects();
    expect(byTest('effects-one')!.textContent).toBe(said.effectsOne);
    expect([byTest('add-clip-effect'), byTest('clip-effects')]).toEqual([null, null]);
    expect(sent(engine)).toEqual([]);
  });

  it('shows the effects of a clip on a locked track, and changes nothing of them', async () => {
    const engine = mount([clip('c1', 0, 100, { fusion: chained() })], { locked: true });
    await openEffects();
    expect(byTest('inspector-locked')).not.toBeNull();
    expect(listed().map(each => each[0])).toEqual(['n6', 'n7', 'n8']);
    expect((byTest('add-clip-effect') as HTMLButtonElement).disabled).toBe(true);
    expect(['n6', 'n7', 'n8'].map(id => switchOf(id).disabled)).toEqual([true, true, true]);
    expect(['n6', 'n7', 'n8'].map(id => effectOf(id).querySelector<HTMLButtonElement>('[data-test="remove-effect"]')!.disabled)).toEqual([true, true, true]);
    expect([boxIn('n7', 'size.x').disabled, rowIn('n7', 'filter').querySelector('select')!.disabled, boxIn('n6', 'frequency').disabled]).toEqual([true, true, true]);
    expect(keysIn('n7', 'blend')[1].disabled).toBe(true);
    fireEvent.click(byTest('add-clip-effect')!);
    await settle();
    expect([byTest('color-menu'), sent(engine)]).toEqual([null, []]);
  });

  it('says where the composition has more than its effects, with Open in Fusion where the page can go there', async () => {
    const onFusion = vi.fn();
    mount([clip('c1', 0, 100, { fusion: branched() })], { onFusion });
    await openEffects();
    expect(byTest('open-fusion')!.textContent).toBe(labels.openInFusion);
    fireEvent.click(byTest('open-fusion')!);
    expect(onFusion).toHaveBeenCalledTimes(1);
    cleanup();
    // Without a page to go to, the note alone; and a composition of effects alone has no note.
    mount([clip('c1', 0, 100, { fusion: branched() })]);
    await openEffects();
    expect([byTest('effects-composition') !== null, byTest('open-fusion')]).toEqual([true, null]);
    cleanup();
    mount([clip('c1', 0, 100, { fusion: plain() })], { onFusion });
    await openEffects();
    expect([byTest('effects-composition'), byTest('open-fusion')]).toEqual([null, null]);
  });
});

describe('Open in Fusion on the Edit page', () => {
  let frames: FrameRequestCallback[];
  const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
  const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
    vi.stubGlobal('cancelAnimationFrame', () => {});
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect(0, 0, 600, 300));
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
  });

  /** V1: c1 (frames 0–99); V2 above it: c3 (50–149), with a composition of a Transform and a Merge besides its Blur. */
  const projectOf = (): Project => ({
    schema: 5, next_id: 20, bins: [], groups: [], media: [media], timelines: [{
      id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
      tracks: [
        { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [clip('c1', 0, 100)] },
        { id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, transitions: [], clips: [clip('c3', 50, 100, { fusion: branched() })] },
      ],
    }],
  });
  /** An engine that keeps a project and answers what the studio asks of it as the real one does. */
  function studioEngine(given: Project) {
    let project = given;
    let revision = 0;
    const listeners = new Set<(event: EngineEvent) => void>();
    const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
    let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
    const bridge = {
      open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
      call: vi.fn(async (method: string, params?: Record<string, unknown>) => {
        if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
        if (method === 'media.thumbnail') return new Promise(() => {});
        if (method === 'transport.state') return { ok: true, result: params?.monitor === 'timeline' ? state : { ...state, monitor: 'source', media: null } };
        if (method === 'transport.load') {
          state = { ...state, timeline: params!.timeline as string, frame: 0, shown: 0, last: 299 } as TransportState;
          tell('transport.state', state); return { ok: true, result: state };
        }
        if (method === 'transport.seek') {
          state = { ...state, frame: params!.frame as number, shown: params!.frame as number };
          tell('transport.state', state); return { ok: true, result: {} };
        }
        if (method === 'transaction.apply') {
          const done = params!.ops as Op[];
          for (const op of done) project = applyOp(project, op);
          revision++;
          tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: project.next_id, done });
          return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
        }
        if (method === 'history.undo' || method === 'history.redo') return { ok: true, result: { revision, done: null } };
        return { ok: true, result: {} };
      }),
      importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
      onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    } as unknown as EngineBridge;
    return { bridge, copy: new ProjectCopy(bridge), native: true };
  }
  const page = async (name: string) => { fireEvent.click(screen.getByRole('tab', { name })); await settle(); await draw(); await settle(); };

  it('goes to the Fusion page on the one clip of picture chosen', async () => {
    render(<Studio cwd="F:/film" engine={studioEngine(projectOf())} language="en" onClose={() => {}} />);
    await settle();
    await page('Edit');
    const picture = document.querySelector<HTMLElement>('[data-clip="c3"]')!;
    fireEvent.pointerDown(picture, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    fireEvent.pointerUp(picture, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    await settle();
    fireEvent.click(byTest('toggle-inspector')!);
    await settle();
    await page('Effects');
    expect(listed()).toEqual([['n6', 'blur', 'Blur 1', true]]);
    fireEvent.click(byTest('open-fusion')!);
    await settle(); await draw(); await settle();
    expect(byTest('fusion-page')).not.toBeNull();
    // The playhead stood at frame 0, where c1 is; the clip chosen is the one shown.
    expect(byTest('fusion-clip-name')!.dataset.clip).toBe('c3');
  });
});
