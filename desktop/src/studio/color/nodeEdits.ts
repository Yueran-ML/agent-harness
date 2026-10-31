/**
 * What the node editor of the Color page asks of the engine, as DaVinci Resolve's node editor does
 * it (Reference Manual 20, Chapter 143, pp. 3308–3322; the interaction reference, §2 and §9.4):
 * which grade a level of the current clip is, the edits and operations of each command, and the
 * keys that ask for them. Pure, so that the editor, the page and the tests work from the same rules.
 *
 * Every command is one transaction, which one undo takes back (the one history of the engine).
 * The engine expands the edits (`add_serial` …) into the operations it broadcasts; the editor
 * sends operations itself only where no edit says what it wants (a link, a label, a mixer's inputs).
 */
import type { Key } from '../../app/keys';
import {
  emptyGrade, type Clip, type Composite, type Corrector, type Grade, type GradeNode, type KeyInput, type Link, type LinkKind, type Op, type Target,
  type Timeline,
} from '../engine';
import type { GradeLevel } from './clips';

/** The edits of grades (PROTOCOL.md, "Grades", "Edits"): the engine works them out into operations. */
export type GradeEdit =
  | { edit: 'add_serial' | 'add_serial_before' | 'add_parallel' | 'add_layer' | 'extract_node' | 'reset_node'; grade: Target; node: string }
  | { edit: 'append_node'; grade: Target }
  | { edit: 'copy_grade'; from: Target; to: Target[]; preserve_nodes?: number }
  | { edit: 'append_node_graph'; from: Target; to: Target[] }
  | { edit: 'paste_node'; from: Target; from_node: string; to: Target; to_node: string }
  | { edit: 'collapse_group_grades'; timeline: string; clips: string[] };
/** What one transaction of the Color page sends. */
export type GradeChange = Op | GradeEdit;

/**
 * Why the editor sends nothing: a refusal of its own, in the words of the engine's (`hint.studio.edit`),
 * where it knows the engine would refuse.
 */
export interface Refused { refused: 'wrong_node' | 'missing' }
/** A command worked out: the changes of one transaction, or why there are none to send, or nothing to do. */
export type Worked = GradeChange[] | Refused | null;
export const isRefused = (worked: Worked): worked is Refused => worked !== null && !Array.isArray(worked);

/** The grade that a level names, for the current clip of a timeline (PROTOCOL.md, "Grades"). */
export function targetOf(level: GradeLevel, timeline: Timeline | null, clip: Clip | null): Target | null {
  if (timeline === null) return null;
  if (level === 'timeline') return { level: 'timeline', timeline: timeline.id };
  if (clip === null) return null;
  if (level === 'clip') return { level: 'clip', timeline: timeline.id, clip: clip.id };
  return clip.group === null ? null : { level, group: clip.group };
}

/** A corrector as the engine makes a new one: every setting at its default (PROTOCOL.md, "A grade"). */
export function newCorrector(): Corrector {
  const wheel = (value: number) => ({ y: value, r: value, g: value, b: value });
  const rgb = (value: number) => ({ r: value, g: value, b: value });
  const curve = () => ({ points: [[0, 0], [1, 1]] as [number, number][], intensity: 100 });
  return {
    primaries: {
      lift: wheel(0), gamma: wheel(0), gain: wheel(1), offset: rgb(25), contrast: 1, pivot: 0.435, saturation: 50, hue: 50, lum_mix: 100,
      temperature: 0, tint: 0, color_boost: 0, midtone_detail: 0, shadows: 0, highlights: 0,
    },
    log: { shadow: rgb(0), midtone: rgb(0), highlight: rgb(0), low_range: 0.333, high_range: 0.55 },
    curves: { ganged: true, y: curve(), r: curve(), g: curve(), b: curve() },
    hsl: { hue_vs_hue: [], hue_vs_sat: [], hue_vs_lum: [], lum_vs_sat: [], sat_vs_sat: [], sat_vs_lum: [] },
    lut: null,
    soft_clip: { low: rgb(0), high: rgb(100), low_soft: rgb(0), high_soft: rgb(0) },
    key: { input: { invert: false, matte: true, gain: 1, offset: 0 }, output: { invert: false, gain: 1, offset: 0 } },
    shot_match: null,
  };
}

/** A new input of a key mixer, as the engine makes one: Mask mode, gain 1 (pp. 3358–3363). */
const newKeyInput = (): KeyInput => ({ invert: false, matte: false, gain: 1, offset: 0 });

