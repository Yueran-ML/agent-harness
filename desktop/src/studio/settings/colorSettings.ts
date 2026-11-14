/**
 * What the Color Management panel of the Project Settings shows and sends (Reference Manual 20,
 * pp. 149–153, 224–252; the interaction reference, §7), worked out apart from drawing it: the
 * spaces of the engine's catalogue and the ODTs of each version of ACES, by the names the engine
 * keeps them by; which settings are shown for the science, the preset and the rest (the "applies"
 * of PROTOCOL.md, "The project's `color`", narrowed as Resolve shows them); and the patch of
 * `set_color_settings` that each change sends, with what has to go along with it.
 *
 * The lists are the engine's (`engine/crates/model/src/color_management.rs`); a test reads that
 * file and checks them one for one.
 */
import type { ColorPreset, ColorScience, ColorSettings, ResizeSpace, ToneMapping, WorkingLuminance } from '../engine';

/** The gamuts and the gammas of the spaces of media, in the order of the catalogue. */
export const MEDIA_GAMUTS = ['Rec.709', 'Rec.601 (PAL)', 'Rec.601 (NTSC)', 'Rec.2020', 'DCI-P3', 'P3-D65'] as const;
export const MEDIA_GAMMAS = ['Gamma 2.4', 'Gamma 2.2', 'Gamma 2.6', 'Gamma 2.8', 'sRGB', 'Linear', 'ST2084', 'HLG'] as const;

/** The spaces of cameras and of grading, in the order of the catalogue. */
export const CAMERA_SPACES = [
  'Rec.709 (Scene)', 'Apple Log', 'Apple Log 2', 'DJI D-Gamut/D-Log', 'Sony S-Gamut3/S-Log3', 'Sony S-Gamut3.Cine/S-Log3',
  'Panasonic V-Gamut/V-Log', 'Canon Cinema Gamut/Canon Log 2', 'Canon Cinema Gamut/Canon Log 3', 'ARRI Wide Gamut 3/LogC3',
  'ARRI Wide Gamut 4/LogC4', 'FujiFilm F-Gamut/F-Log2', 'Blackmagic Design Wide Gamut Gen 5/Film Gen 5', 'RED Wide Gamut RGB/Log3G10',
  'DaVinci Wide Gamut/DaVinci Intermediate', 'DaVinci Wide Gamut/Linear', 'ACEScct', 'ACEScc', 'ACEScg', 'ACES (AP0) Linear',
] as const;

/** The name of a space of media, as the engine names it: `sRGB`, `Rec.2100 ST2084` and `Rec.2100 HLG` by their own names. */
export function mediaSpaceName(gamut: string, gamma: string): string {
  if (gamut === 'Rec.709' && gamma === 'sRGB') return 'sRGB';
  if (gamut === 'Rec.2020' && gamma === 'ST2084') return 'Rec.2100 ST2084';
  if (gamut === 'Rec.2020' && gamma === 'HLG') return 'Rec.2100 HLG';
  return `${gamut} ${gamma}`;
}

/** The 68 spaces of the catalogue, in its order: each gamut of media with each gamma, then those of cameras. */
export const SPACES: readonly string[] = [
  ...MEDIA_GAMUTS.flatMap(gamut => MEDIA_GAMMAS.map(gamma => mediaSpaceName(gamut, gamma))),
  ...CAMERA_SPACES,
];
/** The value of Bypass, where a space may be bypassed (the input and the output, and media, p. 245). */
export const BYPASS = 'bypass';
/** The input of every preset but Custom (p. 230), and the space taken where none is known. */
export const REC709 = 'Rec.709 Gamma 2.4';

/** Whether a space is of high dynamic range: one of PQ or HLG. Bypass is none. */
export function isHdrSpace(space: string): boolean {
  return / (ST2084|HLG)$/.test(space);
}

