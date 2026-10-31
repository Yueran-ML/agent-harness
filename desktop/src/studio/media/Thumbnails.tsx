import { useEffect, useRef, useState } from 'react';
import { Film } from 'lucide-react';
import type { EngineBridge } from '../../bridge';
import { useWords } from '../../app/locale';
import { callEngine, type Media } from '../engine';
import { framesOf, rateOf } from '../timecode';

/** Cache is scoped to one project session. Bound it so a long browsing session does not keep every image. */
export class Thumbnails {
  private readonly images = new Map<string, Promise<string>>();
  constructor(private readonly bridge: Pick<EngineBridge, 'call'>) {}
  get(media: Media): Promise<string> {
    return this.at(media.id, media.mark_in ? framesOf(media.mark_in, rateOf(media)) : 0);
  }
  /** The picture of a frame of media, counted at the media's own rate. */
  at(media: string, frame: number): Promise<string> {
    const key = `${media}/${frame}`;
    const existing = this.images.get(key);
    if (existing) return existing;
    const image = callEngine(this.bridge, 'media.thumbnail', { media, frame, height: 90 })
      .then(result => `data:image/png;base64,${result.png}`);
    this.images.set(key, image);
    void image.catch(() => { if (this.images.get(key) === image) this.images.delete(key); });
    if (this.images.size > 256) this.images.delete(this.images.keys().next().value!);
    return image;
  }
}

export function Thumbnail({ media, cache }: { media: Media; cache: Thumbnails }) {
  const words = useWords();
  const box = useRef<HTMLSpanElement>(null);
  const [picture, setPicture] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let ended = false;
    let asked = false;
    setPicture(null);
    setFailed(false);
    const fetch = () => {
      if (asked || ended) return;
      asked = true;
      void cache.get(media).then(image => { if (!ended) setPicture(image); }, () => { if (!ended) setFailed(true); });
    };
    const observer = typeof IntersectionObserver === 'function' ? new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { fetch(); observer?.disconnect(); }
    }) : null;
    if (observer && box.current) observer.observe(box.current);
    else fetch();
    return () => { ended = true; observer?.disconnect(); };
  }, [cache, media.id, media.mark_in?.ticks, media.mark_in?.scale]);
  return <span ref={box} className="pool-thumbnail" title={failed ? words.hint.studio.thumbnailFailed : undefined}>
    {picture ? <img src={picture} alt="" draggable={false} /> : <Film size={24} aria-hidden="true" />}
  </span>;
}
