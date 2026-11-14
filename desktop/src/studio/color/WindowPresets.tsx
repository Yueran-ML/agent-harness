import { useCallback, useEffect, useRef, useState } from 'react';
import { Ellipsis } from 'lucide-react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { EngineBridge } from '../../bridge';
import type { Messages } from '../../messages/en';
import { callEngine, EngineError, type Target, type WindowPreset } from '../engine';
import { ColorMenu, type Menu, type MenuItem } from './ColorMenu';

/**
 * Window presets (Reference Manual 20, "Saving Window Presets", pp. 3198–3199): the Window
 * palette's option menu saves the windows of the current node as a new preset, updates or deletes
 * one, and recalls one, which "overwrite[s] whatever other windows were set up in that node". The
 * presets are this user's, shared by every project (plan Q10), and kept by the engine
 * (`window_presets.*`, PROTOCOL.md "Window presets"): a preset holds the windows as they are on
 * the frame of the playhead, every parameter fixed (author decision 2026-11-01); recalling one is
 * one undo step of the project, while saving, updating and deleting are not steps of its history.
 */
export type PresetBridge = Pick<EngineBridge, 'call' | 'onEvent'>;

/** This user's presets, listed as they are first shown, again whenever the engine says `window_presets.changed`, and when another engine is opened. */
export function useWindowPresets(bridge: PresetBridge | null, session: number) {
  const [presets, setPresets] = useState<WindowPreset[]>([]);
  const asking = useRef(0);
  const list = useCallback(async () => {
    if (!bridge) return;
    const asked = ++asking.current;
    try {
      const answer = await callEngine(bridge, 'window_presets.list');
      if (asked === asking.current) setPresets(Array.isArray(answer?.presets) ? answer.presets : []);
    } catch {
      if (asked === asking.current) setPresets([]);
    }
  }, [bridge]);
  useEffect(() => {
    if (!bridge) return undefined;
    void list();
    return bridge.onEvent((event) => {
      if ((event.kind === 'notification' && event.method === 'window_presets.changed') || event.kind === 'opened') void list();
    });
  }, [bridge, session, list]);
  return presets;
}

/** What the option menu needs of the Window palette: the current node, the frame of the playhead as its keys count it, and whether its windows can be changed. */
export interface PresetPlace {
  bridge: PresetBridge | null;
  session: number;
  target: Target | null;
  node: string | null;
  frame: number;
  revision: number;
  /** Whether the node's windows can be changed now (an enabled, unlocked corrector, nothing pending). */
  enabled: boolean;
  /** How many windows the node has: Save as New Preset and Update Preset need one at least (p. 3199). */
  windows: number;
}

type Said = Messages['hint']['studio']['colorPage']['windowPresets'];
/** What a refusal of the engine says, in the page's words; its own message otherwise. */
export function presetProblem(said: Said, failure: unknown): string {
  if (!(failure instanceof EngineError)) return failure instanceof Error ? failure.message : String(failure);
  const data = (failure.data ?? {}) as { reason?: string; name?: string };
  if (data.reason === 'name_taken') return said.nameTaken.replace('{name}', data.name ?? '');
  if (data.reason === 'no_windows') return said.noWindows;
  if (data.reason === 'missing' && (failure.data as { what?: string }).what === 'preset') return said.missing;
  if (data.reason === 'window_presets') return said.library;
  return failure.message;
}

/**
 * The option menu's items (p. 3199): Save as New Preset, Update Preset ▸ and Delete Preset ▸ (each
 * a submenu of the presets by name), then the presets themselves, chosen to recall one. Without
 * presets, the menu says so and its submenus are off.
 */
export function presetItems(words: Messages['label']['studio']['colorPage']['windowPresets'], presets: readonly WindowPreset[], place: Pick<PresetPlace, 'enabled' | 'windows'>,
  act: { saveNew: () => void; update: (id: string) => void; remove: (id: string) => void; recall: (id: string) => void }): MenuItem[] {
  const saving = place.enabled && place.windows > 0;
  const none = presets.length === 0;
  return [
    { id: 'window-preset-save', label: words.saveNew, disabled: !saving, run: act.saveNew },
    { id: 'window-preset-update', label: words.update, disabled: !saving || none,
      items: presets.map(preset => ({ id: `window-preset-update-${preset.id}`, label: preset.name, run: () => act.update(preset.id) })) },
    { id: 'window-preset-delete', label: words.delete, disabled: none,
      items: presets.map(preset => ({ id: `window-preset-delete-${preset.id}`, label: preset.name, run: () => act.remove(preset.id) })) },
    ...(none ? [{ id: 'window-preset-none', label: words.none, disabled: true }]
      : presets.map(preset => ({ id: `window-preset-${preset.id}`, label: preset.name, disabled: !place.enabled, run: () => act.recall(preset.id) }))),
  ];
}