const nodeOf = (grade: Grade, id: string | null): GradeNode | undefined => (id === null ? undefined : grade.nodes.find(node => node.id === id));

/** How a node is added (p. 3318): after the current one, before it, at the end, in parallel or as a layer with it. */
export type AddHow = 'serial' | 'before' | 'append' | 'parallel' | 'layer';

/**
 * Add a corrector as Alt+S, Shift+S, Alt+K, Alt+P and Alt+L do (p. 3318), about the current node.
 * A grade with no node yet takes its first after the source (Alt+S) or before the output (Shift+S);
 * a parallel or a layer node needs a current corrector (the engine refuses others as `wrong_node`).
 */
export function addNode(how: AddHow, grade: Target, current: string | null): Worked {
  switch (how) {
    case 'serial': return [{ edit: 'add_serial', grade, node: current ?? 'source' }];
    case 'before': return [{ edit: 'add_serial_before', grade, node: current ?? 'output' }];
    case 'append': return [{ edit: 'append_node', grade }];
    default:
      if (current === null) return { refused: 'wrong_node' };
      return [{ edit: how === 'parallel' ? 'add_parallel' : 'add_layer', grade, node: current }];
  }
}

/** A corrector that nothing is linked to (p. 3318: Add Node › Corrector in the background of the editor). */
export function addLoose(grade: Target): GradeChange[] {
  return [{ op: 'add_node', grade, node: { id: '', kind: 'corrector', label: '', enabled: true, params: newCorrector() } }];
}

/** The id of the corrector that a transaction added, which becomes the current node. */
export function addedCorrector(done: readonly Op[] | null | undefined): string | null {
  const added = (done ?? []).filter((op): op is Extract<Op, { op: 'add_node' }> => op.op === 'add_node' && op.node.kind === 'corrector');
  return added.at(-1)?.node.id ?? null;
}

/**
 * Delete nodes (Delete, or Delete Node, p. 3318). The engine links the nodes on either side of a
 * corrector, as DaVinci does; a mixer is taken out with nothing linked in its place, as the manual
 * says nothing of joining the many inputs of one (the interaction reference, §2.3).
 */
export function deleteNodes(grade: Target, of: Grade, ids: readonly string[]): Worked {
  const nodes = of.nodes.filter(node => ids.includes(node.id));
  if (nodes.length === 0) return null;
  return nodes.map(node => (node.kind === 'corrector' ? { op: 'remove_node', grade, id: node.id } : { op: 'remove_node', grade, id: node.id, links: [] }));
}

/** Take the current node out of the tree, the nodes on either side linked, and leave it there unlinked (E, p. 3319). */
export function extractNode(grade: Target, current: string | null): Worked {
  return current === null ? null : [{ edit: 'extract_node', grade, node: current }];
}

/**
 * Enable or disable the nodes chosen, each the other way (Ctrl+D, or a click on a node's number,
 * p. 3315); the current node is always among those chosen.
 */
export function toggleNodes(grade: Target, of: Grade, ids: readonly string[]): Worked {
  const nodes = of.nodes.filter(node => ids.includes(node.id));
  return nodes.length === 0 ? null : nodes.map(node => ({ op: 'set_node', grade, id: node.id, enabled: !node.enabled }));
}

/**
 * Enable/Disable All Nodes (Alt+D, p. 3315): all off, or, where all are off, all on, "even nodes
 * that had previously been individually disabled".
 */
export function toggleAll(grade: Target, of: Grade): Worked {
  if (of.nodes.length === 0) return null;
  const on = of.nodes.every(node => !node.enabled);
  return of.nodes.filter(node => node.enabled !== on).map(node => ({ op: 'set_node', grade, id: node.id, enabled: on }));
}

/** The three resets of p. 3316: the current node (Shift+Home), every node (Ctrl+Shift+Home), or the whole grade (Ctrl+Home). */
export type ResetHow = 'node' | 'keepNodes' | 'all';

/**
 * Reset a node, every node, or the grade. Reset All Grades and Nodes "restores a single node set to
 * the default"; the project's grade of a new clip has no node (it is DaVinci's single node at its
 * default, the clip ungraded), so the grade becomes that one again.
 */
