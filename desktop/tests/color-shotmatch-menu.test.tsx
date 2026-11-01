// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Studio } from '../src/studio/Studio';
import { emptyGrade, type Grade, type ScopeStatistics } from '../src/studio/engine';
import { newCorrectorNode, type GradeChange } from '../src/studio/color/nodeEdits';
import { neutralMatch } from '../src/studio/color/shotmatch';
import { calls, corrector, fakeEngine, projectOf, settle, sent } from './colorEngine';
import { clip, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const rect = (width: number, height: number) => ({ left: 0, top: 0, width, height, right: width, bottom: height, x: 0, y: 0, toJSON() {} });
beforeEach(() => {
  likeABrowser(); localStorage.clear();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) { return rect(this.classList.contains('color-page') ? 1600 : 600, 300); });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete window.columbina; localStorage.clear(); });

const grade = (...ids: string[]): Grade => {
  const path = ['source', ...ids, 'output'];
  return { nodes: ids.map(id => corrector(id)), links: path.slice(1).map((to, i) => ({ from: path[i], to, kind: 'rgb', input: 0 })) };
};
const thumb = (id: string) => document.querySelector<HTMLElement>(`[data-test="thumbnails"] > [data-clip="${id}"]`)!;
const selected = () => ['c1', 'c2', 'c3'].filter(id => thumb(id).getAttribute('aria-selected') === 'true');
const clickClip = (id: string, ctrlKey = false) => {
  const frame = thumb(id).querySelector('.color-thumb-frame')!;
  fireEvent.pointerDown(frame, { button: 0, ctrlKey }); fireEvent.pointerUp(frame, { button: 0, ctrlKey }); fireEvent.click(frame, { ctrlKey });
};
const openMenu = (id: string) => fireEvent.contextMenu(thumb(id), { clientX: 30, clientY: 20 });

async function mount(empty = false) {
  const project = projectOf({ timelines: [timeline([
    clip('c1', 0, 100, { grade: empty ? emptyGrade() : grade('n1', 'n2') }), clip('c2', 100, 100, { grade: grade('n3', 'n4') }),
    clip('c3', 200, 100, { grade: empty ? emptyGrade() : grade('n5') }),
  ])] });
  const engine = fakeEngine(project);
  const base = engine.bridge.call.getMockImplementation()!;
  let measured: Promise<void> | null = null;
  let nextNode = 100;
  engine.bridge.call.mockImplementation(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'transaction.apply' && empty) {
      const ops = (params!.ops as GradeChange[]).map(input => {
        if ('op' in input) return input;
        if (input.edit !== 'add_serial') throw new Error('Only the default-node edit is supported in this test.');
        const node = newCorrectorNode(`n${nextNode++}`);
        return { op: 'add_node', grade: input.grade, node, links: [
          { from: 'source', to: node.id, kind: 'rgb', input: 0 }, { from: node.id, to: 'output', kind: 'rgb', input: 0 },
        ] };
      });
      return base(method, { ...params, ops });
    }
    if (method === 'scopes.statistics') {
      if (measured) await measured;
      const counts = Array<number>(1024).fill(0);
      for (let i = 100; i < 850; i++) counts[i] = 1;
      return { ok: true, result: { histogram: { bins: 1024, red: counts, green: counts, blue: counts, luma: counts }, red: { mean: 0.5 }, green: { mean: 0.5 }, blue: { mean: 0.5 } } as ScopeStatistics };
    }
    if (method === 'gallery.list') return { ok: true, result: { albums: [] } };
    return base(method, params);
  });
  const view = render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
  await settle();
  fireEvent.click(screen.getByRole('tab', { name: 'Color' })); await settle();
  return { ...engine, ...view, holdMeasurements: (until: Promise<void>) => { measured = until; } };
}

