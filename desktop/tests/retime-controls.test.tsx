// @vitest-environment jsdom
/**
 * The Retime Controls of a clip on the timeline of the Edit page (`src/studio/timeline/RetimeBars`,
 * `useRetimeControls`): how they are shown and put away, what the bar says of the segments of a
 * clip, what each command of a segment's pop-up menu sends, the two drags of a speed point and the
 * speeds that they make, a refusal of the engine, a clip on a locked track, and that a timeline of
 * clips without them mounts none of it.
 *
 * The page is a timeline with what the Edit page gives it: the Retime Controls that are open.
 */
import { readFileSync } from 'node:fs';
import { memo, type FunctionComponent } from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type Retime, type Timeline as TimelineModel } from '../src/studio/engine';
import { beginsLater, courseOf } from '../src/studio/retime';
import { HEADER_WIDTH, RULER_HEIGHT } from '../src/studio/timeline/geometry';
import { arrowGap, CHANGE_SPEEDS } from '../src/studio/timeline/RetimeBars';
import { Timeline } from '../src/studio/timeline/Timeline';
import { useRetimeControls } from '../src/studio/timeline/useRetimeControls';
import { clip, media } from './timeline-fixture';
import { bigTimeline, readyBridge } from './timeline-perf-fixture';
import { likeABrowser } from './mounted';
import { blockOf } from './stylesheet';

const drawn = vi.hoisted(() => ({ bars: 0 }));
/** A component of a module, drawn as it is (a memo component stays one) and counted each time it is drawn. */
async function counted(original: () => Promise<Record<string, unknown>>, exported: string) {
  const real = await original();
  const component = real[exported] as FunctionComponent<object> & { type?: FunctionComponent<object> };
  const inner = component.type ?? component;
  const drawing = (props: object) => { drawn.bars++; return inner(props); };
  return { ...real, [exported]: component.type ? memo(drawing) : drawing };
}
vi.mock('../src/studio/timeline/RetimeBars', original => counted(original, 'RetimeBars'));

beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  // The timeline is 956 px wide beside its headers: the clips of its first frames are in the window that it draws.
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956 + HEADER_WIDTH);
  drawn.bars = 0;
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); });

const rate = { num: 25, den: 1 };
const MEDIA = [media];
const retime = (...points: [number, number, number][]): Retime => ({ points: points.map(([frame, num, den]) => ({ frame, speed: { num, den } })) });
/** 20 frames as the source is, a picture held for 10, and 30 frames back at half the speed. */
const POINTS = retime([0, 1, 1], [20, 0, 1], [30, -1, 2]);
/** c1 from frame 10 for 60 frames, from the first picture of its media. */
const pointed = (fields: Partial<Clip> = {}) => clip('c1', 10, 60, { retime: POINTS, ...fields });
const timelineOf = (clips: Clip[], locked = false, above: Clip[] = []): TimelineModel => ({
  id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] },
    ...(above.length > 0 ? [{ id: 'v2', kind: 'video' as const, name: 'V2', enabled: true, locked: false, clips: above, transitions: [] }] : []),
  ],
});

type Call = ReturnType<typeof vi.fn>;
interface PageProps {
  model: TimelineModel; bridge: Pick<EngineBridge, 'call' | 'onEvent'>; frame?: number; revision?: number; controls?: boolean;
}
/** A timeline as the Edit page holds it: the page keeps which Retime Controls are open. */
function Page({ model, bridge, frame = 25, revision = 17, controls = true }: PageProps) {
  const retimeControls = useRetimeControls(model, 1);
  return <Timeline timeline={model} media={MEDIA} bridge={bridge} session={1} revision={revision} frame={frame} playing={false} onSeek={() => {}}
    retimeControls={controls ? retimeControls : undefined} />;
}
function wire(model: TimelineModel, given: { frame?: number; controls?: boolean } = {}) {
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const props: PageProps = { model, bridge: { call, onEvent: vi.fn(() => () => {}) } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>, ...given };
  const view = render(<Page {...props} />);
  return { ...view, props, call };
}
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
const refusal = (reason: string) => ({ ok: false, error: { code: -32001, message: 'refused', data: { reason } } });