/** The ODTs of each version of ACES, in the order of the engine (`AcesVersion::odts`). */
export const ODTS: Readonly<Record<ColorSettings['aces_version'], readonly string[]>> = {
  '1.3': [
    'Rec.709', 'Rec.709 (D60 sim.)', 'sRGB', 'sRGB (D60 sim.)', 'Display P3', 'Display P3 (D60 sim.)', 'P3-D60', 'P3-D65',
    'P3-D65 (D60 sim.)', 'P3-D65 (Rec.709 Limited)', 'P3-DCI (D60 sim.)', 'P3-DCI (D65 sim.)', 'Rec.2020', 'Rec.2020 (P3-D65 Limited)',
    'Rec.2020 (Rec.709 Limited)', 'Rec.2020 HLG (1000 nits)', 'Rec.2020 ST2084 (1000 nits)', 'Rec.2020 ST2084 (2000 nits)',
    'Rec.2020 ST2084 (4000 nits)', 'P3-D65 ST2084 (108 nits)', 'P3-D65 ST2084 (1000 nits)', 'P3-D65 ST2084 (2000 nits)',
    'P3-D65 ST2084 (4000 nits)', 'ACEScct', 'ACEScc', 'ACEScg', 'No Output Device Transform',
  ],
  '2.0': [
    'Rec.709', 'Rec.709 (Gamma 2.2)', 'sRGB', 'Display P3', 'Display P3 HDR (1000 nits)', 'P3-D65', 'P3-D65 (Rec.709 Limited)',
    'P3-D65 ST2084 (108 nits)', 'P3-D65 ST2084 (500 nits)', 'P3-D65 ST2084 (1000 nits)', 'P3-D65 ST2084 (2000 nits)',
    'P3-D65 ST2084 (4000 nits)', 'Rec.2020 ST2084 (500 nits)', 'Rec.2020 ST2084 (1000 nits)', 'Rec.2020 ST2084 (2000 nits)',
    'Rec.2020 ST2084 (4000 nits)', 'Rec.2020 ST2084 (500 nits, P3-D65 Limited)', 'Rec.2020 ST2084 (1000 nits, P3-D65 Limited)',
    'Rec.2020 ST2084 (2000 nits, P3-D65 Limited)', 'Rec.2020 ST2084 (4000 nits, P3-D65 Limited)', 'Rec.2020 HLG (1000 nits, P3-D65 Limited)',
    'Display P3 HDR (100 nits, Rec.709 Limited)', 'Rec.2020 ST2084 (100 nits, Rec.709 Limited)', 'P3-D65 ST2084 (100 nits, Rec.709 Limited)',
    'ACEScct', 'ACEScc', 'ACEScg', 'No Output Device Transform',
  ],
};
/** The ODT that both versions have, which a change of version takes where the other lacks the one chosen. */
export const COMMON_ODT = 'Rec.709';

/**
 * The output that goes with each preset, sent with it so that the output follows the preset as it
 * does in Resolve (the engine's `Preset::settings().output`, PROTOCOL.md's table of presets).
 */
export const PRESET_OUTPUT: Readonly<Record<Exclude<ColorPreset, 'custom'>, string>> = {
  sdr_rec709: REC709,
  sdr_p3_broadcast: 'P3-D65 Gamma 2.4',
  sdr_p3_cinema: 'DCI-P3 Gamma 2.6',
  sdr_rec2020: 'Rec.2020 Gamma 2.4',
  davinci_wide_gamut: REC709,
  hdr_p3_broadcast: 'P3-D65 ST2084',
  hdr_rec2020: 'Rec.2100 ST2084',
};
/** The output that goes with each Color processing mode of Automatic color management: that of the preset it is taken as (ours). */
export const PROCESSING_OUTPUT: Readonly<Record<ColorSettings['processing_mode'], string>> = {
  sdr: PRESET_OUTPUT.sdr_rec709, hdr: PRESET_OUTPUT.hdr_rec2020,
};

