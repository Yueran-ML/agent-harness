import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { callEngine, type PointerCursor, type ViewPointer, type ViewerTool } from '../engine';
import type { EditedNode } from './useNodeEdit';

/**
 * The pickers of colour of the Color page's palettes on its viewer (Reference Manual 20, pp.
 * 3044–3045, 3110, 3122, 3166; the interaction reference, §1.2 OSC, §3.5, §4.1, §4.3, §8.1).
 *
 * The viewer's native pointer has one owner, `useViewerInput`: a palette never listens to it. A
 * palette gives a picker here, and the owner hands it the pointer while the picker is in force:
 *
 * - an **armed** picker, a button pressed for one click (Pick Black Point and Pick White Point of
 *   Primaries), goes before everything but a split screen;
 * - else the picker of the palette shown in the middle group (the Qualifier's Sample Eyedropper,
 *   or the Curves' sampling), while the viewer's on-screen control is Qualifier: the OSC that
 *   opening the Qualifier palette switches to (p. 2988), and that shows DaVinci's Color Sample
 *   cursor. Opening Curves leaves it as it is; its picker button switches it.
 */
export interface ViewerPicker {
  /** The cursor that the view shows while the picker takes the pointer. */
  cursor: PointerCursor;
  /** What the pointer did on the picture: a left press, its moves and its release, or a cancel. */
  onPointer: (pointer: ViewPointer) => void;
}
export type PickerSlot = 'armed' | 'palette';

/** What the palettes reach through the context: the slots of pickers, and the viewer's OSC. */
export interface PickerHub {
  set: (slot: PickerSlot, picker: ViewerPicker) => void;
  /** Take the picker of a slot away, where it is still `picker`. */
  clear: (slot: PickerSlot, picker: ViewerPicker) => void;
  armed: ViewerPicker | null;
  tool: ViewerTool;
  setTool: (tool: ViewerTool) => void;
  /** The colour that goes into the current node about a point of the picture; null where there is no node. */
  sample: ((x: number, y: number) => Promise<Rgb | null>) | null;
  /** Where a refusal of the engine is said. */
  onFailure: (failure: unknown) => void;
}
export const PickerContext = createContext<PickerHub | null>(null);

/** The slots of the page, and the picker in force for the OSC `tool`. */
export function usePickerHub(tool: ViewerTool, setTool: (tool: ViewerTool) => void, edited: EditedNode | null,
  onFailure: (failure: unknown) => void): { hub: PickerHub; active: ViewerPicker | null } {
  const [slots, setSlots] = useState<Record<PickerSlot, ViewerPicker | null>>({ armed: null, palette: null });
  const set = useCallback((slot: PickerSlot, picker: ViewerPicker) => setSlots(old => (old[slot] === picker ? old : { ...old, [slot]: picker })), []);
  const clear = useCallback((slot: PickerSlot, picker: ViewerPicker) => setSlots(old => (old[slot] === picker ? { ...old, [slot]: null } : old)), []);
  const node = edited?.node.kind === 'corrector' ? edited : null;
  const at = useRef(node);
  at.current = node;
  const has = node !== null;
  // Read at the moment of the sample: the node current then.
  const sample = useMemo(() => (has ? (x: number, y: number) => (at.current ? sampleInput(at.current, x, y) : Promise.resolve(null)) : null), [has]);
  const hub = useMemo(() => ({ set, clear, armed: slots.armed, tool, setTool, sample, onFailure }),
    [set, clear, slots.armed, tool, setTool, sample, onFailure]);
  return { hub, active: pickerInForce(slots, tool) };
}

/** The picker in force: an armed one, else a palette's while the OSC is Qualifier. */
export function pickerInForce(slots: Record<PickerSlot, ViewerPicker | null>, tool: ViewerTool): ViewerPicker | null {
  return slots.armed ?? (tool === 'qualifier' ? slots.palette : null);
}

/** A picker of the crosshair, whose handler is read when the pointer comes, so it stays one object. */
function usePicker(onPointer: (pointer: ViewPointer) => void): ViewerPicker {
  const handler = useRef(onPointer);
  handler.current = onPointer;
  return useMemo(() => ({ cursor: 'crosshair', onPointer: pointer => handler.current(pointer) }), []);
}

/** A palette's picker, given while the palette is shown (`on`). */
export function usePalettePicker(on: boolean, onPointer: (pointer: ViewPointer) => void): PickerHub | null {
  const hub = useContext(PickerContext);
  const picker = usePicker(onPointer);
  const set = hub?.set;
  const clear = hub?.clear;
  useEffect(() => {
    if (!set || !clear || !on) return undefined;
    set('palette', picker);
    return () => clear('palette', picker);
  }, [set, clear, on, picker]);
  return hub;
}

/**
 * A picker for one click (pp. 3044–3045): `arm` gives it the pointer until a left press on the
 * picture, which it is handed, or until Escape, another arming, or `disarm`; the interaction
 * reference takes one click and out (§3.5, to be checked on DaVinci Resolve).
 */