export function reset(how: ResetHow, grade: Target, of: Grade, current: string | null): Worked {
  if (how === 'node') return current === null ? null : [{ edit: 'reset_node', grade, node: current }];
  if (how === 'keepNodes') return of.nodes.length === 0 ? null : of.nodes.map(node => ({ edit: 'reset_node', grade, node: node.id }));
  const empty = emptyGrade();
  if (of.nodes.length === 0 && JSON.stringify(of.links) === JSON.stringify(empty.links)) return null;
  return [{ op: 'set_grade', grade, value: empty }];
}

/** Change a node's label (Change Label, or a double-click on the label, p. 3313); an empty one takes it away. */
export function relabel(grade: Target, of: Grade, id: string, label: string): Worked {
  const node = nodeOf(of, id);
  if (!node || node.label === label) return null;
  return [{ op: 'set_node', grade, id, label }];
}

/** Take away the shot match of a corrector (p. 3048; the interaction reference, §11). */
export function resetShotMatch(grade: Target, id: string): GradeChange[] {
  return [{ op: 'set_node', grade, id, params: { shot_match: null } }];
}

/** Link an output into an input of the same kind; a link that the input had is put out of its place (p. 3319). */
export function connect(grade: Target, link: Link): GradeChange[] {
  return [{ op: 'connect', grade, link }];
}

/** Take a link away (a click on its second half, Delete with it chosen, or Delete Link, p. 3319). */
export function disconnect(grade: Target, link: Link): GradeChange[] {
  return [{ op: 'disconnect', grade, to: link.to, kind: link.kind, input: link.input }];
}

/** Take the end of a link to another input (its second half dragged, p. 3319): the old link goes, the new one comes. */
export function reconnect(grade: Target, old: Link, into: { to: string; input: number }): Worked {
  if (old.to === into.to && old.input === into.input) return null;
  return [...disconnect(grade, old), ...connect(grade, { from: old.from, to: into.to, kind: old.kind, input: into.input })];
}

/** How many inputs of `kind` a mixer has now, and the patch that gives it one more (Add One Input, pp. 3335, 3360). */
function oneMoreInput(node: GradeNode, kind: LinkKind): { count: number; params: Record<string, unknown> } | null {
  if ((node.kind === 'parallel_mixer' || node.kind === 'layer_mixer') && kind === 'rgb') return { count: node.params.inputs, params: { inputs: node.params.inputs + 1 } };
  if (node.kind === 'key_mixer' && kind === 'key') return { count: node.params.inputs.length, params: { inputs: [...node.params.inputs, newKeyInput()] } };
  return null;
}

/** Add One Input to a mixer (the right-click menu of a Parallel, Layer or Key Mixer). */
export function addInput(grade: Target, of: Grade, id: string): Worked {
  const node = nodeOf(of, id);
  const more = node && oneMoreInput(node, node.kind === 'key_mixer' ? 'key' : 'rgb');
  return more ? [{ op: 'set_node', grade, id, params: more.params }] : null;
}

/** A link dropped on a mixer, not on one of its inputs: the mixer takes one more input for it (pp. 3319, 3335). */
export function linkIntoNewInput(grade: Target, of: Grade, id: string, from: string, kind: LinkKind): Worked {
  const node = nodeOf(of, id);
  const more = node && oneMoreInput(node, kind);
  if (!more) return null;
  return [{ op: 'set_node', grade, id, params: more.params }, ...connect(grade, { from, to: id, kind, input: more.count })];
}

/** Morph Into Layer Mixer Node, or Into Parallel Node (pp. 3336–3337): the mixer keeps its id, label, inputs and links. */
export function morph(grade: Target, of: Grade, id: string): Worked {
  const node = nodeOf(of, id);
  if (!node || (node.kind !== 'parallel_mixer' && node.kind !== 'layer_mixer')) return null;
  const into: GradeNode = node.kind === 'parallel_mixer'
    ? { id: node.id, label: node.label, enabled: node.enabled, kind: 'layer_mixer', params: { inputs: node.params.inputs, composite: 'normal' } }
    : { id: node.id, label: node.label, enabled: node.enabled, kind: 'parallel_mixer', params: { inputs: node.params.inputs } };
  return [{ op: 'set_grade', grade, value: { nodes: of.nodes.map(each => (each.id === id ? into : each)), links: of.links } }];
}

/** The composite mode of a Layer Mixer (pp. 1055–1058, 3337). */
export function composite(grade: Target, of: Grade, id: string, mode: Composite): Worked {
  const node = nodeOf(of, id);
  if (!node || node.kind !== 'layer_mixer' || node.params.composite === mode) return null;
  return [{ op: 'set_node', grade, id, params: { composite: mode } }];
}

