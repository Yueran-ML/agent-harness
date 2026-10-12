import { useCallback, useEffect, useRef, useState, type MouseEvent } from 'react';
import { ArrowRight, LoaderCircle, Moon, Play, RotateCcw, Volume2, VolumeX } from 'lucide-react';
import { keyOf } from './app/keys';
import { useWords } from './app/locale';
import { openingKey } from './app/opening';
import './StartupVideo.css';

type Playback = 'loading' | 'ready' | 'playing' | 'revealing' | 'finished' | 'failed';

export function StartupVideo({ onComplete, hasBackgroundMusic, muted, onMutedChange }: {
  onComplete: () => void;
  hasBackgroundMusic: boolean;
  muted: boolean;
  onMutedChange: (muted: boolean) => void;
}) {
  const words = useWords();
  const sound = muted ? words.label.unmute : words.label.mute;
  const container = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const entryButton = useRef<HTMLButtonElement>(null);
  const completed = useRef(false);
  const [reducedMotion, setReducedMotion] = useState(() => matchMedia('(prefers-reduced-motion: reduce)').matches);
  const [playback, setPlayback] = useState<Playback>(() => reducedMotion ? 'finished' : 'loading');
  const awaitingEntry = useRef(reducedMotion);
  const showEntry = playback === 'revealing' || playback === 'finished' || playback === 'failed';
  const playing = playback === 'playing' || playback === 'revealing';

  const finish = useCallback(() => {
    awaitingEntry.current = true;
    video.current?.pause();
    setPlayback('finished');
  }, []);

  const complete = useCallback(() => {
    if (completed.current) return;
    completed.current = true;
    video.current?.pause();
    onComplete();
  }, [onComplete]);

  const play = useCallback(async () => {
    const element = video.current;
    if (!element || completed.current) return;
    element.currentTime = 0;
    awaitingEntry.current = false;
    container.current?.focus();
    setPlayback('loading');
    try {
      await element.play();
    } catch {
      if (!completed.current && !awaitingEntry.current) setPlayback(current => current === 'failed' ? current : 'ready');
    }
  }, []);

  useEffect(() => {
    container.current?.focus();
    const preference = matchMedia('(prefers-reduced-motion: reduce)');
    const changed = () => setReducedMotion(preference.matches);
    preference.addEventListener('change', changed);
    return () => preference.removeEventListener('change', changed);
  }, []);

  useEffect(() => {
    if (reducedMotion) {
      finish();
    } else {
      void play();
    }
  }, [finish, play, reducedMotion]);

  useEffect(() => {
    if (playback === 'finished' || playback === 'failed') entryButton.current?.focus();
  }, [playback]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const place = showEntry ? 'end' : playing || playback === 'loading' ? 'film' : 'waiting';
      const action = openingKey(keyOf(event), place, event.target instanceof HTMLButtonElement);
      if (action !== null) {
        event.preventDefault();
        event.stopPropagation();
        if (action === 'play') void play();
        else complete();
      } else if (event.key === 'Tab') {
        const buttons = [
          ...document.querySelectorAll<HTMLButtonElement>('[data-startup-control] button:not(:disabled)'),
          ...(container.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []),
        ];
        if (!buttons.length) return;
        event.preventDefault();
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = current < 0 ? (event.shiftKey ? buttons.length - 1 : 0)
          : (current + (event.shiftKey ? -1 : 1) + buttons.length) % buttons.length;
        buttons[next].focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [complete, play, playback, playing, showEntry]);

  // A click leaves the opening as a key does, unless it is a click on one of its controls.
  const onClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!(event.target as Element).closest('button')) complete();
  };

  return <div ref={container} className={`startup-video${showEntry ? ' startup-video-finished' : ''}`} role="dialog" aria-label={words.label.opening.name} tabIndex={-1} onClick={onClick}>
    <video ref={video} src="./columbina-intro-extended.mp4" poster="./moonlit-columbina.png"
      autoPlay={!reducedMotion} playsInline muted preload={reducedMotion ? 'metadata' : 'auto'}
      disablePictureInPicture disableRemotePlayback aria-label={words.label.opening.name}
      onTimeUpdate={event => {
        const element = event.currentTarget;
        if (!awaitingEntry.current && !element.paused && element.duration - element.currentTime <= 3.4) {
          setPlayback(current => current === 'playing' ? 'revealing' : current);
        }
      }}
      onPlaying={() => { if (awaitingEntry.current || completed.current) video.current?.pause(); else setPlayback(current => current === 'revealing' ? current : 'playing'); }} onEnded={finish}
      onError={() => { awaitingEntry.current = true; setPlayback('failed'); }} />
    <div className="startup-video-shade" aria-hidden="true" />
    {showEntry ? <div className="startup-video-entry">
      {playback === 'failed' && <p className="startup-video-error" role="status">{words.hint.openingUnavailable}</p>}
      <div className="startup-entry-ornament" aria-hidden="true"><span /><Moon size={21} strokeWidth={1.2} /><span /></div>
      <button ref={entryButton} className="startup-video-enter" onClick={complete}>
        <span className="startup-entry-star" aria-hidden="true" /><span>Columbina</span><span className="startup-entry-star" aria-hidden="true" />
      </button>
      <div className="startup-entry-actions">
        <span className="startup-entry-hint">{words.label.opening.begin}</span>
        {playback !== 'failed' && <button className="startup-video-replay" onClick={() => void play()}><RotateCcw size={12} />{words.label.opening.again}</button>}
        {hasBackgroundMusic && <button className="startup-video-entry-sound" aria-label={sound} title={sound} onClick={() => onMutedChange(!muted)}>
          {muted ? <VolumeX size={15} /> : <Volume2 size={15} />}
        </button>}
      </div>
    </div> : !playing && <div className="startup-video-center">
      {playback === 'loading' ? <LoaderCircle className="startup-video-loading" size={26} aria-label={words.label.opening.loading} />
        : <button className="startup-video-play" onClick={() => void play()}><Play size={20} fill="currentColor" />{words.label.opening.play}<kbd>Enter</kbd></button>}
    </div>}
    {!showEntry && <div className="startup-video-controls">
      {hasBackgroundMusic && <button className="startup-video-sound" aria-label={sound}
        onClick={() => onMutedChange(!muted)}>
        {muted ? <VolumeX size={18} /> : <Volume2 size={18} />}<span>{muted ? words.label.soundOff : words.label.soundOn}</span>
      </button>}
      <span className="startup-video-hint">{words.label.opening.anyKey}</span>
      <button className="startup-video-skip" onClick={complete}>{words.label.opening.skip} <ArrowRight size={16} /></button>
    </div>}
  </div>;
}