export function useArmedPicker(onPress: (pointer: ViewPointer) => void) {
  const hub = useContext(PickerContext);
  const picker = usePicker((pointer) => {
    if (pointer.kind !== 'down' || pointer.button !== 'left' || pointer.x === null || pointer.y === null) return;
    hub?.clear('armed', picker);
    onPress(pointer);
  });
  const armed = hub !== null && hub.armed === picker;
  const clear = hub?.clear;
  useEffect(() => {
    if (!armed || !clear) return undefined;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopPropagation();
      clear('armed', picker);
    };
    window.addEventListener('keydown', escape, true);
    return () => window.removeEventListener('keydown', escape, true);
  }, [armed, clear, picker]);
  // A picker armed and then put away (its palette closed) disarms.
  useEffect(() => () => clear?.('armed', picker), [clear, picker]);
  return {
    available: hub !== null,
    armed,
    toggle: () => { if (!hub) return; if (armed) hub.clear('armed', picker); else hub.set('armed', picker); },
  };
}

export type Rgb = readonly [number, number, number];

/** How many pixels a sample averages, a square about the point (`view.sample`; odd, 1 to 31). */
export const SAMPLE_SIZE = 3;

/** The colour that goes into the node edited, about a point of the picture of the timeline monitor. */
export async function sampleInput(edited: EditedNode, x: number, y: number): Promise<Rgb | null> {
  const sample = await callEngine(edited.bridge, 'view.sample', {
    monitor: 'timeline', x, y, size: SAMPLE_SIZE, grade: edited.target, node: edited.node.id,
    ...(edited.target.level === 'pre_clip' || edited.target.level === 'post_clip') && edited.clip ? { clip: edited.clip } : {},
  }, edited.session === undefined ? undefined : { expectedSession: edited.session });
  return sample.node?.input ?? null;
}

/**
 * A drag of a picker over the picture (p. 3166: a click takes a pixel, a drag the pixels it goes
 * over): the colours taken, in order. One sample is asked at a time, the latest point waiting;
 * `onSamples` is told the colours each time one comes, and once more, `ended`, when the button is
 * let go and the last has come. A cancel forgets the drag.
 */
export class SampleDrag {
  private samples: Rgb[] = [];
  private asking = false;
  private waiting: { x: number; y: number } | null = null;
  private released = false;
  private over = false;
  constructor(
    private readonly sample: (x: number, y: number) => Promise<Rgb | null>,
    private readonly onSamples: (samples: readonly Rgb[], ended: boolean) => void,
    private readonly onFailure: (failure: unknown) => void,
  ) {}

  /** A point of the picture under the pointer, as the drag goes. */
  at(x: number | null, y: number | null): void {
    if (this.over || this.released || x === null || y === null) return;
    this.waiting = { x, y };
    this.next();
  }

  /** The button let go. */
  release(): void {
    if (this.over || this.released) return;
    this.released = true;
    if (!this.asking) this.end();
  }

  cancel(): void { this.over = true; }

  get done(): boolean { return this.over; }

  private end(): void {
    if (this.over) return;
    this.over = true;
    if (this.samples.length > 0) this.onSamples(this.samples, true);
  }

  private next(): void {
    if (this.asking || this.waiting === null || this.over) return;
    const { x, y } = this.waiting;
    this.waiting = null;
    this.asking = true;
    void this.sample(x, y).then((rgb) => {
      this.asking = false;
      if (this.over) return;
      if (rgb !== null) {
        this.samples = [...this.samples, rgb];
        if (!this.released || this.waiting === null) this.onSamples(this.samples, false);
      }
      if (this.waiting !== null) this.next();
      else if (this.released) this.end();
    }, (failure: unknown) => {
      this.asking = false;
      this.over = true;
      this.onFailure(failure);
    });
  }
}

/**
 * The drags of a palette's picker: a left press on the picture begins one (Alt as it is pressed
 * kept for it), its moves add to it, its release ends it; `onSamples` is given the colours with
 * whether it ended, and `onCancel` where the pointer was lost.
 */
export function usePickerDrags(sample: ((x: number, y: number) => Promise<Rgb | null>) | null,
  onSamples: (samples: readonly Rgb[], ended: boolean, alt: boolean) => void, onCancel: () => void, onFailure: (failure: unknown) => void) {
  const drag = useRef<{ run: SampleDrag; alt: boolean } | null>(null);
  const now = useRef({ sample, onSamples, onCancel, onFailure });
  now.current = { sample, onSamples, onCancel, onFailure };
  // Escape while a drag goes on puts back what it made (as the controls of the palettes do).
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      const going = drag.current;
      if (event.key !== 'Escape' || !going || going.run.done) return;
      event.preventDefault(); event.stopPropagation();
      going.run.cancel(); drag.current = null; now.current.onCancel();
    };
    window.addEventListener('keydown', escape, true);
    return () => { window.removeEventListener('keydown', escape, true); drag.current?.run.cancel(); };
  }, []);
  return useCallback((pointer: ViewPointer) => {
    const going = drag.current;
    if (pointer.kind === 'down') {
      if (pointer.button !== 'left' || pointer.x === null || pointer.y === null || now.current.sample === null) return;
      going?.run.cancel();
      const ask = now.current.sample;
      const alt = pointer.alt;
      const run = new SampleDrag(ask, (samples, ended) => now.current.onSamples(samples, ended, alt), failure => now.current.onFailure(failure));
      drag.current = { run, alt };
      run.at(pointer.x, pointer.y);
      return;
    }
    if (!going || going.run.done) return;
    if (pointer.kind === 'move') going.run.at(pointer.x, pointer.y);
    else if (pointer.kind === 'up' && pointer.button === 'left') { going.run.at(pointer.x, pointer.y); going.run.release(); }
    else if (pointer.kind === 'cancel') { going.run.cancel(); drag.current = null; now.current.onCancel(); }
  }, []);
}
