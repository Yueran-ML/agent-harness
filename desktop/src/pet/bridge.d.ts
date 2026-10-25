import type { Animations } from './animate';
import type { PetState } from './state';

/** What the main window says the task does, as the shell hands it on to the window of the pet. */
export interface Shown {
  state: PetState;
  label: string;
  /** What the pet says in its bubble, in the language of the main window. Empty for nothing. */
  bubble: string;
}

/** The picture of a pet, as an address of data, and the rows that its package names, or null. */
export interface Sheet {
  sheet: string;
  animations: Animations | null;
  /** What the pet is called, for those who cannot see it. */
  name: string;
}

/** What the window of the pet can ask of the shell. It can ask nothing else of the system. */
export interface PetBridge {
  /** The pet to show, and what the task does now. */
  ready(): Promise<{ pet: Sheet | null; shown: Shown }>;
  onShown(callback: (shown: Shown) => void): () => void;
  onSheet(callback: (pet: Sheet | null) => void): () => void;
  /** Show the main window. */
  click(): Promise<void>;
  /** Move the window with the pointer, from where a drag began to where it ends. */
  drag(phase: 'start' | 'move' | 'end'): Promise<void>;
  /** Open the menu of the pet. */
  menu(): Promise<void>;
}

declare global {
  interface Window { columbinaPet?: PetBridge }
}
