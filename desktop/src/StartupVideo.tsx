import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowRight, LoaderCircle, Moon, Play, RotateCcw, Volume2, VolumeX } from 'lucide-react';
import './StartupVideo.css';

type Playback = 'loading' | 'ready' | 'playing' | 'revealing' | 'finished' | 'failed';

export function StartupVideo({ onComplete, hasBackgroundMusic, muted, onMutedChange }: {
  onComplete: () => void;
  hasBackgroundMusic: boolean;
  muted: boolean;
  onMutedChange: (muted: boolean) => void;
}) {
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
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        if (playing || !showEntry) finish();
      } else if (event.key === 'Enter' && !(event.target instanceof HTMLButtonElement)) {
        event.preventDefault();
        if (showEntry) complete();
        else if (!playing) void play();
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
  }, [complete, finish, play, playing, showEntry]);

  return <div ref={container} className={`startup-video${showEntry ? ' startup-video-finished' : ''}`} role="dialog" aria-label="Columbina opening video" tabIndex={-1}>
    <video ref={video} src="./columbina-intro-extended.mp4" poster="./moonlit-columbina.png"
      autoPlay={!reducedMotion} playsInline muted preload={reducedMotion ? 'metadata' : 'auto'}
      disablePictureInPicture disableRemotePlayback aria-label="Columbina opening video"
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
      {playback === 'failed' && <p className="startup-video-error" role="status">Video unavailable</p>}
      <div className="startup-entry-ornament" aria-hidden="true"><span /><Moon size={21} strokeWidth={1.2} /><span /></div>
      <button ref={entryButton} className="startup-video-enter" onClick={complete}>
        <span className="startup-entry-star" aria-hidden="true" /><span>Columbina</span><span className="startup-entry-star" aria-hidden="true" />
      </button>
      <div className="startup-entry-actions">
        <span className="startup-entry-hint"><kbd>Enter</kbd> to begin</span>
        {playback !== 'failed' && <button className="startup-video-replay" onClick={() => void play()}><RotateCcw size={12} />Replay intro</button>}
        {hasBackgroundMusic && <button className="startup-video-entry-sound" aria-label={muted ? 'Turn sound on' : 'Mute sound'} onClick={() => onMutedChange(!muted)}>
          {muted ? <VolumeX size={15} /> : <Volume2 size={15} />}
        </button>}
      </div>
    </div> : !playing && <div className="startup-video-center">
      {playback === 'loading' ? <LoaderCircle className="startup-video-loading" size={26} aria-label="Loading opening video" />
        : <button className="startup-video-play" onClick={() => void play()}><Play size={20} fill="currentColor" />Play intro<kbd>Enter</kbd></button>}
    </div>}
    {!showEntry && <div className="startup-video-controls">
      {hasBackgroundMusic && <button className="startup-video-sound" aria-label={muted ? 'Turn sound on' : 'Mute sound'}
        onClick={() => onMutedChange(!muted)}>
        {muted ? <VolumeX size={18} /> : <Volume2 size={18} />}<span>{muted ? 'Sound off' : 'Sound on'}</span>
      </button>}
      <button className="startup-video-skip" onClick={finish}>Skip intro <ArrowRight size={16} /><kbd>Esc</kbd></button>
    </div>}
  </div>;
}
