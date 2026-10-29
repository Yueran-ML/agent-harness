// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { MediaPool } from '../src/studio/MediaPool';
import { applyOp, type Bin, type EngineAnswer, type Media, type Op, type Project } from '../src/studio/engine';
import { MEDIA_DRAG } from '../src/studio/media/drag';

const media = (id: string, bin: string | null = null): Media => ({
  id, name: `Take ${id}`, bin, mark_in: { ticks: 25, scale: 25 }, mark_out: { ticks: 75, scale: 25 }, path: `C:/media/${id}.mov`,
  identity: { size: 123, modified_us: 0, digest: '0'.repeat(64) },
  facts: { duration: { ticks: 100, scale: 25 }, video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'prores', bits: 10, rotation: 0 }, audio: { rate: 48000, channels: 2, codec: 'pcm_s24le' } },
});
function pool(items = [media('m1'), media('m2'), media('m3')], bins: Bin[] = []) {
  const load = vi.fn();
  const importing = vi.fn();
  const call = vi.fn<(...args: any[]) => Promise<EngineAnswer>>();
  function Harness() {
    const [project, setProject] = useState<Project>({ schema: 2, next_id: 10, media: items, bins, timelines: [] });
    call.mockImplementation(async (method, params) => {
      if (method !== 'transaction.apply') throw new Error(method);
      const done: Op[] = params.ops.map((op: Op) => op.op === 'add_bin' ? { ...op, bin: { ...op.bin, id: 'b10' } } : op);
      setProject(old => done.reduce(applyOp, old));
      return { ok: true, result: { revision: 1, next_id: 11, done, names: {}, clamped: [] } };
    });
    return <MediaPool media={project.media} bins={project.bins} timelines={project.timelines} proxies={{}} loaded={null}
      bridge={{ call }} session={1} onImport={importing} onLoad={load} onMakeAgain={() => {}} />;
  }
  render(<Harness />);
  return { call, load, importing };
}
const row = (id: string) => document.querySelector<HTMLElement>(`[data-media="${id}"]`)!;
const selected = () => [...document.querySelectorAll('[data-media][aria-selected="true"]')].map(el => el.getAttribute('data-media'));
const bin = (id: string) => document.querySelector<HTMLElement>(`[data-bin="${id}"]`)!;
const settle = () => act(async () => { await Promise.resolve(); });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe('the media pool used for editing', () => {
  it('selects without loading, extends and toggles a selection, and loads only at double click', () => {
    const { load } = pool();
    expect(row('m1').textContent).toContain('Take m1');
    fireEvent.click(row('m1'));
    fireEvent.click(row('m3'), { shiftKey: true });
    expect(selected()).toEqual(['m1', 'm2', 'm3']);
    fireEvent.click(row('m2'), { ctrlKey: true });
    expect(selected()).toEqual(['m1', 'm3']);
    expect(load).not.toHaveBeenCalled();
    fireEvent.doubleClick(row('m3'));
    expect(load).toHaveBeenCalledWith('m3');
  });

  it('renames in the project without changing the source path, and Enter during IME composition does not commit', async () => {
    const { call } = pool();
    fireEvent.click(row('m1'));
    fireEvent.keyDown(row('m1'), { key: 'F2' });
    const field = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(field, { target: { value: '海边采访' } });
    fireEvent.keyDown(field, { key: 'Enter', isComposing: true });
    expect(call).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { ops: [{ op: 'set_media', id: 'm1', name: '海边采访' }] });
    expect(row('m1').textContent).toContain('海边采访');
    expect(row('m1').title).toBe('C:/media/m1.mov');
  });

  it('removes selected media together and keeps the selection when the engine says one is in use', async () => {
    const { call } = pool();
    fireEvent.click(row('m1'));
    fireEvent.click(row('m2'), { ctrlKey: true });
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'used by t1', data: { reason: 'in_use' } } });
    fireEvent.keyDown(row('m2'), { key: 'Delete' });
    await settle();
    expect(screen.getByRole('alert').textContent).toMatch(/timeline/i);
    expect(selected()).toEqual(['m1', 'm2']);
    fireEvent.keyDown(row('m2'), { key: 'Delete' });
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { ops: [{ op: 'remove_media', id: 'm1' }, { op: 'remove_media', id: 'm2' }] });
    expect(row('m1')).toBeNull();
    expect(row('m3')).not.toBeNull();
  });

  it('creates nested bins, imports into the chosen bin and refuses removing a nonempty bin', async () => {
    const { call, importing } = pool([media('m1', 'b1')], [{ id: 'b1', name: 'Day 1', parent: null }]);
    fireEvent.click(bin('b1'));
    fireEvent.click(screen.getByRole('button', { name: 'New bin' }));
    const field = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(field, { target: { value: 'Interviews' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { ops: [{ op: 'add_bin', bin: { id: '', name: 'Interviews', parent: 'b1' } }] });
    fireEvent.click(bin('b10'));
    fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    expect(importing).toHaveBeenCalledWith('b10');
    fireEvent.click(bin('b1'));
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'bin is not empty', data: { reason: 'not_empty' } } });
    fireEvent.keyDown(bin('b1'), { key: 'Delete' });
    await settle();
    expect(bin('b1')).not.toBeNull();
    expect(screen.getByRole('alert').textContent).toMatch(/bin/i);
  });

  it('moves a dragged selection into a bin as one transaction and ignores a drag from another project session', async () => {
    const { call } = pool(undefined, [{ id: 'b1', name: 'Selects', parent: null }]);
    fireEvent.click(row('m1'));
    fireEvent.click(row('m2'), { ctrlKey: true });
    const values = new Map<string, string>();
    const dataTransfer = { setData: (key: string, value: string) => values.set(key, value), getData: (key: string) => values.get(key) ?? '', types: [MEDIA_DRAG], effectAllowed: '', dropEffect: '' };
    fireEvent.dragStart(row('m2'), { dataTransfer });
    expect(JSON.parse(values.get(MEDIA_DRAG)!)).toEqual({ session: 1, media: ['m1', 'm2'] });
    fireEvent.drop(bin('b1'), { dataTransfer });
    await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { ops: [{ op: 'set_media', id: 'm1', bin: 'b1' }, { op: 'set_media', id: 'm2', bin: 'b1' }] });
    call.mockClear();
    values.set(MEDIA_DRAG, JSON.stringify({ session: 2, media: ['m3'] }));
    fireEvent.drop(bin('b1'), { dataTransfer });
    expect(call).not.toHaveBeenCalled();
  });

  it('renames and removes an empty bin, and returns to Master after removal', async () => {
    const { call } = pool([media('m1')], [{ id: 'b1', name: 'Unused', parent: null }]);
    fireEvent.click(bin('b1'));
    fireEvent.keyDown(bin('b1'), { key: 'F2' });
    const field = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(field, { target: { value: 'Selects' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { ops: [{ op: 'set_bin', id: 'b1', name: 'Selects' }] });
    expect(bin('b1').textContent).toContain('Selects');
    fireEvent.keyDown(bin('b1'), { key: 'Delete' });
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { ops: [{ op: 'remove_bin', id: 'b1' }] });
    expect(bin('b1')).toBeNull();
    expect(bin('').getAttribute('aria-selected')).toBe('true');
    expect(row('m1')).not.toBeNull();
  });

  it('moves by visual rows in the thumbnail grid and extends the selection with Shift', () => {
    vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} });
    pool([media('m1'), media('m2'), media('m3'), media('m4'), media('m5')]);
    fireEvent.click(screen.getByRole('button', { name: 'Thumbnail view' }));
    for (let i = 0; i < 5; i++) Object.defineProperty(row(`m${i + 1}`), 'offsetTop', { value: Math.floor(i / 2) * 100 });
    fireEvent.click(row('m2'));
    fireEvent.keyDown(row('m2'), { key: 'ArrowDown', shiftKey: true });
    expect(selected()).toEqual(['m2', 'm3', 'm4']);
    expect(document.activeElement).toBe(row('m4'));
    fireEvent.keyDown(row('m4'), { key: 'ArrowUp' });
    expect(selected()).toEqual(['m2']);
    fireEvent.keyDown(row('m2'), { key: 'ArrowRight' });
    expect(selected()).toEqual(['m3']);
  });

  it('shows metadata and source marks for the selected media', () => {
    pool();
    fireEvent.click(row('m1'));
    const details = screen.getByRole('region', { name: 'Media details' });
    for (const text of ['C:/media/m1.mov', 'prores', 'pcm_s24le', '1920×1080', '48000', '00:00:01:00', '00:00:03:00']) {
      expect(details.textContent).toContain(text);
    }
  });

  it('requests thumbnails only for visible video and reuses them after switching views', async () => {
    const observers: { visible: () => void; disconnect: ReturnType<typeof vi.fn> }[] = [];
    vi.stubGlobal('IntersectionObserver', class {
      disconnect = vi.fn();
      observe = vi.fn();
      constructor(told: IntersectionObserverCallback) {
        observers.push({ visible: () => told([{ isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver), disconnect: this.disconnect });
      }
    });
    const { call } = pool();
    call.mockResolvedValue({ ok: true, result: { width: 160, height: 90, png: 'aGVsbG8=' } });
    expect(observers).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Thumbnail view' }));
    expect(observers).toHaveLength(3);
    expect(call).not.toHaveBeenCalled();
    await act(async () => observers[1].visible());
    expect(call).toHaveBeenCalledWith('media.thumbnail', { media: 'm2', frame: 25, height: 90 });
    expect(row('m1').querySelector('img')).toBeNull();
    expect(row('m2').querySelector('img')?.src).toBe('data:image/png;base64,aGVsbG8=');
    fireEvent.click(screen.getByRole('button', { name: 'List view' }));
    expect(observers.every(observer => observer.disconnect.mock.calls.length > 0)).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'Thumbnail view' }));
    await act(async () => observers[4].visible());
    expect(call).toHaveBeenCalledTimes(1);
    expect(row('m2').querySelector('img')).not.toBeNull();
  });
});