// The names of the menus that Resolve writes the same in every language: the sciences, the presets,
// the working luminances, the gamuts, and the rest that are names rather than words. What is a word
// (Custom, None, Simple, Linear …) is in the tables of words.

/** The Color science menu, in Resolve's order (pp. 149, 224–226). */
export const SCIENCES: readonly ColorScience[] = ['yrgb', 'yrgb_managed', 'acescc', 'acescct'];
export const SCIENCE_NAMES: Readonly<Record<ColorScience, string>> = {
  yrgb: 'DaVinci YRGB', yrgb_managed: 'DaVinci YRGB Color Managed', acescc: 'DaVinci ACEScc', acescct: 'DaVinci ACEScct',
};
/** The presets of Resolve color management (pp. 230–232); Custom is a word. */
export const PRESETS: readonly ColorPreset[] = [
  'sdr_rec709', 'sdr_p3_broadcast', 'sdr_p3_cinema', 'sdr_rec2020', 'davinci_wide_gamut', 'hdr_p3_broadcast', 'hdr_rec2020', 'custom',
];
export const PRESET_NAMES: Readonly<Record<Exclude<ColorPreset, 'custom'>, string>> = {
  sdr_rec709: 'SDR Rec.709', sdr_p3_broadcast: 'SDR P3 Broadcast', sdr_p3_cinema: 'SDR P3 Cinema', sdr_rec2020: 'SDR Rec.2020',
  davinci_wide_gamut: 'DaVinci Wide Gamut', hdr_p3_broadcast: 'HDR P3 Broadcast', hdr_rec2020: 'HDR Rec.2020',
};
/** Timeline working luminance (p. 238); Custom is a word. */
export const LUMINANCES: readonly WorkingLuminance[] = ['sdr_100', 'hdr_500', 'hdr_1000', 'hdr_2000', 'hdr_4000', 'hdr_er_1000_2000', 'custom'];
export const LUMINANCE_NAMES: Readonly<Record<Exclude<WorkingLuminance, 'custom'>, string>> = {
  sdr_100: 'SDR 100', hdr_500: 'HDR 500', hdr_1000: 'HDR 1000', hdr_2000: 'HDR 2000', hdr_4000: 'HDR 4000', hdr_er_1000_2000: 'HDR ER 1000/2000',
};
/** Color processing mode (p. 230). */
export const PROCESSING_NAMES: Readonly<Record<ColorSettings['processing_mode'], string>> = { sdr: 'SDR', hdr: 'HDR' };
/** Limit output gamut to (p. 239): None is a word. */
export const GAMUTS: readonly NonNullable<ColorSettings['limit_gamut']>[] = ['rec709', 'rec601-pal', 'rec601-ntsc', 'rec2020', 'dci-p3', 'p3-d65'];
export const GAMUT_NAMES: Readonly<Record<NonNullable<ColorSettings['limit_gamut']>, string>> = {
  rec709: 'Rec.709', 'rec601-pal': 'Rec.601 (PAL)', 'rec601-ntsc': 'Rec.601 (NTSC)', rec2020: 'Rec.2020', 'dci-p3': 'DCI-P3', 'p3-d65': 'P3-D65',
};
/** The DRTs (pp. 239–242), in Resolve's order; DaVinci is a name, the others words. RED IPP2 is not offered. */
export const DRTS: readonly ToneMapping[] = ['none', 'simple', 'luminance', 'davinci', 'saturation_preserving'];
export const DAVINCI = 'DaVinci';
/** Apply resize transformations in (p. 243); Log is a name, the others words. */
export const RESIZES: readonly ResizeSpace[] = ['timeline', 'log', 'linear', 'linear_mapped', 'gamma', 'gamma_mapped'];
export const LOG = 'Log';
export const ACES_VERSIONS: readonly ColorSettings['aces_version'][] = ['1.3', '2.0'];
/** Process Node LUTs in (p. 250). */
export const NODE_LUT_NAMES: Readonly<Record<ColorSettings['node_luts_in'], string>> = {
  timeline: 'ACEScc AP1 Timeline Space', ap0_linear: 'ACES AP0 Linear',
};

