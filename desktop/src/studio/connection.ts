/**
 * The engine of the studio as the page reaches it: through the shell in the desktop app, or the
 * engine of the replay in a browser (`createMockEngine`), which plays nothing and draws no view.
 * There is one copy of the project for each, made the first time the studio is opened, and kept
 * while the page lives: the shell keeps the engine open while the app is on its folder.
 */
import type { EngineBridge } from '../bridge';
import { createMockEngine } from '../mock';
import { ProjectCopy } from './engine';

export interface StudioEngine {
  bridge: EngineBridge;
  copy: ProjectCopy;
  /** Whether the shell lays the engine's view over the viewer: false in a browser. */
  native: boolean;
}

const made = new WeakMap<EngineBridge, StudioEngine>();
let replay: EngineBridge | null = null;

/** The engine of the studio of this page. */
export function studioEngine(): StudioEngine {
  const shell = window.columbina?.engine;
  const bridge = shell ?? (replay ??= createMockEngine());
  let engine = made.get(bridge);
  if (!engine) {
    engine = { bridge, copy: new ProjectCopy(bridge), native: shell !== undefined };
    made.set(bridge, engine);
  }
  return engine;
}