describe('Shot Match in the thumbnail menu', () => {
  it('keeps the multi-selection and matches each clip current node to the right-clicked clip', async () => {
    const { bridge } = await mount();
    clickClip('c1'); await settle();
    const choose = (id: string) => { fireEvent.pointerDown(document.querySelector(`[data-node="${id}"]`)!, { button: 0 }); fireEvent.pointerUp(window); };
    choose('n2'); await settle();
    clickClip('c2'); await settle(); choose('n4'); await settle();
    clickClip('c1', true); await settle();
    expect(selected()).toEqual(['c1', 'c2']);
    openMenu('c3');
    expect(selected()).toEqual(['c1', 'c2']);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Shot Match to This Clip' })); await settle();
    expect(selected()).toEqual(['c1', 'c2']);
    expect(calls(bridge, 'scopes.statistics').map(p => [p.grade.clip, p.node])).toEqual([['c3', 'n5'], ['c1', 'n2'], ['c2', 'n4']]);
    expect(sent(bridge)).toHaveLength(1);
    expect(sent(bridge)[0]).toEqual([
      { op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c1' }, id: 'n2', params: { shot_match: neutralMatch() } },
      { op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c2' }, id: 'n4', params: { shot_match: neutralMatch() } },
    ]);
    expect(thumb('c1').querySelector('[data-test="badge-matched"]')).not.toBeNull();
    expect(thumb('c2').querySelector('[data-test="badge-matched"]')).not.toBeNull();
    expect(thumb('c3').querySelector('[data-test="badge-matched"]')).toBeNull();
    expect(screen.getByText('Matched 2 clips.')).toBeTruthy();
    // Reset removes matches from the whole selection, as one transaction.
    openMenu('c1'); fireEvent.click(screen.getByRole('menuitem', { name: 'Reset Shot Match' })); await settle();
    expect(sent(bridge)).toHaveLength(2);
    expect(sent(bridge)[1]).toEqual([
      { op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c1' }, id: 'n2', params: { shot_match: null } },
      { op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c2' }, id: 'n4', params: { shot_match: null } },
    ]);
  });

  it('disables matching when only the reference is selected', async () => {
    const { bridge } = await mount();
    clickClip('c1'); await settle(); openMenu('c1');
    expect((screen.getByRole('menuitem', { name: 'Shot Match to This Clip' }) as HTMLButtonElement).disabled).toBe(true);
    expect(sent(bridge)).toHaveLength(0);
  });

  it('uses the current grade and its revision if the agent edits while the menu is open', async () => {
    const engine = await mount();
    clickClip('c1'); await settle(); openMenu('c3');
    engine.outside([{ op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c1' }, id: 'n1',
      params: { shot_match: { ...neutralMatch(), gain: { r: 0.9, g: 1, b: 1 } } } }]);
    await settle();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Shot Match to This Clip' })); await settle();
    const applied = calls(engine.bridge, 'transaction.apply');
    expect(applied).toHaveLength(1);
    expect(applied[0].revision).toBe(1);
    // Removing the old match is part of fitting its replacement. An old menu snapshot would
    // have seen no match and written a neutral gain of 1 over the agent's new adjustment.
    expect(applied[0].ops[0].params.shot_match.gain.r).toBeCloseTo(0.9, 2);
  });

  it.each(['page', 'project'] as const)('cancels a pending match when leaving the %s', async (leaving) => {
    const engine = await mount();
    let release!: () => void;
    engine.holdMeasurements(new Promise<void>(resolve => { release = resolve; }));
    clickClip('c1'); await settle(); openMenu('c3');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Shot Match to This Clip' })); await settle();
    expect(screen.getByText('Matching the selected clips…')).toBeTruthy();
    expect(calls(engine.bridge, 'scopes.statistics')).toHaveLength(1);
    if (leaving === 'page') fireEvent.click(screen.getByRole('tab', { name: 'Edit' }));
    else engine.restart();
    await settle(); release(); await settle();
    expect(sent(engine.bridge)).toHaveLength(0);
  });

  it.each(['c1', 'c3'])('stops if clip %s is removed while the menu is open', async (id) => {
    const engine = await mount();
    clickClip('c1'); await settle(); openMenu('c3');
    engine.outside([{ op: 'remove_clip', timeline: 't1', id }]); await settle();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Shot Match to This Clip' })); await settle();
    expect(sent(engine.bridge)).toHaveLength(0);
    expect(screen.getByText('A selected clip or the reference is no longer available. Choose the clips again.')).toBeTruthy();
  });

  it('restores empty grades immediately when leaving Color while the view read stays pending', async () => {
    const engine = await mount(true);
    vi.useFakeTimers();
    let release!: () => void;
    engine.holdMeasurements(new Promise<void>(resolve => { release = resolve; }));
    clickClip('c1'); await settle(); openMenu('c3');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Shot Match to This Clip' })); await settle();
    expect(sent(engine.bridge)).toHaveLength(1);
    expect(calls(engine.bridge, 'scopes.statistics')).toHaveLength(1);
    fireEvent.click(screen.getByRole('tab', { name: 'Edit' })); await settle();
    const grades = () => engine.project().timelines[0].tracks.flatMap(track => track.kind === 'subtitle' ? [] : track.clips)
      .filter(clip => clip.id !== 'c2').map(clip => clip.grade);
    expect(grades()).toEqual([emptyGrade(), emptyGrade()]);
    expect(sent(engine.bridge)).toHaveLength(2);
    expect(calls(engine.bridge, 'transaction.apply')[1].gesture).toMatchObject({ end: true });
    await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
    release(); await settle();
    expect(grades()).toEqual([emptyGrade(), emptyGrade()]);
    expect(sent(engine.bridge)).toHaveLength(2);
  });
});