/** A setting of the panel: a field of the project's `color`. */
export type ColorField = keyof ColorSettings;

/**
 * The settings of the Color Management panel shown for these settings, in the order of the panel
 * (the interaction reference, §7.1): the science; under Resolve color management, Automatic color
 * management with its processing mode or else the preset, the output, and under Custom the rest,
 * each where it means something (the nits of a custom working luminance, the rolloffs of a
 * Saturation Preserving DRT, the 203 nits reference only for an HDR output of an SDR timeline,
 * p. 239); under ACES its five; and the Lookup Tables, which every science has.
 */
export function colorFieldsShown(settings: ColorSettings): ColorField[] {
  const shown: ColorField[] = ['science'];
  if (settings.science === 'yrgb_managed') {
    shown.push('automatic', settings.automatic ? 'processing_mode' : 'preset', 'output');
    if (!settings.automatic && settings.preset === 'custom') {
      shown.push('separate_gamma', 'input', 'timeline', 'working_luminance');
      if (settings.working_luminance === 'custom') shown.push('custom_luminance');
      if (isHdrSpace(settings.output) && !isHdrSpace(settings.timeline)) shown.push('hdr_203_nits');
      shown.push('limit_gamut', 'input_drt');
      if (settings.input_drt === 'saturation_preserving') shown.push('input_sat_rolloff');
      shown.push('output_drt');
      if (settings.output_drt === 'saturation_preserving') shown.push('output_sat_rolloff');
      shown.push('inverse_drt', 'white_point_adaptation', 'color_space_aware_tools', 'resize_in');
    }
    // With Automatic color management, only the processing mode and the output are left (p. 230).
    if (!settings.automatic) shown.push('graphics_white');
  } else if (settings.science === 'acescct' || settings.science === 'acescc') {
    shown.push('aces_version', 'idt', 'odt', 'gamut_compress', 'node_luts_in');
  }
  shown.push('input_lut', 'output_lut', 'viewer_lut', 'lut_interpolation');
  return shown;
}

/** Whether the panel shows the spaces as a gamut and a gamma (Use Separate Color Space and Gamma, p. 234), which only Custom has. */
export function separateGamma(settings: ColorSettings): boolean {
  return settings.science === 'yrgb_managed' && !settings.automatic && settings.preset === 'custom' && settings.separate_gamma;
}

/** The patch of a preset chosen: the preset, and the output that goes with it (Custom keeps the output). */
export function presetPatch(preset: ColorPreset): Partial<ColorSettings> {
  return preset === 'custom' ? { preset } : { preset, output: PRESET_OUTPUT[preset] };
}

/** The patch of a processing mode chosen under Automatic color management: the mode, and its output. */
export function processingPatch(mode: ColorSettings['processing_mode']): Partial<ColorSettings> {
  return { processing_mode: mode, output: PROCESSING_OUTPUT[mode] };
}

/**
 * The patch of a version of ACES chosen: the version, and where it has no ODT of the name chosen,
 * Rec.709, which both have (the engine refuses a version alone whose ODT it lacks, `unknown_space`).
 */
export function acesVersionPatch(version: ColorSettings['aces_version'], odt: string): Partial<ColorSettings> {
  return ODTS[version].includes(odt) ? { aces_version: version } : { aces_version: version, odt: COMMON_ODT };
}

// The draft of the window. Resolve applies the Project Settings when Save is clicked (p. 139): the
// window edits a draft, the edits of each setting kept apart (to the field of a field, so that the
// limit of a rolloff and the 3D LUT of a stage are edits of their own), and shown over the project
// as it is now. A change of the project meanwhile (another client, an undo) so shows at once in
// what was not edited here, and Save sends only what differs from the project then.

type Fields = Record<string, unknown>;
const isFields = (value: unknown): value is Fields => typeof value === 'object' && value !== null && !Array.isArray(value);

