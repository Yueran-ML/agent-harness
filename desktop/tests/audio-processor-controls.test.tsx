// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Knob } from '../src/studio/audio/Knob';
import { likeABrowser } from './mounted';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });
it('uses Alt and Shift tenth precision, absolute input, reset and cancel', () => {
  const change = vi.fn(), cancel = vi.fn(), commit = vi.fn();
  render(<Knob label="Trim" value={0} min={-20} max={20} step={1} fallback={0} onStart={() => true} onChange={change} onEnd={() => {}} onCancel={cancel} onCommit={commit} />);
  const knob = screen.getByRole('slider');
  fireEvent.pointerDown(knob, { button: 0, clientY: 100 }); fireEvent.pointerMove(knob, { clientY: 90, altKey: true });
  expect(change).toHaveBeenLastCalledWith(1);
  fireEvent.pointerCancel(knob); expect(cancel).toHaveBeenCalled();
  fireEvent.pointerDown(knob, { button: 0, clientY: 100 }); fireEvent.pointerMove(knob, { clientY: 90, shiftKey: true }); expect(change).toHaveBeenLastCalledWith(1);
  fireEvent.pointerUp(knob); fireEvent.doubleClick(screen.getByText('Trim')); expect(commit).toHaveBeenLastCalledWith(0);
  const input = screen.getByRole('spinbutton'); fireEvent.change(input, { target: { value: '7.25' } }); fireEvent.keyDown(input, { key: 'Enter' }); expect(commit).toHaveBeenLastCalledWith(7.25);
  fireEvent.change(input, { target: { value: '14' } }); fireEvent.keyDown(input, { key: 'Escape' }); expect(commit).toHaveBeenCalledTimes(2);
  fireEvent.keyDown(input, { key: 'ArrowUp', altKey: true }); expect(commit).toHaveBeenLastCalledWith(.1);
  fireEvent.pointerDown(input, { button: 0, clientX: 100 }); fireEvent.pointerMove(input, { clientX: 120, altKey: true });
  expect(change).toHaveBeenLastCalledWith(2); fireEvent.pointerUp(input);
});
