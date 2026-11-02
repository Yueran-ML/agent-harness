import {
  useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState,
  type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode, type RefObject, type WheelEvent,
} from 'react';
import { createPortal } from 'react-dom';
import { Ellipsis, Expand, SlidersHorizontal, X } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Monitor } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { ColorMenu, type Menu } from './ColorMenu';
import {
  engineSettings, fitted, GAMUT_NAMES, GAMUTS, LAYOUTS, loadScopes, overlayOf, pictureSize, SCOPE_KINDS, scopesStorageKey, storeScopes,
  type FloatRect, type Gamut, type Layout, type PaneSettings, type ReferenceLevels, type ScopeKind, type ScopeOptions, type ScopesSettings,
} from './scopeSettings';
import { useScope, type ScopeBridge } from './useScope';
import './scopes.css';

/**
 * What is written the same in every language, as DaVinci Resolve writes it: the channels of a
 * mode, the targets of the bars, the HDR scale and the CIE diagram.
 */
const NAMES = { y: 'Y', rgb: 'RGB', yrgb: 'YRGB', t75: '75%', t100: '100%', hdr: 'HDR (ST.2084/HLG)', cie1931: 'CIE 1931 xy' } as const;
/** How far Alt and the wheel zoom a graph (p. 3018), and how far in it goes. */
const ZOOM_STEP = 1.15;
const ZOOM_MOST = 8;
/** How long a pane rests at a size before its picture is asked for at that size. */
const RESIZE_MS = 150;

/** What the scopes need of the page. */
export interface ScopesContext {
  bridge: ScopeBridge;
  /** The engine open, or null. */
  session: number | null;
  /** The monitor whose frames are measured: on the Color page, the timeline's. */
  view: Monitor;
  /** Whether the viewer has something loaded: until then a scope has nothing to measure. */
  loaded: boolean;
  /** Whether the page is in the desktop app, which reaches the engine; in a browser the scopes say they are not there. */
  native: boolean;
}

const ratio = () => (typeof window.devicePixelRatio === 'number' && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1);

/** The size of an element in pixels of the page, as it is now and after it is resized (once it rests). */
function useSize(element: RefObject<HTMLElement | null>): { width: number; height: number } | null {
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  useLayoutEffect(() => {
    const node = element.current;
    if (!node) return undefined;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const measure = () => {
      const box = node.getBoundingClientRect();
      if (box.width <= 0 || box.height <= 0) return;
      setSize(old => (old && old.width === box.width && old.height === box.height ? old : { width: box.width, height: box.height }));
    };
    measure();
    const later = () => { if (timer !== null) clearTimeout(timer); timer = setTimeout(measure, RESIZE_MS); };
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(later) : null;
    observer?.observe(node);
    return () => { observer?.disconnect(); if (timer !== null) clearTimeout(timer); };
  }, []);
  return size;
}

/** A slider of a scope's controls, 0–1 unless it says otherwise, its value shown beside it. */
function Slider({ label, value, onChange, min = 0, max = 1, step = 0.01, digits = 2, test }: {
  label: string; value: number; onChange: (value: number) => void; min?: number; max?: number; step?: number; digits?: number; test: string;
}) {
  return <label className="scope-control scope-slider">
    <span>{label}</span>
    <input type="range" min={min} max={max} step={step} value={value} aria-label={label} data-test={test}
      onChange={event => onChange(Number(event.target.value))} />
    <output>{value.toFixed(digits)}</output>
  </label>;
}
function Check({ label, checked, onChange, test }: { label: string; checked: boolean; onChange: (checked: boolean) => void; test: string }) {
  return <label className="scope-control scope-check">
    <input type="checkbox" checked={checked} onChange={event => onChange(event.target.checked)} data-test={test} />
    <span>{label}</span>
  </label>;
}
function Choice<T extends string>({ label, value, choices, onChange, test }: {
  label: string; value: T; choices: readonly (readonly [T, string])[]; onChange: (value: T) => void; test: string;
}) {
  return <div className="scope-control scope-choice" role="radiogroup" aria-label={label} data-test={test}>
    {choices.map(([each, name]) => <button key={each} type="button" role="radio" aria-checked={each === value} data-value={each}
      onClick={() => onChange(each)}>{name}</button>)}
  </div>;
}