const root = () => screen.getByRole('region', { name: 'Timeline' });
const clipOf = (id: string) => document.querySelector<HTMLElement>(`[data-clip="${id}"]`)!;
const barOf = (id = 'c1') => document.querySelector<HTMLElement>(`[data-retime="${id}"]`);
const pointer = (clientX: number, extra = {}) => ({ button: 0, clientX, clientY: 50, ...extra });
/** Choose a clip as a press on it does. */
const choose = (id = 'c1') => { fireEvent.pointerDown(clipOf(id), pointer(100)); fireEvent.pointerUp(root(), pointer(100)); };
/** Open the Retime Controls of a clip from its right-click menu. */
const open = (id = 'c1') => {
  fireEvent.contextMenu(clipOf(id), { clientX: 200, clientY: 50 });
  fireEvent.click(screen.getByRole('menuitem', { name: 'Retime Controls' }));
  return barOf(id)!;
};
const segments = (id = 'c1') => [...barOf(id)!.querySelectorAll<HTMLElement>('[data-retime-segment]')];
const told = (id = 'c1') => segments(id).map(each => [Number(each.dataset.percent), each.dataset.way, Number(each.style.getPropertyValue('--from')), Number(each.style.getPropertyValue('--frames'))]);
const speeds = (id = 'c1') => [...barOf(id)!.querySelectorAll<HTMLButtonElement>('[data-test="retime-speed"]')];
const handles = (id = 'c1') => [...barOf(id)!.querySelectorAll<HTMLButtonElement>('[data-retime-handle]')];
const handle = (which: 'upper' | 'lower', at: number, id = 'c1') => barOf(id)!.querySelector<HTMLButtonElement>(`[data-retime-handle="${which}"][data-retime-at="${at}"]`)!;
const tip = () => document.querySelector('[data-test="retime-tip"]')?.textContent ?? null;
const items = () => within(screen.getByRole('menu', { name: 'Retime Controls' })).getAllByRole('menuitem').map(each => [each.getAttribute('aria-label'), (each as HTMLButtonElement).disabled]);
/** Do a command of the pop-up menu of a segment. */
const command = async (segment: number, name: string) => {
  fireEvent.click(speeds()[segment]);
  fireEvent.click(screen.getByRole('menuitem', { name }));
  await settle();
};
const edit = (fields: Record<string, unknown>) => ({ revision: 17, ops: [{ timeline: 't1', clip: 'c1', ...fields }] });
/** What a move of a speed point sends: the speed points of the clip whole, as an edit, which the engine gives to the clips linked to it that play as it does. */
const setPoints = (points: Retime) => ({ edit: 'set_speed_points', timeline: 't1', clip: 'c1', retime: points, linked: true });