/** Copy one node's settings onto another's (Alt+drag, or Ctrl+C and Ctrl+V in the node editor, pp. 3286, 3320). */
export function pasteNode(from: Target, fromNode: string, to: Target, toNode: string): Worked {
  if (fromNode === toNode && JSON.stringify(from) === JSON.stringify(to)) return null;
  return [{ edit: 'paste_node', from, from_node: fromNode, to, to_node: toNode }];
}

/**
 * Swap what two correctors do (Ctrl+drag one onto the other, p. 3320): each takes the settings of
 * the other, and both stay where they are. The engine merges a patch of `params` object by object,
 * so the whole settings of one, given as a patch, replace the other's.
 */
export function swapNodes(grade: Target, of: Grade, a: string, b: string): Worked {
  const one = nodeOf(of, a);
  const other = nodeOf(of, b);
  if (!one || !other || a === b) return null;
  if (one.kind !== 'corrector' || other.kind !== 'corrector') return { refused: 'wrong_node' };
  return [
    { op: 'set_node', grade, id: a, params: other.params as unknown as Record<string, unknown> },
    { op: 'set_node', grade, id: b, params: one.params as unknown as Record<string, unknown> },
  ];
}

/**
 * A loose corrector dropped on a link goes into it (p. 3319): the link's output feeds the node,
 * and the node feeds the input that the link went into, in its place.
 */
export function insertOnLink(grade: Target, of: Grade, id: string, link: Link): Worked {
  const node = nodeOf(of, id);
  if (!node || node.kind !== 'corrector' || link.from === id || link.to === id) return null;
  if (of.links.some(each => each.from === id || each.to === id)) return null;
  return [
    ...connect(grade, { from: link.from, to: id, kind: link.kind, input: 0 }),
    ...connect(grade, { from: id, to: link.to, kind: link.kind, input: link.input }),
  ];
}

/**
 * Apply Grade From One Clip Prior (=) or Two Clips Prior (-) (p. 3288): the grade of the clip one
 * or two before the current one, in the order of the thumbnail timeline, copied onto it.
 */
export function applyPrior(timeline: string, clips: readonly Clip[], current: string | null, back: 1 | 2): Worked {
  const at = current === null ? -1 : clips.findIndex(clip => clip.id === current);
  if (at < back) return null;
  const from: Target = { level: 'clip', timeline, clip: clips[at - back].id };
  return [{ edit: 'copy_grade', from, to: [{ level: 'clip', timeline, clip: clips[at].id }] }];
}

/** Paste a whole grade, copied with Ctrl+C in the thumbnail timeline, onto the clips chosen (Ctrl+V, p. 3286). */
export function pasteGrade(from: Target, to: readonly Target[]): Worked {
  const others = to.filter(each => JSON.stringify(each) !== JSON.stringify(from));
  return others.length === 0 ? null : [{ edit: 'copy_grade', from, to: others }];
}

/** Collapse Group Grades on clips (pp. 3306–3307): those in no group are left out, as the engine would refuse them. */
export function collapseGroups(timeline: string, clips: readonly Clip[]): Worked {
  const grouped = clips.filter(clip => clip.group !== null);
  return grouped.length === 0 ? null : [{ edit: 'collapse_group_grades', timeline, clips: grouped.map(clip => clip.id) }];
}

/** What a key asks of the node editor (the interaction reference, §2.2–2.5 and §9.4). */
export type NodeCommand =
  | { kind: 'add'; how: AddHow }
  | { kind: 'delete'; key: 'Delete' | 'Backspace' }
  | { kind: 'extract' | 'enable' | 'enableAll' | 'copy' | 'paste' | 'tool' | 'cancel' }
  | { kind: 'reset'; how: ResetHow }
  | { kind: 'step'; direction: -1 | 1 };

/**
 * The keys of the node editor, as DaVinci Resolve has them on Windows (pp. 3314–3319): Alt+S,
 * Shift+S, Alt+K, Alt+P and Alt+L add a node; Delete deletes the nodes chosen (Backspace only a
 * link chosen); E extracts; Ctrl+D enables or disables the nodes chosen and Alt+D all of them;
 * Shift+Home, Ctrl+Shift+Home and Ctrl+Home reset; Alt+Shift+; and Alt+Shift+' go to the
 * previous and the next node; Ctrl+C and Ctrl+V copy and paste a node; H changes between the
 * selection tool and the hand. A letter is known by where it lies, as Alt changes what it types on
 * some layouts. Alt+O (an outside node) and the keys of nodes with windows wait for keys and windows.
 */