/**
 * The Custom Controls of a pane's scope (pp. 3019–3022), opened by the Option button beside the
 * Name drop-down and put away by a press outside it or Escape.
 */
function CustomControls({ pane, onPane, onClose }: { pane: PaneSettings; onPane: (pane: PaneSettings) => void; onClose: () => void }) {
  const labels = useWords().label.studio.colorPage.scopes;
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const away = (event: PointerEvent) => {
      if (event.target instanceof Node && (box.current?.contains(event.target) || (event.target as Element).closest?.('[data-scope-controls-button]'))) return;
      onClose();
    };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose(); } };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', escape, true);
    return () => { window.removeEventListener('pointerdown', away, true); window.removeEventListener('keydown', escape, true); };
  }, [onClose]);
  const set = <K extends ScopeKind>(kind: K, change: Partial<PaneSettings[K]>) => onPane({ ...pane, [kind]: { ...pane[kind], ...change } });
  const reference = (kind: 'waveform' | 'parade' | 'histogram', levels: ReferenceLevels) => <>
    <Check label={labels.reference} checked={levels.shown} onChange={shown => set(kind, { reference: { ...levels, shown } })} test="scope-reference" />
    {levels.shown && <>
      <Slider label={labels.low} value={levels.low} min={0} max={1023} step={1} digits={0} test="scope-reference-low"
        onChange={low => set(kind, { reference: { ...levels, low: Math.min(low, levels.high) } })} />
      <Slider label={labels.high} value={levels.high} min={0} max={1023} step={1} digits={0} test="scope-reference-high"
        onChange={high => set(kind, { reference: { ...levels, high: Math.max(high, levels.low) } })} />
    </>}
  </>;
  const graticule = (kind: ScopeKind) => <Slider label={labels.graticule} value={pane[kind].graticule} test="scope-graticule"
    onChange={value => set(kind, { graticule: value })} />;
  const brightness = (kind: ScopeKind, label = labels.kinds[kind]) => <Slider label={label} value={pane[kind].brightness} test="scope-brightness"
    onChange={value => set(kind, { brightness: value })} />;
  const body = (() => {
    switch (pane.kind) {
      case 'waveform': return <>
        <Choice label={labels.mode} value={pane.waveform.channels} choices={[['luma', NAMES.y], ['rgb', NAMES.rgb]]} test="scope-mode"
          onChange={channels => set('waveform', { channels })} />
        {brightness('waveform')}{graticule('waveform')}{reference('waveform', pane.waveform.reference)}
      </>;
      case 'parade': return <>{brightness('parade')}{graticule('parade')}{reference('parade', pane.parade.reference)}</>;
      case 'vectorscope': {
        const scope = pane.vectorscope;
        return <>
          <Choice label={labels.mode} value={scope.tones} test="scope-tones"
            choices={(['all', 'low', 'mid', 'high'] as const).map(each => [each, labels.tones[each]] as const)} onChange={tones => set('vectorscope', { tones })} />
          {brightness('vectorscope')}{graticule('vectorscope')}
          <Slider label={labels.lowRange} value={scope.low} test="scope-low-range" onChange={low => set('vectorscope', { low: Math.min(low, scope.high) })} />
          <Slider label={labels.highRange} value={scope.high} test="scope-high-range" onChange={high => set('vectorscope', { high: Math.max(high, scope.low) })} />
          <Check label={labels.zoom2x} checked={scope.zoom2x} onChange={zoom2x => set('vectorscope', { zoom2x })} test="scope-zoom2x" />
          <Check label={labels.skinTone} checked={scope.skinTone} onChange={skinTone => set('vectorscope', { skinTone })} test="scope-skin-tone" />
          <Check label={labels.showGraticule} checked={scope.showGraticule} onChange={showGraticule => set('vectorscope', { showGraticule })} test="scope-show-graticule" />
        </>;
      }
      case 'histogram': return <>
        <Choice label={labels.mode} value={pane.histogram.channels} choices={[['rgb', NAMES.rgb], ['yrgb', NAMES.yrgb]]} test="scope-mode"
          onChange={channels => set('histogram', { channels })} />
        {brightness('histogram', labels.gain)}{graticule('histogram')}{reference('histogram', pane.histogram.reference)}
      </>;
      case 'chromaticity': return <>
        <Choice label={labels.mode} value="1931" choices={[['1931', NAMES.cie1931]]} test="scope-mode" onChange={() => {}} />
        {brightness('chromaticity')}{graticule('chromaticity')}
        <label className="scope-control scope-select">
          <span>{labels.extraGamut}</span>
          <select className="field" aria-label={labels.extraGamut} value={pane.chromaticity.extra ?? ''} data-test="scope-extra-gamut"
            onChange={event => set('chromaticity', { extra: event.target.value === '' ? null : event.target.value as Gamut })}>
            <option value="">{labels.none}</option>
            {GAMUTS.filter(each => each !== 'rec709').map(each => <option key={each} value={each}>{GAMUT_NAMES[each]}</option>)}
          </select>
        </label>
        <Check label={labels.showGraticule} checked={pane.chromaticity.showGraticule} test="scope-show-graticule"
          onChange={showGraticule => set('chromaticity', { showGraticule })} />
      </>;
    }
  })();
  return <div ref={box} className="scope-controls" role="dialog" aria-label={labels.controls} data-test="scope-controls">{body}</div>;
}

