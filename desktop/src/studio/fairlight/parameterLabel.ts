import type { FairlightText } from '../../messages/fairlight';
import type { LanesOf, Timeline } from '../engine';
import { DUCKER_NUMBERS, EFFECT_NUMBERS } from '../audio/effectModel';
import { AUDIO_EFFECTS } from './EffectsLibrary';

type Text = (key: FairlightText) => string;
const fields: Record<string, FairlightText> = {
  on: 'Enable', fader_db: 'Fader', trim_db: 'Trim', mute: 'Mute', invert: 'Polarity',
  freq: 'Frequency', gain_db: 'Gain', q: 'Q', x: 'Left/Right', y: 'Front/Back',
  divergence: 'Divergence', level_db: 'Level', threshold_db: 'Threshold', range_db: 'Range',
  ratio: 'Ratio', knee_db: 'Knee', mix: 'Dry Mix', attack_ms: 'Attack', hold_ms: 'Hold',
  release_ms: 'Release', makeup_db: 'Make Up', soft: 'Soft', listen: 'Listen',
  time_smoothing: 'Time Smoothing',
};
const fieldLabel = (field: string, t: Text): string => {
  if (field.startsWith('pan.')) return `${t('pan')} · ${fieldLabel(field.slice(4), t)}`;
  if (field.startsWith('sidechain.')) return `${t('Sidechain')} · ${fieldLabel(field.slice(10), t)}`;
  return fields[field] ? t(fields[field]) : field.replaceAll('_', ' ').replace(/\b\w/g, c => c.toUpperCase());
};

/** Display names are separate from the unchanged protocol path used for edits. */
export function parameterLabel(timeline: Timeline, of: LanesOf, path: string, t: Text): string {
  const owner = 'track' in of ? timeline.tracks.find(row => row.id === of.track)
    : 'bus' in of ? timeline.buses?.find(row => row.id === of.bus) : undefined;
  const strip = owner && 'strip' in owner ? owner.strip : undefined;
  if (path.startsWith('eq.')) {
    const rest = path.slice(3), band = /^(\d+)\.(.*)$/.exec(rest);
    return [t('Equalizer'), ...(band ? [`${t('Band')} ${Number(band[1]) + 1}`, fieldLabel(band[2], t)] : [fieldLabel(rest, t)])].join(' · ');
  }
  if (path.startsWith('dynamics.')) {
    let rest = path.slice(9);
    const section = (['expander', 'compressor', 'limiter'] as const).find(key => rest.startsWith(`${key}.`));
    const labels: Record<NonNullable<typeof section>, FairlightText> = { expander: 'Expander / Gate', compressor: 'Compressor', limiter: 'Limiter' };
    if (section) rest = rest.slice(section.length + 1);
    return [t('Dynamics'), ...(section ? [t(labels[section])] : []), fieldLabel(rest, t)].join(' · ');
  }
  if (path.startsWith('insert.')) {
    const inserts = strip?.inserts ?? [];
    const index = inserts.findIndex(row => path.startsWith(`insert.${row.id}.`));
    if (index >= 0) {
      const insert = inserts[index], field = path.slice(`insert.${insert.id}.`.length);
      const number = EFFECT_NUMBERS[insert.effect.kind].find(row => row.field === field);
      const kind = AUDIO_EFFECTS.find(([kind]) => kind === insert.effect.kind)![1];
      return [`${t('Insert')} ${index + 1}`, t(kind), number ? t(number.label as FairlightText) : fieldLabel(field, t)].join(' · ');
    }
  }
  if (path.startsWith('track_fx.')) {
    const field = path.slice(9), number = DUCKER_NUMBERS.find(row => row.field === field);
    return `${t('Ducker')} · ${number ? t(number.label as FairlightText) : fieldLabel(field, t)}`;
  }
  for (const [prefix, label] of [['send', 'Send'], ['output', 'Output']] as const) {
    if (!path.startsWith(`${prefix}.`)) continue;
    const bus = timeline.buses?.find(row => path.startsWith(`${prefix}.${row.id}.`));
    if (bus) return [t(label), bus.name, fieldLabel(path.slice(`${prefix}.${bus.id}.`.length), t)].join(' · ');
  }
  return fieldLabel(path, t);
}
