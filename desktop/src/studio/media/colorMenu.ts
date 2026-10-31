/**
 * The colour of media in the right-click menu of the media pool (Reference Manual 20, pp. 221–222,
 * 228–230, 234–235, 245; the interaction reference, §7.3): Input Color Space ▸ (ACES Input
 * Transform ▸ under ACES), with Input Gamma ▸ where the project shows spaces as a gamut and a gamma,
 * and Data Levels ▸. What is chosen goes to every medium chosen with a picture, as one transaction
 * of `set_media_color`, which one undo takes back.
 *
 * At the top of Input Color Space, Project Default takes back what was set by hand (the manual does
 * not say how; the interaction reference's choice) and Bypass leaves the media out of colour
 * management (p. 245, which Resolve has in the menu of a thumbnail of the Color page; here it is in
 * the media pool too, as the plan asks). Resolve shows Input Color Space only where the project is
 * colour managed; Data Levels, which Resolve sets in the Clip Attributes window, is a submenu here.
 */
import type { Messages } from '../../messages/en';
import { keptOverride, type ColorSettings, type DataLevels, type Media, type Op } from '../engine';
import type { MenuItem } from '../color/ColorMenu';
import {
  BYPASS, gammasOf, joinSpace, projectInput, separateGamma, SPACES, SPLIT_GAMUTS, splitSpace, withGamut,
} from '../settings/colorSettings';

const LEVELS: readonly DataLevels[] = ['auto', 'video', 'full'];

/** The one value that every medium has, or undefined where they differ. */
function common<T>(values: readonly T[]): T | undefined {
  return values.length > 0 && values.every(value => value === values[0]) ? values[0] : undefined;
}

/**
 * The colour items of the menu of the media chosen, or none where the project is gone. `run` sends
 * the operations of one transaction; a choice that changes nothing sends nothing.
 */
export function mediaColorItems(chosen: readonly Media[], settings: ColorSettings, labels: Messages['label']['studio']['media'],
  run: (ops: Op[]) => void): MenuItem[] {
  // Media without a picture has no colour to set (the engine refuses it, `no_stream`).
  const pictures = chosen.filter(media => media.facts.video != null);
  const none = pictures.length === 0;
  const overrides = pictures.map(media => keptOverride(media));
  const sendTo = (each: (media: Media, index: number) => Partial<{ input: string | null; levels: DataLevels }> | null) => {
    const ops: Op[] = [];
    pictures.forEach((media, index) => {
      const change = each(media, index);
      if (change === null) return;
      const was = overrides[index];
      const fields = Object.fromEntries(Object.entries(change).filter(([key, value]) => was[key as keyof typeof was] !== value));
      if (Object.keys(fields).length > 0) ops.push({ op: 'set_media_color', id: media.id, ...fields } as Op);
    });
    if (ops.length > 0) run(ops);
  };

  const items: MenuItem[] = [];
  const managed = settings.science !== 'yrgb';
  if (managed) {
    const aces = settings.science === 'acescct' || settings.science === 'acescc';
    const input = common(overrides.map(each => each.input));
    const top: MenuItem[] = [
      { id: 'input-default', label: labels.projectDefault, checked: !none && input === null, run: () => sendTo(() => ({ input: null })) },
      { id: 'input-bypass', label: labels.bypass, checked: !none && input === BYPASS, run: () => sendTo(() => ({ input: BYPASS })) },
    ];
    // What each medium is now, as a space: the one set by hand, else the project's.
    const base = (index: number) => {
      const set = overrides[index].input;
      return set === null || set === BYPASS ? projectInput(settings) : set;
    };
    if (!aces && separateGamma(settings)) {
      const gamut = common(overrides.map(each => (each.input === null || each.input === BYPASS ? null : splitSpace(each.input).gamut)));
      const gammaOf = common(pictures.map((_media, index) => (overrides[index].input === null || overrides[index].input === BYPASS
        ? null : splitSpace(base(index)).gamma)));
      const shownGamut = splitSpace(base(0)).gamut;
      items.push({ id: 'input-space', label: labels.inputColorSpace, disabled: none, items: [
        ...top,
        ...SPLIT_GAMUTS.map(each => ({ id: `input-gamut-${each}`, label: each, checked: gamut === each,
          run: () => sendTo((_media, index) => ({ input: withGamut(base(index), each) })) })),
      ] });
      items.push({ id: 'input-gamma', label: labels.inputGamma, disabled: none, items: gammasOf(shownGamut).map(each => ({
        id: `input-gamma-${each}`, label: each, checked: gammaOf === each,
        run: () => sendTo((_media, index) => {
          const space = joinSpace(splitSpace(base(index)).gamut, each);
          return space === null ? null : { input: space };
        }),
      })) });
    } else {
      items.push({ id: 'input-space', label: aces ? labels.acesInput : labels.inputColorSpace, disabled: none, items: [
        ...top,
        ...SPACES.map(space => ({ id: `input-${space}`, label: space, checked: input === space, run: () => sendTo(() => ({ input: space })) })),
      ] });
    }
  }
  const levels = common(overrides.map(each => each.levels));
  items.push({ id: 'data-levels', label: labels.dataLevels, disabled: none, items: LEVELS.map(each => ({
    id: `levels-${each}`, label: labels.levels[each], checked: levels === each, run: () => sendTo(() => ({ levels: each })),
  })) });
  return items;
}

/** The reasons of the engine's refusals of the colour of media, said in the words of `hint.studio.edit`. */
export const MEDIA_COLOR_REASONS = ['camera_raw', 'unknown_space', 'no_stream'] as const;
