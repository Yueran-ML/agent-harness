import { useFairlightText } from './words';
import { useState } from 'react';
import type { FairlightText } from '../../messages/fairlight';
import type { Effect } from '../engine';
export const AUDIO_EFFECT_DRAG = 'application/x-columbina-audio-effect';
export const AUDIO_EFFECTS: [Effect['kind'], FairlightText][] = [['gain', 'Gain'], ['limiter', 'Limiter'], ['soft_clipper', 'Soft Clipper'], ['stereo_width', 'Stereo Width'],
  ['de_esser', 'De-Esser'], ['de_hummer', 'De-Hummer'], ['noise_reduction', 'Noise Reduction'], ['pitch', 'Pitch']];
const FAVORITES = 'columbina.fairlight.fx-favorites';
export function EffectsLibrary() {
  const t = useFairlightText();
  const [favorites, setFavorites] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem(FAVORITES) ?? '[]'); } catch { return []; } });
  const toggle = (id: string) => { const next = favorites.includes(id) ? favorites.filter(each => each !== id) : [...favorites, id]; setFavorites(next); localStorage.setItem(FAVORITES, JSON.stringify(next)); };
  return <section className="fairlight-effects" aria-label={t('Audio effects')}><h2>{t('Audio FX')}</h2><p>{t('Drag to an audio clip or track.')}</p>
    {[...AUDIO_EFFECTS].sort(([a], [b]) => Number(favorites.includes(b)) - Number(favorites.includes(a))).map(([id, name]) =>
      <div key={id} draggable onDragStart={event => event.dataTransfer.setData(AUDIO_EFFECT_DRAG, id)} className="fairlight-effect">
        <span>{t(name)}</span><button type="button" aria-label={`${t('Favorite')} ${t(name)}`} aria-pressed={favorites.includes(id)} onClick={() => toggle(id)}>☆</button>
      </div>)}
  </section>;
}