describe('the Retime Controls of a clip', () => {
  it('are shown from the menu of the clip and with Ctrl+R on the clips chosen, and put away the same way or with their button', () => {
    wire(timelineOf([pointed(), clip('c2', 100, 50)]));
    expect(barOf()).toBeNull();
    expect(drawn.bars).toBe(0);
    const bar = open();
    expect(bar.getAttribute('aria-label')).toBe('Retime Controls');
    // Over its clip: placed by the style sheet from the frames of the clip, within the lane of its track.
    expect([bar.style.getPropertyValue('--start'), bar.style.getPropertyValue('--length'), bar.style.getPropertyValue('--header')]).toEqual(['10', '60', String(HEADER_WIDTH)]);
    expect([bar.style.top, bar.style.height]).toEqual([`${RULER_HEIGHT + 4}px`, '64px']);
    expect(barOf('c2')).toBeNull();
    fireEvent.click(bar.querySelector('[data-test="retime-close"]')!);
    expect(barOf()).toBeNull();
    // Ctrl+R shows them on the clips chosen, a clip of one speed too, and again puts them away.
    choose('c2');
    fireEvent.keyDown(root(), { key: 'r', ctrlKey: true });
    expect([barOf(), barOf('c2')].map(each => each !== null)).toEqual([false, true]);
    expect(told('c2')).toEqual([[100, 'forward', 0, 50]]);
    fireEvent.keyDown(root(), { key: 'r', ctrlKey: true });
    expect(barOf('c2')).toBeNull();
    // With no clip chosen it does nothing.
    fireEvent.pointerDown(document.querySelector('.timeline-lane')!, pointer(900)); fireEvent.pointerUp(root(), pointer(900));
    fireEvent.keyDown(root(), { key: 'r', ctrlKey: true });
    expect(document.querySelector('.retime-bar')).toBeNull();
  });

  it('say the speed of each segment, with arrows its way and a handle at each speed point', () => {
    wire(timelineOf([pointed()]));
    open();
    expect(told()).toEqual([[100, 'forward', 0, 20], [0, 'freeze', 20, 10], [-50, 'backward', 30, 30]]);
    expect(speeds().map(each => each.textContent)).toEqual(['100% ▾', '0% ▾', '−50% ▾']);
    // The arrows are nearer each other the faster a segment plays; a segment at another speed than the source's is marked.
    expect(segments().map(each => Number(each.style.getPropertyValue('--gap')))).toEqual([16, 16, 32]);
    expect(segments().map(each => each.hasAttribute('data-changed'))).toEqual([false, true, true]);
    expect([arrowGap(100), arrowGap(200), arrowGap(-50), arrowGap(10), arrowGap(10_000), arrowGap(0)]).toEqual([16, 8, 32, 40, 6, 16]);
    // An upper and a lower handle at each speed point, and an upper one at the end of the clip. The end of a freeze does not move.
    expect(handles().map(each => [each.dataset.retimeHandle, Number(each.dataset.retimeAt), each.disabled])).toEqual([
      ['upper', 20, false], ['lower', 20, false], ['upper', 30, true], ['lower', 30, false], ['upper', 60, false],
    ]);
    expect(handles().map(each => each.parentElement!.style.getPropertyValue('--at'))).toEqual(['20', '20', '30', '30', '60']);
  });

  it('keep off the controls of the clip beneath, and the clip does not say the range of its speeds while they are open', () => {
    wire(timelineOf([pointed(), clip('c2', 100, 50, { retime: retime([0, 2, 1], [20, 1, 1]) })]));
    const badge = (id: string) => clipOf(id).querySelector('.timeline-clip-speed')?.textContent ?? null;
    expect([badge('c1'), badge('c2')]).toEqual(['−50–100%', '100–200%']);
    const bar = open();
    // The bar says the speed of each segment; the badge of the clip, which would lie on the last of them, is gone, and that of the other clip stays.
    expect([badge('c1'), badge('c2')]).toEqual([null, '100–200%']);
    // Only the last segment says that it is: its speed leaves room for the handle at the end of the clip.
    expect(segments().map(each => each.hasAttribute('data-last'))).toEqual([false, false, true]);
    fireEvent.click(bar.querySelector('[data-test="retime-close"]')!);
    expect(badge('c1')).toBe('−50–100%');
    // Where each control lies is the style sheet's, which the DOM of the tests does not lay out: the rules are read.
    const css = readFileSync('src/studio/Studio.css', 'utf8');
    // The Keyframe button of the clip keeps its corner (2px in, 16px), the close button is at its left, and the handle at the end of the clip is under it.
    expect(blockOf(css, '.clip-keys-toggle')).toMatch(/top: 2px;\s+right: 2px;\s+width: 16px;\s+height: 16px;/);
    expect(blockOf(css, '.retime-close')).toMatch(/top: 2px;[^}]*right: 20px;/);
    expect(blockOf(css, '.retime-point[data-end] .retime-handle')).toMatch(/left: -9px;\s+top: 20px;/);
    // The key diamonds have the lowest strip of the clip (7px, 3px up); the speeds and the lower handles stand on it, and the upper handles are at the upper edge.
    expect(blockOf(css, '.clip-key')).toMatch(/bottom: 3px;[^}]*height: 7px;/);
    expect(blockOf(css, '.retime-speed')).toMatch(/bottom: 13px;[^}]*max-width: calc\(100% - 10px\);/);
    expect(blockOf(css, ".retime-handle[data-retime-handle='lower']")).toContain('bottom: 13px;');
    expect(blockOf(css, ".retime-handle[data-retime-handle='upper']")).toContain('top: 0;');
    expect(blockOf(css, '.retime-handle')).toMatch(/width: 9px;\s+height: 10px;/);
    expect(blockOf(css, '.retime-segment[data-last] .retime-speed')).toMatch(/left: calc\(50% - 2\.5px\);\s+max-width: calc\(100% - 15px\);/);
  });

  it('follow the clip as the project changes, and close where the clip is gone', () => {
    const { rerender, props } = wire(timelineOf([pointed(), clip('c2', 100, 50)]));
    open();
    rerender(<Page {...props} model={timelineOf([pointed({ retime: retime([0, 2, 1], [40, 1, 1]) }), clip('c2', 100, 50)])} />);
    expect(told()).toEqual([[200, 'forward', 0, 40], [100, 'forward', 40, 20]]);
    rerender(<Page {...props} model={timelineOf([clip('c2', 100, 50)])} />);
    expect(barOf()).toBeNull();
    // They stay closed where the clip comes back, by Undo.
    rerender(<Page {...props} model={timelineOf([pointed(), clip('c2', 100, 50)])} />);
    expect(barOf()).toBeNull();
  });

  it('are not offered for a title, nor on a timeline that is not given them', () => {
    const title = clip('c3', 100, 50, { source: { kind: 'title', text: 'Hello', style: { font: 'sans', size: 72, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } } });
    const first = wire(timelineOf([pointed(), title]));
    fireEvent.contextMenu(clipOf('c3'), { clientX: 200, clientY: 50 });
    expect(screen.getAllByRole('menuitem').length).toBeGreaterThan(0);
    expect(screen.queryByRole('menuitem', { name: 'Retime Controls' })).toBeNull();
    first.unmount();
    wire(timelineOf([pointed()]), { controls: false });
    fireEvent.contextMenu(clipOf('c1'), { clientX: 200, clientY: 50 });
    expect(screen.queryByRole('menuitem', { name: 'Retime Controls' })).toBeNull();
    fireEvent.keyDown(document.querySelector('[role="menu"]')!, { key: 'Escape' });
    choose();
    fireEvent.keyDown(root(), { key: 'r', ctrlKey: true });
    expect(document.querySelector('.retime-bar')).toBeNull();
  });
});