export function nodeKey(key: Key): NodeCommand | null {
  if (key.composing || key.meta) return null;
  const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
  const only = (ctrl: boolean, alt: boolean, shift: boolean) => key.ctrl === ctrl && key.alt === alt && key.shift === shift;
  if (key.key === 'Home') {
    if (only(false, false, true)) return { kind: 'reset', how: 'node' };
    if (only(true, false, true)) return { kind: 'reset', how: 'keepNodes' };
    if (only(true, false, false)) return { kind: 'reset', how: 'all' };
    return null;
  }
  if (key.repeat) return null;
  if (only(false, true, true)) {
    if (key.code === 'Semicolon' || key.key === ';' || key.key === ':') return { kind: 'step', direction: -1 };
    if (key.code === 'Quote' || key.key === "'" || key.key === '"') return { kind: 'step', direction: 1 };
    return null;
  }
  if (only(false, true, false)) {
    switch (letter) {
      case 's': return { kind: 'add', how: 'serial' };
      case 'k': return { kind: 'add', how: 'append' };
      case 'p': return { kind: 'add', how: 'parallel' };
      case 'l': return { kind: 'add', how: 'layer' };
      case 'd': return { kind: 'enableAll' };
      default: return null;
    }
  }
  if (only(false, false, true)) return letter === 's' ? { kind: 'add', how: 'before' } : null;
  if (only(true, false, false)) {
    if (letter === 'd') return { kind: 'enable' };
    if (letter === 'c') return { kind: 'copy' };
    if (letter === 'v') return { kind: 'paste' };
    return null;
  }
  if (!only(false, false, false)) return null;
  if (key.key === 'Delete' || key.key === 'Backspace') return { kind: 'delete', key: key.key };
  if (key.key === 'Escape') return { kind: 'cancel' };
  if (letter === 'e') return { kind: 'extract' };
  if (letter === 'h') return { kind: 'tool' };
  return null;
}

/** What a key asks of the Color page as a whole: undo and redo, a grade from a clip before, a grade copied or pasted. */
export type PageCommand =
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'prior'; back: 1 | 2 }
  | { kind: 'copy' | 'paste' };

/**
 * The keys of the page: Ctrl+Z and Ctrl+Shift+Z undo and redo, in the one history of the engine
 * (as on the Edit page); = and - apply the grade of the clip one or two before (p. 3288); Ctrl+C
 * and Ctrl+V copy and paste whole grades where the thumbnail timeline has the focus (p. 3286).
 */
export function pageKey(key: Key): PageCommand | null {
  if (key.composing || key.meta || key.repeat) return null;
  const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
  if (key.ctrl && !key.alt) {
    if (letter === 'z') return { kind: 'history', method: key.shift ? 'history.redo' : 'history.undo' };
    if (key.shift) return null;
    if (letter === 'c') return { kind: 'copy' };
    if (letter === 'v') return { kind: 'paste' };
    return null;
  }
  if (key.ctrl || key.alt || key.shift) return null;
  if (key.key === '=') return { kind: 'prior', back: 1 };
  if (key.key === '-') return { kind: 'prior', back: 2 };
  return null;
}

/** The nodes in the order of their numbers, the one before or after `current`, round from the last to the first (p. 3314). */
export function stepNode(of: Grade, current: string | null, direction: -1 | 1): string | null {
  if (of.nodes.length === 0) return null;
  const at = of.nodes.findIndex(node => node.id === current);
  if (at < 0) return of.nodes[direction > 0 ? 0 : of.nodes.length - 1].id;
  return of.nodes[(at + direction + of.nodes.length) % of.nodes.length].id;
}

/** The composite modes of a Layer Mixer, in the order of its menu (pp. 1055–1058). */
export const COMPOSITES: readonly Composite[] = [
  'normal', 'add', 'color', 'color_burn', 'color_dodge', 'darken', 'darker_color', 'difference', 'divide', 'exclusion', 'hard_mix',
  'hardlight', 'hue', 'lighten', 'lighter_color', 'linear_burn', 'linear_dodge', 'linear_light', 'luminosity', 'multiply', 'overlay',
  'pin_light', 'saturation', 'screen', 'softlight', 'subtract', 'vivid_light',
];