/** The draft with an edit made: the fields of `patch` over those of `draft`, a field of a field kept apart. */
export function withEdit(draft: Fields, patch: Fields): Fields {
  const next: Fields = { ...draft };
  for (const [name, value] of Object.entries(patch)) {
    next[name] = isFields(value) && isFields(draft[name]) ? withEdit(draft[name] as Fields, value) : value;
  }
  return next;
}

/** The settings as the window shows them: the draft's edits over the settings of the project now. */
export function drafted(settings: ColorSettings, draft: Fields): ColorSettings {
  const over = (base: unknown, edits: Fields): Fields => {
    const shown: Fields = { ...(base as Fields) };
    for (const [name, value] of Object.entries(edits)) shown[name] = isFields(value) && isFields(shown[name]) ? over(shown[name], value) : value;
    return shown;
  };
  return over(settings, draft) as unknown as ColorSettings;
}

/**
 * The patch of `set_color_settings` that Save sends: each edit of the draft that differs from the
 * settings of the project now, a field of a field alone (the engine merges it), or null where
 * none does.
 */
export function draftChanges(settings: ColorSettings, draft: Fields): Fields | null {
  const changes = (base: unknown, edits: Fields): Fields | null => {
    const changed: Fields = {};
    for (const [name, value] of Object.entries(edits)) {
      const was = isFields(base) ? base[name] : undefined;
      if (isFields(value) && isFields(was)) {
        const inner = changes(was, value);
        if (inner !== null) changed[name] = inner;
      } else if (!Object.is(was, value)) {
        changed[name] = value;
      }
    }
    return Object.keys(changed).length === 0 ? null : changed;
  };
  return changes(settings, draft);
}

/** The input space that media takes where none is set by hand and its file says none: the project's, as it is in force. */
export function projectInput(settings: ColorSettings): string {
  if (settings.science === 'acescct' || settings.science === 'acescc') return settings.idt;
  return settings.science === 'yrgb_managed' && !settings.automatic && settings.preset === 'custom' ? settings.input : REC709;
}

// A space as a gamut and a gamma (Use Separate Color Space and Gamma, p. 234). A space of media is
// its gamut and its gamma; one of a camera is cut at its slash (Sony S-Gamut3 and S-Log3), and one
// without a slash is its own gamut and gamma (Apple Log, ACEScct). How Resolve names the gamuts of
// cameras in this mode the manual does not say: this is ours.

/** A space as a gamut and a gamma. */
export function splitSpace(space: string): { gamut: string; gamma: string } {
  for (const gamut of MEDIA_GAMUTS) {
    for (const gamma of MEDIA_GAMMAS) if (mediaSpaceName(gamut, gamma) === space) return { gamut, gamma };
  }
  const slash = space.indexOf('/');
  return slash < 0 ? { gamut: space, gamma: space } : { gamut: space.slice(0, slash), gamma: space.slice(slash + 1) };
}

/** The gamuts of the catalogue, in its order. */
export const SPLIT_GAMUTS: readonly string[] = [...new Set(SPACES.map(space => splitSpace(space).gamut))];

/** The gammas that a gamut has in the catalogue, in its order. */
export function gammasOf(gamut: string): string[] {
  return SPACES.map(splitSpace).filter(each => each.gamut === gamut).map(each => each.gamma);
}

/** The space of a gamut and a gamma, or null where the catalogue has none. */
export function joinSpace(gamut: string, gamma: string): string | null {
  return SPACES.find((space) => { const split = splitSpace(space); return split.gamut === gamut && split.gamma === gamma; }) ?? null;
}

/** The space of a gamut chosen anew: with the gamma it had where the gamut has that one, else the first of the gamut. */
export function withGamut(space: string, gamut: string): string {
  const gamma = space === BYPASS ? null : splitSpace(space).gamma;
  return (gamma !== null ? joinSpace(gamut, gamma) : null) ?? joinSpace(gamut, gammasOf(gamut)[0])!;
}