describe('the pop-up menu of a segment', () => {
  it('has the commands of the Retime Controls, those that would change nothing left out of reach', () => {
    // The playhead is at frame 25 of the timeline, frame 15 of the clip.
    wire(timelineOf([pointed()]));
    open();
    fireEvent.click(speeds()[2]);
    expect(items()).toEqual([
      ['Change Speed', false], ['Add Speed Point', false], ['Clear Speed Point', false], ['Freeze Frame', false], ['Reverse Segment', false],
      ['Reset Speed', false], ['Reset Clip', false],
    ]);
    // The speeds of Change Speed, in hundredths of the speed of the source.
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Speed' }));
    expect(within(screen.getByRole('menu', { name: 'Change Speed' })).getAllByRole('menuitem').map(each => each.getAttribute('aria-label'))).toEqual(CHANGE_SPEEDS.map(each => `${each}%`));
    expect(CHANGE_SPEEDS).toEqual([10, 25, 50, 75, 100, 110, 150, 200, 400, 800]);
    expect(screen.getByRole('menuitem', { name: '50%' }).hasAttribute('data-checked')).toBe(true);
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Retime Controls' }), { key: 'Escape' });
    // The first segment has no speed point to clear and is at the speed of the source; a freeze has no way to turn.
    fireEvent.click(speeds()[0]);
    expect(items().filter(([, disabled]) => disabled).map(([name]) => name)).toEqual(['Clear Speed Point', 'Reset Speed']);
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Retime Controls' }), { key: 'Escape' });
    fireEvent.click(speeds()[1]);
    expect(items().filter(([, disabled]) => disabled).map(([name]) => name)).toEqual(['Reverse Segment']);
  });

  it('leaves out of reach what needs the playhead on the clip, and a speed point where one is', () => {
    const { rerender, props } = wire(timelineOf([pointed()]), { frame: 200 });
    open();
    fireEvent.click(speeds()[0]);
    expect(items().filter(([, disabled]) => disabled).map(([name]) => name)).toEqual(['Add Speed Point', 'Clear Speed Point', 'Freeze Frame', 'Reset Speed']);
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Retime Controls' }), { key: 'Escape' });
    // Frame 30 of the timeline is frame 20 of the clip, where a speed point is: a freeze may still begin there.
    rerender(<Page {...props} frame={30} />);
    fireEvent.click(speeds()[2]);
    expect(items().filter(([, disabled]) => disabled).map(([name]) => name)).toEqual(['Add Speed Point']);
  });

  it('sends one edit for each command, at a frame of the clip counted from its first', async () => {
    const { call } = wire(timelineOf([pointed()]));
    open();
    await command(2, 'Add Speed Point');
    await command(2, 'Freeze Frame');
    await command(2, 'Reverse Segment');
    await command(2, 'Clear Speed Point');
    await command(2, 'Reset Speed');
    await command(0, 'Reset Clip');
    fireEvent.click(speeds()[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Speed' }));
    fireEvent.click(screen.getByRole('menuitem', { name: '150%' }));
    await settle();
    expect(applied(call)).toEqual([
      edit({ edit: 'add_speed_point', at: 15, linked: true }),
      // Held for a second of the timeline.
      edit({ edit: 'freeze_frame', at: 15, hold: 25, ripple: false, linked: true }),
      edit({ edit: 'reverse_segment', at: 30, linked: true }),
      edit({ edit: 'clear_speed_point', at: 30, linked: true }),
      edit({ edit: 'set_segment_speed', at: 30, speed: { num: 1, den: 1 }, ripple: false, linked: true }),
      edit({ edit: 'reset_speed', ripple: false, linked: true }),
      edit({ edit: 'set_segment_speed', at: 0, speed: { num: 3, den: 2 }, ripple: false, linked: true }),
    ]);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('ripples what follows with the Trim tool, and says a refusal of the engine where the timeline says them', async () => {
    const { call } = wire(timelineOf([pointed()]));
    open();
    fireEvent.keyDown(root(), { key: 't' });
    expect(root().getAttribute('data-tool')).toBe('trim');
    call.mockResolvedValueOnce(refusal('out_of_source'));
    await command(2, 'Reverse Segment');
    await command(2, 'Reset Clip');
    expect(applied(call)).toEqual([edit({ edit: 'reverse_segment', at: 30, linked: true }), edit({ edit: 'reset_speed', ripple: true, linked: true })]);
    // The first was refused: the sentence stays until the next edit goes.
    call.mockResolvedValueOnce(refusal('out_of_source'));
    await command(0, 'Reset Clip');
    expect(screen.getByRole('alert').textContent).toContain('The chosen range extends beyond the source.');
    // The controls stay open, as they were.
    expect(told()).toEqual([[100, 'forward', 0, 20], [0, 'freeze', 20, 10], [-50, 'backward', 30, 30]]);
  });
});

describe('the handles of a speed point', () => {
  it('change the speed of the segment before with the upper handle: one edit as it is let go, of a whole percent', async () => {
    const { call } = wire(timelineOf([pointed()]));
    open();
    const upper = handle('upper', 20);
    // A press and a release without a move send nothing.
    fireEvent.pointerDown(upper, pointer(100)); fireEvent.pointerUp(upper, pointer(101));
    expect(tip()).toBeNull();
    // 40 px at 4 px a frame: the first segment, 20 frames, is to last 30, at two thirds of its speed.
    fireEvent.pointerDown(upper, pointer(100));
    fireEvent.pointerMove(upper, pointer(140));
    expect(tip()).toBe('67%');
    expect(upper.parentElement!.style.getPropertyValue('--at')).toBe('30');
    // Nothing is sent before it is let go, and the segments are as they were.
    expect(applied(call)).toEqual([]);
    expect(told()[0]).toEqual([100, 'forward', 0, 20]);
    fireEvent.pointerUp(upper, pointer(140));
    await settle();
    expect(tip()).toBeNull();
    expect(applied(call)).toEqual([edit({ edit: 'set_segment_speed', at: 0, speed: { num: 67, den: 100 }, ripple: false, linked: true })]);
    // To the left it plays faster: 20 frames in 10.
    fireEvent.pointerDown(upper, pointer(100)); fireEvent.pointerMove(upper, pointer(60));
    expect(tip()).toBe('200%');
    fireEvent.pointerUp(upper, pointer(60));
    await settle();
    expect(applied(call)[1]).toEqual(edit({ edit: 'set_segment_speed', at: 0, speed: { num: 2, den: 1 }, ripple: false, linked: true }));
    // Dragged back to where it was, nothing is sent; the segment lasts a frame at the least.
    fireEvent.pointerDown(upper, pointer(100)); fireEvent.pointerMove(upper, pointer(140)); fireEvent.pointerMove(upper, pointer(100)); fireEvent.pointerUp(upper, pointer(100));
    fireEvent.pointerDown(upper, pointer(100)); fireEvent.pointerMove(upper, pointer(-900));
    expect(tip()).toBe('2000%');
    fireEvent.keyDown(barOf()!, { key: 'Escape' });
    expect(tip()).toBeNull();
    fireEvent.pointerUp(upper, pointer(-900));
    await settle();
    expect(applied(call)).toHaveLength(2);
  });

  it('change the last segment with the handle at the end of the clip, the way that it goes, and leave a freeze as long as it is', async () => {
    const { call } = wire(timelineOf([pointed()]));
    open();
    // The last segment goes backward at half the speed for 30 frames: in 15 frames, at the speed of the source, backward still.
    const end = handle('upper', 60);
    fireEvent.pointerDown(end, pointer(300)); fireEvent.pointerMove(end, pointer(240));
    expect(tip()).toBe('−100%');
    fireEvent.pointerUp(end, pointer(240));
    await settle();
    expect(applied(call)).toEqual([edit({ edit: 'set_segment_speed', at: 30, speed: { num: 1, den: 1 }, ripple: false, linked: true })]);
    // The handle at the end of the freeze does not move.
    const frozen = handle('upper', 30);
    fireEvent.pointerDown(frozen, pointer(100)); fireEvent.pointerMove(frozen, pointer(140)); fireEvent.pointerUp(frozen, pointer(140));
    await settle();
    expect(tip()).toBeNull();
    expect(applied(call)).toHaveLength(1);
  });

  it('move the speed point with the lower handle, the clip lasting as long: one gesture that sets the speed points whole', async () => {
    const { call } = wire(timelineOf([pointed()]));
    open();
    const lower = handle('lower', 20);
    // 20 px: the speed point goes from frame 20 to 25. The 20 frames of the source before it are shown in 25, at 80%; the freeze stays one.
    fireEvent.pointerDown(lower, pointer(100));
    fireEvent.pointerMove(lower, pointer(120));
    const moved = retime([0, 4, 5], [25, 0, 1], [30, -1, 2]);
    expect(tip()).toBe('80% · 0%');
    // The bar shows the speed points as the drag has them, whatever the timeline is given meanwhile.
    expect(told()).toEqual([[80, 'forward', 0, 25], [0, 'freeze', 25, 5], [-50, 'backward', 30, 30]]);
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [setPoints(moved)], gesture: { id: expect.any(String) } }]);
    // It stays strictly before the end of its segment.
    fireEvent.pointerMove(lower, pointer(900));
    expect(told()).toEqual([[69, 'forward', 0, 29], [0, 'freeze', 29, 1], [-50, 'backward', 30, 30]]);
    fireEvent.pointerMove(lower, pointer(120));
    fireEvent.pointerUp(lower, pointer(120));
    await settle();
    // The last move ends the gesture: the whole drag is one step to undo.
    const sent = applied(call);
    expect(sent.at(-1)).toEqual({ revision: 17, ops: [setPoints(moved)], gesture: { id: sent[0].gesture.id, end: true } });
    expect(sent.every(each => each.gesture.id === sent[0].gesture.id)).toBe(true);
    expect(tip()).toBeNull();
  });

  it('take a move of a speed point back with Escape, and where it is let go at the frame that it was at', async () => {
    const { call, rerender, props } = wire(timelineOf([pointed()]));
    open();
    const lower = handle('lower', 30);
    // Later, the freeze lasts 20 frames, and the 15 frames of the source that the clip goes back over are shown in 20, at 75%.
    fireEvent.pointerDown(lower, pointer(100)); fireEvent.pointerMove(lower, pointer(140));
    expect(tip()).toBe('0% · −75%');
    await settle();
    expect(applied(call)[0].ops).toEqual([setPoints(retime([0, 1, 1], [20, 0, 1], [40, -3, 4]))]);
    fireEvent.keyDown(barOf()!, { key: 'Escape' });
    await settle();
    expect(applied(call).at(-1)).toEqual({ revision: 17, ops: [setPoints(POINTS)], gesture: { id: expect.any(String), end: true } });
    expect(told()).toEqual([[100, 'forward', 0, 20], [0, 'freeze', 20, 10], [-50, 'backward', 30, 30]]);
    // The release after it does nothing more.
    fireEvent.pointerUp(lower, pointer(140));
    await settle();
    const before = applied(call).length;
    // The page is told of the project as the engine has it now, as after every edit: another gesture may begin.
    rerender(<Page {...props} revision={20} />);
    // Dragged away and back to its frame: the speed points are those that the clip had, and the gesture is taken back.
    fireEvent.pointerDown(lower, pointer(100)); fireEvent.pointerMove(lower, pointer(140)); fireEvent.pointerMove(lower, pointer(100)); fireEvent.pointerUp(lower, pointer(100));
    await settle();
    expect(applied(call).length).toBeGreaterThan(before);
    expect(applied(call).at(-1)).toEqual({ revision: 20, ops: [setPoints(POINTS)], gesture: { id: expect.any(String), end: true } });
  });

  it('keep the pictures before the segments that change where the lowest place of the path moves with a speed point', async () => {
    // c1 begins 500 frames into its media and goes 10 frames up, 30 down and 20 up: its path is lowest at frame 40, 20 frames below where it
    // begins, and that is where the clip begins in its media.
    const turned = retime([0, 1, 1], [10, -1, 1], [40, 1, 1]);
    const { call } = wire(timelineOf([clip('c1', 10, 60, { retime: turned, source: { kind: 'media', media: 'm1', in: { ticks: 500, scale: 25 } } })]));
    open();
    const lower = handle('lower', 40);
    // 28 px: the speed point goes from frame 40 to 47. The 30 frames down are gone in 37, at 81%, and the 20 frames up in 13, at 154%.
    fireEvent.pointerDown(lower, pointer(100)); fireEvent.pointerMove(lower, pointer(128));
    expect(tip()).toBe('−81% · 154%');
    const moved = retime([0, 1, 1], [10, -81, 100], [47, 77, 50]);
    await settle();
    // One edit, which leaves it to the engine to keep the place where the path begins: no operation that leaves `source.in` where it is.
    expect(applied(call)).toEqual([{ revision: 17, ops: [setPoints(moved)], gesture: { id: expect.any(String) } }]);
    // 37 frames at 81% go 29.97 frames: the lowest place of the path is 0.03 of a frame nearer to where it begins, which is then at the tick 19
    // above where the clip begins and no longer at the tick 20. So the clip begins one tick later, as the engine counts it and the page after it.
    const [before, after] = [courseOf(turned, 60)!, courseOf(moved, 60)!];
    const later = beginsLater(before, after, 'start', rate, 25);
    expect(later).toBe(1n);
    for (let frame = 0; frame < 10; frame++) {
      // Every frame of the first segment shows the tick that it showed. Had the clip begun where it did, it would show the tick before.
      expect(after.tick(500n + later, frame, rate, 25)).toBe(before.tick(500, frame, rate, 25));
      expect(after.tick(500, frame, rate, 25)).toBe(before.tick(500, frame, rate, 25) - 1n);
    }
    // Taken back, the speed points that the clip had put it back where it began, to the tick.
    fireEvent.keyDown(barOf()!, { key: 'Escape' });
    await settle();
    expect(applied(call).at(-1)).toEqual({ revision: 17, ops: [setPoints(turned)], gesture: { id: expect.any(String), end: true } });
    expect(beginsLater(before, after, 'start', rate, 25) + beginsLater(after, before, 'start', rate, 25)).toBe(0n);
  });

  it('take along the clips linked to it that play along the same speed points, and say a refusal that comes to the drag', async () => {
    const model = timelineOf([pointed({ link: 'g1' })], false, [clip('c5', 10, 60, { link: 'g1', retime: POINTS }), clip('c6', 100, 60, { link: 'g1', retime: POINTS })]);
    const { call } = wire(model);
    open();
    const lower = handle('lower', 20);
    call.mockResolvedValueOnce(refusal('out_of_source'));
    fireEvent.pointerDown(lower, pointer(100)); fireEvent.pointerMove(lower, pointer(120));
    const moved = retime([0, 4, 5], [25, 0, 1], [30, -1, 2]);
    // c5 lies with it and plays as it does, and c6 is linked too and lies elsewhere: the one edit names the clip, and the engine takes c5 along.
    expect(applied(call)[0].ops).toEqual([setPoints(moved)]);
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('The chosen range extends beyond the source.');
    // The drag is over: the bar shows the clip as it is.
    fireEvent.pointerMove(lower, pointer(140));
    expect(tip()).toBeNull();
    expect(told()).toEqual([[100, 'forward', 0, 20], [0, 'freeze', 20, 10], [-50, 'backward', 30, 30]]);
    fireEvent.pointerUp(lower, pointer(140));
    await settle();
    expect(applied(call)).toHaveLength(1);
  });
});

describe('a clip on a locked track', () => {
  it('shows its Retime Controls, and none of them changes it', async () => {
    const { call } = wire(timelineOf([pointed()], true));
    const bar = open();
    expect(bar.hasAttribute('data-locked')).toBe(true);
    expect(told()).toEqual([[100, 'forward', 0, 20], [0, 'freeze', 20, 10], [-50, 'backward', 30, 30]]);
    expect(handles().every(each => each.disabled)).toBe(true);
    fireEvent.click(speeds()[2]);
    expect(items().every(([, disabled]) => disabled)).toBe(true);
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Retime Controls' }), { key: 'Escape' });
    for (const which of ['upper', 'lower'] as const) {
      const each = handle(which, 20);
      fireEvent.pointerDown(each, pointer(100)); fireEvent.pointerMove(each, pointer(140)); fireEvent.pointerUp(each, pointer(140));
    }
    await settle();
    expect(tip()).toBeNull();
    expect(applied(call)).toEqual([]);
  });
});

describe('a timeline of a thousand clips', () => {
  it('mounts nothing of the Retime Controls until some are open, and draws them again neither as the playhead moves nor at a zoom', async () => {
    const model = bigTimeline();
    const bridge = readyBridge();
    const view = render(<Page model={model} bridge={bridge} frame={0} />);
    await settle();
    expect(document.querySelectorAll('[data-clip]').length).toBeGreaterThan(0);
    expect(document.querySelector('.retime-bar, .retime-segment, .retime-point, .retime-handle')).toBeNull();
    expect(drawn.bars).toBe(0);
    // One clip shows them: one bar, of one segment, and nothing for the others.
    const first = document.querySelector<HTMLElement>('[data-kind="video"] [data-clip]')!.dataset.clip!;
    open(first);
    expect(document.querySelectorAll('.retime-bar')).toHaveLength(1);
    expect(told(first)).toEqual([[100, 'forward', 0, 180]]);
    const before = drawn.bars;
    for (let frame = 1; frame <= 10; frame++) view.rerender(<Page model={model} bridge={bridge} frame={frame} />);
    expect(drawn.bars).toBe(before);
    // A zoom changes the pixels of a frame on the timeline, and the bar is placed from it by the style sheet.
    const scale = root().getAttribute('data-scale');
    fireEvent.keyDown(root(), { key: '=', ctrlKey: true });
    expect(root().getAttribute('data-scale')).not.toBe(scale);
    expect(drawn.bars).toBe(before);
  });
});