/**
 * A pane of the scopes (pp. 3018–3022): the Name drop-down that chooses its scope, the Option
 * button of its Custom Controls, and the scope's picture with the graticule over it. Alt and the
 * wheel zoom the graph, the middle button pans it, and a double click puts it back (not in the
 * manual). The scope is subscribed while the pane is `active`.
 */
export function ScopePane({ context, active, pane, options, onPane, index, extra }: {
  context: ScopesContext; active: boolean; pane: PaneSettings; options: ScopeOptions; onPane: (pane: PaneSettings) => void;
  /** Which pane of the window it is, from 0. */
  index: number;
  /** More buttons at the right of its head. */
  extra?: ReactNode;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage.scopes;
  const said = words.hint.studio.colorPage.scopes;
  const body = useRef<HTMLDivElement>(null);
  const box = useSize(body);
  const [controls, setControls] = useState(false);
  const [lens, setLens] = useState({ zoom: 1, x: 0, y: 0 });
  useEffect(() => { setLens({ zoom: 1, x: 0, y: 0 }); }, [pane.kind]);
  const size = box === null ? null : pictureSize(pane.kind, box.width, box.height, ratio());
  const settings = useMemo(() => (size === null ? null : engineSettings(pane, options, size)),
    [JSON.stringify(size), JSON.stringify(pane), JSON.stringify(options)]);
  const scope = useScope({ bridge: context.bridge, session: context.session, view: context.view, settings, active: active && context.native });
  const shown = scope.status === 'shown' && scope.graticule !== null && box !== null;
  const fit = shown ? fitted(scope.graticule!.width, scope.graticule!.height, box!.width, box!.height) : null;
  const overlay = shown ? overlayOf(scope.graticule!, pane, options) : null;
  const message = !context.native ? said.browser
    : scope.status === 'tooMany' ? said.tooMany
      : scope.status === 'unavailable' ? said.unavailable
        : scope.status === 'failed' ? said.failed
          : scope.status === 'preparing' || (scope.status === 'idle' && active && context.session !== null) ? (context.loaded ? said.preparing : said.empty)
            : null;
  const closeControls = useCallback(() => setControls(false), []);

  // Alt and the wheel zoom about the pointer; the middle button pans (p. 3018).
  const zoom = (event: WheelEvent) => {
    if (!event.altKey || !body.current) return;
    event.preventDefault();
    const at = body.current.getBoundingClientRect();
    const [px, py] = [event.clientX - at.left, event.clientY - at.top];
    setLens((old) => {
      const zoomed = Math.min(ZOOM_MOST, Math.max(1, old.zoom * (event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP)));
      const k = zoomed / old.zoom;
      return zoomed === 1 ? { zoom: 1, x: 0, y: 0 } : { zoom: zoomed, x: px - (px - old.x) * k, y: py - (py - old.y) * k };
    });
  };
  const pan = (event: ReactPointerEvent) => {
    if (event.button !== 1) return;
    event.preventDefault();
    const start = { x: event.clientX, y: event.clientY, lens };
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture?.(event.pointerId);
    const move = (moved: PointerEvent) => setLens({ ...start.lens, x: start.lens.x + moved.clientX - start.x, y: start.lens.y + moved.clientY - start.y });
    const up = () => { target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', up); target.removeEventListener('pointercancel', up); };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  };

  const opacity = Math.min(1, pane[pane.kind].graticule * 1.6);
  return <section className="scope-pane" aria-label={labels.kinds[pane.kind]} data-kind={pane.kind} data-status={scope.status} data-pane={index} data-test="scope-pane">
    <header className="scope-pane-head">
      <select className="field scope-name" aria-label={labels.scope} value={pane.kind} data-test="scope-kind"
        onChange={(event) => { onPane({ ...pane, kind: event.target.value as ScopeKind }); event.currentTarget.blur(); }}>
        {SCOPE_KINDS.map(each => <option key={each} value={each}>{labels.kinds[each]}</option>)}
      </select>
      <button type="button" className="icon-btn" aria-label={labels.controls} title={labels.controls} aria-expanded={controls} data-scope-controls-button=""
        onClick={() => setControls(open => !open)} data-test="scope-controls-button"><SlidersHorizontal size={13} aria-hidden="true" /></button>
      <span className="scope-gap" />
      {extra}
    </header>
    <div ref={body} className="scope-pane-body" onWheel={zoom} onPointerDown={pan} onDoubleClick={() => setLens({ zoom: 1, x: 0, y: 0 })}
      onMouseDown={(event) => { if (event.button === 1) event.preventDefault(); }} onAuxClick={event => event.preventDefault()}>
      <div className="scope-lens" style={lens.zoom === 1 ? undefined : { transform: `translate(${lens.x}px, ${lens.y}px) scale(${lens.zoom})` }}>
        <canvas ref={scope.canvas} className="scope-picture" hidden={!shown} data-test="scope-picture"
          style={fit ? { left: fit.x, top: fit.y, width: scope.graticule!.width * fit.scale, height: scope.graticule!.height * fit.scale } : undefined} />
        {overlay && fit && <svg className="scope-graticule" width={box!.width} height={box!.height} aria-hidden="true" data-test="scope-graticule">
          <g opacity={opacity}>
            {overlay.lines.filter(line => !line.dashed).map((line, at) => <line key={at} x1={fit.x + line.x1 * fit.scale} y1={fit.y + line.y1 * fit.scale}
              x2={fit.x + line.x2 * fit.scale} y2={fit.y + line.y2 * fit.scale} stroke={`rgb(${line.colour})`} />)}
            {overlay.labels.map((label, at) => <text key={at} x={fit.x + label.x * fit.scale} y={fit.y + label.y * fit.scale}
              fill={`rgb(${label.colour})`}>{label.text}</text>)}
          </g>
          {overlay.lines.filter(line => line.dashed).map((line, at) => <line key={`r${at}`} className="scope-reference" data-test="scope-reference-line"
            x1={fit.x + line.x1 * fit.scale} y1={fit.y + line.y1 * fit.scale} x2={fit.x + line.x2 * fit.scale} y2={fit.y + line.y2 * fit.scale}
            stroke={`rgb(${line.colour})`} />)}
        </svg>}
      </div>
      {message !== null && <p className="caption scope-status" role="status" data-test="scope-status">{message}</p>}
    </div>
    {controls && <CustomControls pane={pane} onPane={onPane} onClose={closeControls} />}
  </section>;
}

/** The option menu (…) of the scopes, which holds for all of them (pp. 3009–3010). */
function OptionsButton({ options, onOptions }: { options: ScopeOptions; onOptions: (options: ScopeOptions) => void }) {
  const labels = useWords().label.studio.colorPage.scopes;
  const [menu, setMenu] = useState<Menu | null>(null);
  const set = (change: Partial<ScopeOptions>) => onOptions({ ...options, ...change });
  const open = (event: ReactMouseEvent<HTMLButtonElement>) => {
    const at = event.currentTarget.getBoundingClientRect();
    setMenu({
      x: at.left, y: at.bottom, label: labels.menu,
      items: [
        { id: 'waveform-scale', label: labels.waveformScale, items: [
          { id: 'scale-10-bit', label: labels.tenBit, checked: options.scale === '10-bit', run: () => set({ scale: '10-bit' }) },
          { id: 'scale-percent', label: labels.percent, checked: options.scale === 'percent', run: () => set({ scale: 'percent' }) },
          { id: 'scale-nits', label: NAMES.hdr, checked: options.scale === 'nits', run: () => set({ scale: 'nits' }) },
        ] },
        { id: 'vectorscope-scale', label: labels.vectorscopeScale, items: [
          { id: 'style-off', label: labels.off, checked: options.vectorStyle === 'off', run: () => set({ vectorStyle: 'off' }) },
          { id: 'style-standard', label: labels.standard, checked: options.vectorStyle === 'standard', run: () => set({ vectorStyle: 'standard' }) },
          { id: 'targets-75', label: NAMES.t75, checked: options.targets === '75', run: () => set({ targets: '75' }) },
          { id: 'targets-100', label: NAMES.t100, checked: options.targets === '100', run: () => set({ targets: '100' }) },
          { id: 'targets-both', label: labels.both, checked: options.targets === 'both', run: () => set({ targets: 'both' }) },
          { id: 'hue-labels', label: labels.hueLabels, checked: options.hueLabels, run: () => set({ hueLabels: !options.hueLabels }) },
        ] },
      ],
    });
  };
  return <>
    <button type="button" className="icon-btn" aria-label={labels.menu} title={labels.menu} aria-haspopup="menu" aria-expanded={menu !== null}
      onClick={open} data-test="scope-options"><Ellipsis size={14} aria-hidden="true" /></button>
    {menu && <ColorMenu menu={menu} onClose={() => setMenu(null)} />}
  </>;
}

const LAYOUT_NAMES = { 1: 'one', 2: 'two', 4: 'four', 9: 'nine' } as const;
/** The least size of the floating window, and where it is first. */
const FLOAT_LEAST = { width: 240, height: 180 };
function firstRect(): FloatRect {
  const width = Math.min(560, Math.max(FLOAT_LEAST.width, (window.innerWidth || 1280) * 0.4));
  return { x: Math.max(0, (window.innerWidth || 1280) - width - 24), y: 72, width, height: Math.round(width * 0.75) };
}
/** A rectangle kept within the window, at least so large. */
function within(rect: FloatRect): FloatRect {
  const room = { width: window.innerWidth || 1280, height: window.innerHeight || 800 };
  const width = Math.max(FLOAT_LEAST.width, Math.min(rect.width, room.width));
  const height = Math.max(FLOAT_LEAST.height, Math.min(rect.height, room.height));
  return { width, height, x: Math.min(Math.max(0, rect.x), Math.max(0, room.width - width)), y: Math.min(Math.max(0, rect.y), Math.max(0, room.height - 32)) };
}

/** The buttons at the upper right of the floating scopes: 1, 2, 4 or 9 panes (p. 3018), the option menu (…), and Close. */
function FloatingTools({ settings, onSettings, onClose }: {
  settings: ScopesSettings; onSettings: (change: (old: ScopesSettings) => ScopesSettings) => void; onClose: () => void;
}) {
  const labels = useWords().label.studio.colorPage.scopes;
  const { layout } = settings.floating;
  return <>
    <div className="scope-layouts" role="radiogroup" aria-label={labels.layout}>
      {LAYOUTS.map(each => <button key={each} type="button" role="radio" aria-checked={each === layout} aria-label={labels.layouts[LAYOUT_NAMES[each]]}
        title={labels.layouts[LAYOUT_NAMES[each]]} data-layout={each} data-test={`scope-layout-${each}`}
        onClick={() => onSettings(old => ({ ...old, floating: { ...old.floating, layout: each as Layout } }))}>{each}</button>)}
    </div>
    <OptionsButton options={settings.options} onOptions={options => onSettings(old => ({ ...old, options }))} />
    <button type="button" className="icon-btn" aria-label={labels.close} title={`${labels.close} (Ctrl+Shift+W)`} onClick={onClose}
      data-test="scope-window-close"><X size={14} aria-hidden="true" /></button>
  </>;
}

/** The panes of the floating scopes, as many as the layout shows, each with its own scope and settings. */
function FloatingGrid({ context, active, settings, onSettings }: {
  context: ScopesContext; active: boolean; settings: ScopesSettings; onSettings: (change: (old: ScopesSettings) => ScopesSettings) => void;
}) {
  const { layout, panes } = settings.floating;
  const setPane = (at: number) => (pane: PaneSettings) => onSettings(old => ({
    ...old, floating: { ...old.floating, panes: old.floating.panes.map((each, index) => (index === at ? pane : each)) },
  }));
  return <div className="scope-grid" data-layout={layout}>
    {panes.slice(0, layout).map((pane, at) => <ScopePane key={at} index={at} context={context} active={active} pane={pane} options={settings.options}
      onPane={setPane(at)} />)}
  </div>;
}

/**
 * The floating window of the scopes as a window of its own (T19), which the shell opens over the
 * main one: the engine's view of the viewer does not cover it, and it may go to another display.
 * Its frame is the system's, which moves and resizes it; it fills it.
 */
export function ScopesWindowBody({ context, active, settings, onSettings, onClose }: {
  context: ScopesContext; active: boolean; settings: ScopesSettings; onSettings: (change: (old: ScopesSettings) => ScopesSettings) => void; onClose: () => void;
}) {
  const labels = useWords().label.studio.colorPage.scopes;
  return <section className="scope-window scope-window-own" aria-label={labels.window} data-layout={settings.floating.layout} data-test="scope-window">
    <header className="scope-window-head">
      <strong>{labels.window}</strong>
      <span className="scope-gap" />
      <FloatingTools settings={settings} onSettings={onSettings} onClose={onClose} />
    </header>
    <FloatingGrid context={context} active={active} settings={settings} onSettings={onSettings} />
  </section>;
}

/**
 * The floating window of the scopes within the page (pp. 3007, 3018–3019), where the shell has no
 * window of its own for it (a browser): its head is dragged to move it and its lower right corner
 * to resize it; the buttons at its upper right show 1, 2, 4 or 9 panes, each with its own scope and
 * settings; the option menu (…) holds for all. The engine's view of the viewer, which lies over
 * the page, would cover the part of it that lies over the viewer.
 */
function FloatingScopes({ context, active, settings, onSettings, onClose }: {
  context: ScopesContext; active: boolean; settings: ScopesSettings; onSettings: (change: (old: ScopesSettings) => ScopesSettings) => void; onClose: () => void;
}) {
  const labels = useWords().label.studio.colorPage.scopes;
  const { layout } = settings.floating;
  const [rect, setRect] = useState<FloatRect>(() => within(settings.floating.rect ?? firstRect()));
  const keep = (next: FloatRect) => onSettings(old => ({ ...old, floating: { ...old.floating, rect: next } }));
  const drag = (event: ReactPointerEvent<HTMLElement>, how: 'move' | 'size') => {
    if (event.button !== 0 || (how === 'move' && (event.target as Element).closest('button, select, input'))) return;
    event.preventDefault();
    const start = { x: event.clientX, y: event.clientY, rect };
    const target = event.currentTarget;
    target.setPointerCapture?.(event.pointerId);
    let last = rect;
    const move = (moved: PointerEvent) => {
      const [dx, dy] = [moved.clientX - start.x, moved.clientY - start.y];
      last = within(how === 'move' ? { ...start.rect, x: start.rect.x + dx, y: start.rect.y + dy }
        : { ...start.rect, width: start.rect.width + dx, height: start.rect.height + dy });
      setRect(last);
    };
    const up = () => {
      target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', up); target.removeEventListener('pointercancel', up);
      keep(last);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  };
  return createPortal(<section className="scope-window" role="dialog" aria-modal="false" aria-label={labels.window} data-studio-portal=""
    data-layout={layout} data-test="scope-window" style={{ left: rect.x, top: rect.y, width: rect.width, height: rect.height }}>
    <header className="scope-window-head" onPointerDown={event => drag(event, 'move')}>
      <strong>{labels.window}</strong>
      <span className="scope-gap" />
      <FloatingTools settings={settings} onSettings={onSettings} onClose={onClose} />
    </header>
    <FloatingGrid context={context} active={active} settings={settings} onSettings={onSettings} />
    <span className="scope-window-corner" aria-hidden="true" onPointerDown={event => drag(event, 'size')} data-test="scope-window-corner" />
  </section>, document.body);
}

/** Whether the page is shown: the scopes measure nothing for a window put away. */
export function usePageShown(): boolean {
  const [shown, setShown] = useState(() => document.visibilityState !== 'hidden');
  useEffect(() => {
    const changed = () => setShown(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', changed);
    return () => document.removeEventListener('visibilitychange', changed);
  }, []);
  return shown;
}

/**
 * The scopes of the Color page (pp. 3007–3022; the interaction reference, §6): one docked at the
 * bottom right, in the Scopes panel of the palettes, and the floating window, opened by the
 * Expand button of the docked one or Ctrl+Shift+W (Workspace › Video Scopes › On/Off) and closed
 * by its own button or the keys again. While it is open, the docked panel says so and measures
 * nothing. A scope is subscribed only while it is shown: its pane is mounted, the page is shown,
 * and the viewer does not cover it (Full page and Cinema). Their settings are kept on this desktop.
 */
export function useColorScopes(context: ScopesContext & { viewerMode: string; language?: string }): { docked: ReactNode; floating: ReactNode } {
  const { viewerMode, language, ...shared } = context;
  const words = useWords();
  const labels = words.label.studio.colorPage.scopes;
  const [settings, setSettings] = useState<ScopesSettings>(loadScopes);
  const [open, setOpen] = useState(false);
  const pageShown = usePageShown();
  const change = useCallback((how: (old: ScopesSettings) => ScopesSettings) => setSettings((old) => {
    const next = how(old);
    storeScopes(next);
    return next;
  }), []);
  // The window of the scopes keeps the settings of the floating panes too: read them again as it changes them.
  useEffect(() => {
    const changed = (event: StorageEvent) => { if (event.key === null || event.key === scopesStorageKey) setSettings(loadScopes()); };
    window.addEventListener('storage', changed);
    return () => window.removeEventListener('storage', changed);
  }, []);
  // In the desktop app, the floating scopes are a window of their own, which the shell opens (T19).
  const shell = useMemo(() => (shared.native ? scopesShell(shared.bridge) : null), [shared.native, shared.bridge]);
  useEffect(() => {
    if (!shell) return undefined;
    // Closed by itself (its own button or keys): docked again.
    return shared.bridge.onEvent((event) => { if (event.kind === 'scopes-window' && !event.open) setOpen(false); });
  }, [shell, shared.bridge]);
  useEffect(() => {
    if (!shell || !open) return;
    void shell({ open: true, loaded: shared.loaded, language, bounds: keptWindowBounds() }).catch(() => setOpen(false));
  }, [shell, open, shared.loaded, shared.session, language]);
  useEffect(() => {
    if (!shell || !open) return undefined;
    // Closed with the page: the scopes measure the Color page's viewer.
    return () => { void shell({ open: false }).catch(() => {}); };
  }, [shell, open]);
  // Ctrl+Shift+W: the floating window on and off (pp. 692, 3007–3008).
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || key.repeat || !key.ctrl || !key.shift || key.alt || key.meta) return;
      if (key.code !== 'KeyW' && key.key.toLowerCase() !== 'w') return;
      event.preventDefault();
      setOpen(old => !old);
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  const covered = viewerMode === 'cinema';
  const dockedShown = pageShown && !open && !covered && viewerMode !== 'full';
  const docked = open
    ? <div className="scope-docked" data-test="scopes-docked"><p className="caption color-empty" data-test="scope-floating-note">{words.hint.studio.colorPage.scopes.floating}</p></div>
    : <div className="scope-docked" data-test="scopes-docked">
      <ScopePane index={0} context={shared} active={dockedShown} pane={settings.docked} options={settings.options}
        onPane={pane => change(old => ({ ...old, docked: pane }))}
        extra={<>
          <OptionsButton options={settings.options} onOptions={options => change(old => ({ ...old, options }))} />
          <button type="button" className="icon-btn" aria-label={labels.expand} title={`${labels.expand} (Ctrl+Shift+W)`} onClick={() => setOpen(true)}
            data-test="scope-expand"><Expand size={13} aria-hidden="true" /></button>
        </>} />
    </div>;
  const floating = open && !shell
    ? <FloatingScopes context={shared} active={pageShown && !covered} settings={settings} onSettings={change} onClose={() => setOpen(false)} />
    : null;
  return { docked, floating };
}

/** The shell's window of the scopes, where the bridge is the desktop app's. */
function scopesShell(bridge: ScopeBridge): EngineBridge['scopesWindow'] | null {
  const shell = (bridge as Partial<Pick<EngineBridge, 'scopesWindow'>>).scopesWindow;
  return typeof shell === 'function' ? shell.bind(bridge) : null;
}

/** Where the window of the scopes was last, as its page keeps it (screen pixels), or none. */
export const SCOPES_WINDOW_KEY = 'columbina-scopes-window';
export function keptWindowBounds(): { x: number; y: number; width: number; height: number } | null {
  try {
    const kept: unknown = JSON.parse(localStorage.getItem(SCOPES_WINDOW_KEY) ?? 'null');
    if (kept === null || typeof kept !== 'object') return null;
    const { x, y, width, height } = kept as Record<string, unknown>;
    return [x, y, width, height].every(Number.isInteger) ? { x: x as number, y: y as number, width: width as number, height: height as number } : null;
  } catch {
    return null;
  }
}
export function keepWindowBounds(bounds: { x: number; y: number; width: number; height: number }): void {
  try {
    localStorage.setItem(SCOPES_WINDOW_KEY, JSON.stringify(bounds));
  } catch {
    // Without storage the window opens where the shell puts it.
  }
}
