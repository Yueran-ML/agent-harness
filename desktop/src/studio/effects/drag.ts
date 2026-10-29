import type { TransitionKind } from '../engine';

/** What the effects library holds: three transitions of picture, one of sound, a title and a solid colour. */
export type Effect = TransitionKind | 'title' | 'color';
export const EFFECTS: readonly Effect[] = ['dissolve', 'dip_black', 'dip_white', 'crossfade', 'title', 'color'];
export const isTransition = (effect: Effect): effect is TransitionKind => effect !== 'title' && effect !== 'color';

/**
 * A drag of an effect names it in the session of the engine. Its kind is also a type of the drag of
 * its own, because a page may read only the types while it is dragged over: where it may be dropped
 * depends on whether it is a transition.
 */
export const EFFECT_DRAG = 'application/x-columbina-effect';
const typeOf = (effect: Effect) => `${EFFECT_DRAG}-${effect.replace('_', '-')}`;

export function startEffectDrag(data: Pick<DataTransfer, 'setData' | 'effectAllowed'>, session: number, effect: Effect): void {
  data.setData(EFFECT_DRAG, JSON.stringify({ session, effect }));
  data.setData(typeOf(effect), '');
  data.effectAllowed = 'copy';
}

/** The effect that is being dragged, from the types of the drag alone; null where it is none. */
export function effectOfTypes(types: readonly string[]): Effect | null {
  if (!types.includes(EFFECT_DRAG)) return null;
  return EFFECTS.find(effect => types.includes(typeOf(effect))) ?? null;
}

/** The effect that was dropped, where it was dragged in this session. */
export function droppedEffect(data: Pick<DataTransfer, 'getData'>, session: number): Effect | null {
  try {
    const value = JSON.parse(data.getData(EFFECT_DRAG));
    return value.session === session && EFFECTS.includes(value.effect) ? value.effect : null;
  } catch { return null; }
}
