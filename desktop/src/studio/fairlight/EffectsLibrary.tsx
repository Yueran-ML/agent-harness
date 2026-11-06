import { useFairlightText } from './words';
import { useEffectFavorites } from '../audio/effectFavorites';
import type { FairlightText } from '../../messages/fairlight';
import type { Effect } from '../engine';
export const AUDIO_EFFECT_DRAG = 'application/x-columbina-audio-effect';
export const AUDIO_EFFECTS: [Effect['kind'], FairlightText][] = [['gain', 'Gain'], ['limiter', 'Limiter'], ['soft_clipper', 'Soft Clipper'], ['stereo_width', 'Stereo Width'],
  ['de_esser', 'De-Esser'], ['de_hummer', 'De-Hummer'], ['noise_reduction', 'Noise Reduction'], ['pitch', 'Pitch']];
/**
 * The Audio FX of the Effects Library (Reference Manual 20, pp. 3696–3697): each is dragged to a
 * clip or a track, and its star adds it to the favourites, which are listed again on their own
 * below the library, in the order they were added.
 */
export function EffectsLibrary() {
  const t = useFairlightText();
  const [favorites, toggle] = useEffectFavorites();
  const effect = ([id, name]: (typeof AUDIO_EFFECTS)[number], star: boolean) =>
    <div key={id} draggable onDragStart={event => event.dataTransfer.setData(AUDIO_EFFECT_DRAG, id)} className="fairlight-effect">
      <span>{t(name)}</span>
      {star && <button type="button" aria-label={`${t('Favorite')} ${t(name)}`} aria-pressed={favorites.includes(id)} onClick={() => toggle(id)}>☆</button>}
    </div>;
  const chosen = favorites.flatMap(id => AUDIO_EFFECTS.filter(([kind]) => kind === id));
  return <section className="fairlight-effects" aria-label={t('Audio effects')}><h2>{t('Audio FX')}</h2><p>{t('Drag to an audio clip or track.')}</p>
    {AUDIO_EFFECTS.map(each => effect(each, true))}
    {chosen.length > 0 && <section className="fairlight-favorites" aria-label={t('Favorites')}><h2>{t('Favorites')}</h2>
      {chosen.map(each => effect(each, false))}</section>}
  </section>;
}