/** The Window palette's option button (…) and its menu of presets, with the dialog that names a new one. */
export function WindowPresets({ place, onRecalled }: {
  place: PresetPlace;
  /** Told the preset's first window once a recall is done, if the palette still shows the node it went to. */
  onRecalled?: (window: string) => void;
}) {
  const messages = useWords();
  const words = messages.label.studio.colorPage.windowPresets;
  const said = messages.hint.studio.colorPage.windowPresets;
  const presets = useWindowPresets(place.bridge, place.session);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [naming, setNaming] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const now = useRef(place);
  now.current = place;
  const close = useCallback(() => setMenu(null), []);
  const here = () => {
    const { bridge, target, node } = now.current;
    return bridge && target && node ? { bridge, target, node } : null;
  };
  const fail = (failure: unknown) => setProblem(presetProblem(said, failure));
  const save = async (name: string) => {
    const at = here();
    if (!at) return;
    try {
      await callEngine(at.bridge, 'window_presets.save', { name, grade: at.target, node: at.node, frame: now.current.frame });
      setNaming(null); setProblem(null);
    } catch (failure) { fail(failure); }
  };
  const act = {
    saveNew: () => { setProblem(null); setNaming(''); },
    update: (preset: string) => {
      const at = here();
      if (at) callEngine(at.bridge, 'window_presets.update', { preset, grade: at.target, node: at.node, frame: now.current.frame })
        .then(() => setProblem(null), fail);
    },
    // No warning, and no undo (p. 3199).
    remove: (preset: string) => {
      const at = here();
      if (at) callEngine(at.bridge, 'window_presets.delete', { preset }).then(() => setProblem(null), fail);
    },
    recall: (preset: string) => {
      const at = here();
      if (!at) return;
      const first = presets.find(each => each.id === preset)?.windows[0]?.id;
      const key = JSON.stringify([at.target, at.node]);
      callEngine(at.bridge, 'window_presets.apply', { preset, grade: at.target, node: at.node, revision: now.current.revision })
        .then(() => {
          setProblem(null);
          if (first !== undefined && JSON.stringify([now.current.target, now.current.node]) === key) onRecalled?.(first);
        }, fail);
    },
  };
  const open = (box: DOMRect) => setMenu({ x: box.left, y: box.bottom, label: words.options, items: presetItems(words, presets, place, act) });
  const name = naming?.trim() ?? '';
  return <>
    <button type="button" className="icon-btn" aria-label={words.options} title={words.options} aria-haspopup="menu" aria-expanded={menu !== null}
      disabled={!place.bridge || !place.target || !place.node} data-test="window-options"
      onClick={event => (menu ? close() : open(event.currentTarget.getBoundingClientRect()))}><Ellipsis size={14} /></button>
    {menu && <ColorMenu menu={menu} onClose={close} />}
    {problem !== null && naming === null && <p role="alert" data-test="window-preset-problem">{problem}</p>}
    {naming !== null && <Sheet title={words.saveNew} onClose={() => setNaming(null)} foot={<>
      <button type="button" className="btn" onClick={() => setNaming(null)}>{messages.label.cancel}</button>
      <button type="button" className="btn btn-main" disabled={name === ''} onClick={() => void save(name)} data-test="window-preset-ok">{words.ok}</button>
    </>}>
      <form onSubmit={(event) => { event.preventDefault(); if (name !== '') void save(name); }}>
        <label className="setting-name" htmlFor="window-preset-name">{words.name}</label>
        <input id="window-preset-name" className="field" value={naming} maxLength={100} data-first autoComplete="off" spellCheck={false}
          onChange={event => setNaming(event.target.value)} data-test="window-preset-name" />
        {problem !== null && <p role="alert" data-test="window-preset-problem">{problem}</p>}
      </form>
    </Sheet>}
  </>;
}
