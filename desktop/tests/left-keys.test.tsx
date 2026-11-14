// @vitest-environment jsdom
/**
 * The keys that the menu of the window had (Ctrl with plus, minus or 0, F11 and F12) go to the page
 * first; one that nothing on the page took is handed back to the shell, once every listener has had
 * it, also one added after the hook's own, as a page of the studio adds its own.
 */
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { menuKey, useLeftKeys } from '../src/app/leftKeys';

function Page() { useLeftKeys(); return null; }
let leftKey: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.useFakeTimers();
  leftKey = vi.fn(async () => true);
  vi.stubGlobal('columbina', { platform: 'win32', leftKey });
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('the keys that the menu had', () => {
  it('are known by key and code, without Alt or the Windows key', () => {
    expect(menuKey({ key: '=', code: 'Equal', ctrlKey: true, altKey: false, metaKey: false })).toBe(true);
    expect(menuKey({ key: 'Insert', code: 'Numpad0', ctrlKey: true, altKey: false, metaKey: false })).toBe(true);
    expect(menuKey({ key: 'F11', code: 'F11', ctrlKey: false, altKey: false, metaKey: false })).toBe(true);
    expect(menuKey({ key: 'F10', code: 'F10', ctrlKey: false, altKey: false, metaKey: false })).toBe(false);
    expect(menuKey({ key: '=', code: 'Equal', ctrlKey: true, altKey: true, metaKey: false })).toBe(false);
    expect(menuKey({ key: '=', code: 'Equal', ctrlKey: false, altKey: false, metaKey: false })).toBe(false);
  });

  it('are handed back where nothing took them, and kept where a listener added later took them', () => {
    render(<Page />);
    // A listener of the studio comes after the hook's, and takes F11 (Replace) and Ctrl+= (zoom of the timeline).
    const studio = (event: KeyboardEvent) => { if (event.key === 'F11' || event.key === '=') event.preventDefault(); };
    window.addEventListener('keydown', studio);
    fireEvent.keyDown(document.body, { key: 'F11', code: 'F11' });
    fireEvent.keyDown(document.body, { key: '=', code: 'Equal', ctrlKey: true });
    fireEvent.keyDown(document.body, { key: '0', code: 'Digit0', ctrlKey: true });
    fireEvent.keyDown(document.body, { key: 'a', code: 'KeyA', ctrlKey: true });
    expect(leftKey).not.toHaveBeenCalled();
    vi.runAllTimers();
    expect(leftKey.mock.calls).toEqual([[{ key: '0', code: 'Digit0', control: true, shift: false, alt: false, meta: false }]]);
    window.removeEventListener('keydown', studio);
    fireEvent.keyDown(document.body, { key: 'F11', code: 'F11' });
    // Full screen is asked once for a key held down.
    fireEvent.keyDown(document.body, { key: 'F11', code: 'F11', repeat: true });
    vi.runAllTimers();
    expect(leftKey).toHaveBeenCalledTimes(2);
    expect(leftKey.mock.calls[1][0]).toMatchObject({ key: 'F11' });
  });

  it('are not handed back on macOS, where the menu keeps them, nor after the page is gone', () => {
    vi.stubGlobal('columbina', { platform: 'darwin', leftKey });
    const mac = render(<Page />);
    fireEvent.keyDown(document.body, { key: 'F11', code: 'F11' });
    vi.runAllTimers();
    expect(leftKey).not.toHaveBeenCalled();
    mac.unmount();
    vi.stubGlobal('columbina', { platform: 'win32', leftKey });
    const page = render(<Page />);
    fireEvent.keyDown(document.body, { key: 'F11', code: 'F11' });
    page.unmount();
    vi.runAllTimers();
    expect(leftKey).not.toHaveBeenCalled();
  });
});
