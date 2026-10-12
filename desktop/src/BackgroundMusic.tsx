import { useCallback, useEffect, useRef, useState } from 'react';
import { LoaderCircle, Music2, Play, Trash2, Upload, Volume2, VolumeX } from 'lucide-react';
import { useWords } from './app/locale';
import './BackgroundMusic.css';

type Track = { name: string; blob: Blob; source?: 'local' };
type StoredSelection = Track | { disabled: true };
type Playback = 'empty' | 'loading' | 'playing' | 'blocked' | 'failed';
/** What went wrong with a file. The interface says each in its own words. */
type Problem = 'cannotPlay' | 'cannotOpenKept' | 'wrongKind' | 'wrongSize' | 'noKept';
/** What could not be kept from one start of the application to the next. */
type Unkept = 'cannotRead' | 'thisWindowOnly' | 'notCleared' | 'choiceNotKept';
const storageKey = 'columbina-music-muted';
const acceptedFiles = '.mp3,.wav,.m4a,.ogg,.flac,audio/mpeg,audio/wav,audio/mp4,audio/ogg,audio/flac';

async function readLocalTrack(): Promise<Track | null> {
  const local = await window.columbina?.getLocalMusic();
  return local ? { name: local.name, blob: new Blob([local.bytes], { type: local.mimeType }), source: 'local' } : null;
}

async function musicStore<T>(mode: IDBTransactionMode, action: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('columbina-background-music', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('tracks');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    return await new Promise<T>((resolve, reject) => {
      const transaction = database.transaction('tracks', mode);
      const request = action(transaction.objectStore('tracks'));
      transaction.oncomplete = () => resolve(request.result);
      transaction.onerror = () => reject(transaction.error ?? request.error);
      transaction.onabort = () => reject(transaction.error ?? request.error);
    });
  } finally { database.close(); }
}

export function useBackgroundMusic() {
  const audio = useRef<HTMLAudioElement | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const [track, setTrack] = useState<Track | null>(null);
  const [busy, setBusy] = useState(true);
  const [muted, setMuted] = useState(() => { try { return localStorage.getItem(storageKey) === 'true'; } catch { return false; } });
  const [playback, setPlayback] = useState<Playback>('empty');
  const [problem, setProblem] = useState<Problem | null>(null);
  const [unkept, setUnkept] = useState<Unkept | null>(null);

  const play = useCallback(async () => {
    const element = audio.current;
    if (!element?.src) return;
    try {
      await element.play();
      if (element === audio.current && !element.paused) setPlayback('playing');
    } catch (failure) {
      if (element !== audio.current || (failure instanceof DOMException && failure.name === 'AbortError')) return;
      if (failure instanceof DOMException && failure.name === 'NotAllowedError') setPlayback('blocked');
      else { setPlayback('failed'); setProblem('cannotPlay'); }
    }
  }, []);

  useEffect(() => {
    let active = true;
    const restore = async () => {
      let saved: StoredSelection | undefined;
      try { saved = await musicStore<StoredSelection | undefined>('readonly', store => store.get('current')); }
      catch { if (active) setUnkept('cannotRead'); }
      if (!active || (saved && 'disabled' in saved)) return;
      if (saved && 'blob' in saved && saved.blob instanceof Blob && typeof saved.name === 'string') { setTrack(saved); return; }
      try {
        const local = await readLocalTrack();
        if (active && local) setTrack(local);
      } catch { if (active) setProblem('cannotOpenKept'); }
    };
    void restore().finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    if (!track) { setPlayback('empty'); return; }
    const element = new Audio();
    const url = URL.createObjectURL(track.blob);
    audio.current = element;
    element.loop = true;
    element.volume = 0.45;
    element.muted = muted;
    element.preload = 'auto';
    element.src = url;
    setPlayback('loading');
    const onError = () => { setPlayback('failed'); setProblem('cannotPlay'); };
    const onPause = () => { if (!element.ended && !element.error) setPlayback('blocked'); };
    element.addEventListener('error', onError);
    element.addEventListener('pause', onPause);
    void play();
    return () => {
      element.removeEventListener('error', onError);
      element.removeEventListener('pause', onPause);
      audio.current = null;
      element.pause();
      element.removeAttribute('src');
      element.load();
      URL.revokeObjectURL(url);
    };
  }, [track, play]);

  useEffect(() => {
    if (audio.current) { audio.current.muted = muted; void play(); }
    try { localStorage.setItem(storageKey, String(muted)); } catch { /* The current window still keeps the chosen sound state. */ }
  }, [muted, play]);

  const chooseFile = useCallback(() => input.current?.click(), []);
  const importFile = async (file: File) => {
    if (!/\.(mp3|wav|m4a|ogg|flac)$/i.test(file.name)) { setProblem('wrongKind'); return; }
    if (file.size === 0 || file.size > 100 * 1024 * 1024) { setProblem('wrongSize'); return; }
    setBusy(true);
    setProblem(null);
    const next = { name: file.name.replace(/\.[^.]+$/, ''), blob: file };
    try {
      await musicStore('readwrite', store => store.put(next, 'current'));
      setUnkept(null);
    } catch { setUnkept('thisWindowOnly'); }
    setTrack(next);
    setBusy(false);
  };
  const remove = async () => {
    setBusy(true);
    try {
      await musicStore('readwrite', store => store.put({ disabled: true }, 'current'));
      setTrack(null);
      setProblem(null);
      setUnkept(null);
    } catch {
      setTrack(null);
      setProblem(null);
      setUnkept('notCleared');
    }
    finally { setBusy(false); }
  };
  const restoreLocal = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const local = await readLocalTrack();
      if (!local) { setProblem('noKept'); return; }
      try { await musicStore('readwrite', store => store.delete('current')); setUnkept(null); }
      catch { setUnkept('choiceNotKept'); }
      setTrack(local);
    } catch { setProblem('cannotOpenKept'); }
    finally { setBusy(false); }
  };

  /** Where the music stands, as one of the things the interface has words for. */
  const stands = busy || (playback === 'loading' && track) ? 'loading' as const
    : !track ? 'none' as const
      : playback === 'failed' ? 'unavailable' as const
        : playback === 'blocked' ? 'blocked' as const
          : muted ? 'muted' as const : 'playing' as const;
  const fileInput = <input ref={input} className="music-file-input" type="file" accept={acceptedFiles} aria-hidden="true" tabIndex={-1}
    onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importFile(file); }} />;
  return { hasTrack: !!track, name: track?.name, localTrack: track?.source === 'local', canRestoreLocal: !!window.columbina?.getLocalMusic, busy, muted, setMuted, playback, problem, unkept, stands, play, chooseFile, remove, restoreLocal, fileInput };
}

export type BackgroundMusic = ReturnType<typeof useBackgroundMusic>;

/**
 * The music in the bar of the window: what plays, and a way to silence it.
 * Without music there is nothing to show, and the settings are where some is chosen.
 */
export function MusicTitlebar({ music }: { music: BackgroundMusic }) {
  const words = useWords();
  const sound = music.muted ? words.label.unmute : words.label.mute;
  const waits = music.playback === 'blocked';
  return <div className="titlebar-music" data-startup-control>
    {music.fileInput}
    {music.hasTrack && <>
      <button
        type="button"
        className="titlebar-music-track"
        disabled={music.busy}
        title={`${music.name} · ${words.label.music.stands[music.stands]}`}
        aria-label={waits ? words.label.music.play : words.label.music.change}
        onClick={() => (waits ? void music.play() : music.chooseFile())}
      >
        {music.busy ? <LoaderCircle size={13} className="spin" /> : waits ? <Play size={13} /> : <Music2 size={13} />}
        <span>{music.name}</span>
      </button>
      <button type="button" className="icon-btn" aria-label={sound} aria-pressed={music.muted} title={sound} onClick={() => music.setMuted(!music.muted)}>
        {music.muted ? <VolumeX size={14} /> : <Volume2 size={14} />}
      </button>
    </>}
  </div>;
}

/** The music in the settings: which file plays, and the ways to change that. */
export function MusicSettings({ music }: { music: BackgroundMusic }) {
  const words = useWords();
  const sound = music.muted ? words.label.unmute : words.label.mute;
  return <div className="music-settings">
    <div className="music-card">
      <Music2 size={18} className="music-card-icon" aria-hidden="true" />
      <div className="music-card-words">
        <strong>{music.name ?? words.label.music.none}</strong>
        <span className="caption" role="status">{music.hasTrack || music.busy ? words.label.music.stands[music.stands] : words.hint.music.about}</span>
      </div>
      {music.hasTrack && <button type="button" className="icon-btn" onClick={() => music.setMuted(!music.muted)} aria-label={sound} aria-pressed={music.muted} title={sound}>
        {music.muted ? <VolumeX size={16} /> : <Volume2 size={16} />}
      </button>}
    </div>
    <div className="music-actions">
      <button type="button" className="btn" onClick={music.chooseFile} disabled={music.busy}>
        <Upload size={14} aria-hidden="true" />{music.hasTrack ? words.label.music.change : words.label.music.choose}
      </button>
      {music.hasTrack && music.playback === 'blocked' && <button type="button" className="btn" onClick={() => void music.play()}>
        <Play size={14} aria-hidden="true" />{words.label.music.play}
      </button>}
      {music.canRestoreLocal && !music.localTrack && <button type="button" className="btn" disabled={music.busy} onClick={() => void music.restoreLocal()}>
        <Music2 size={14} aria-hidden="true" />{words.label.music.useKept}
      </button>}
      {music.hasTrack && <button type="button" className="btn btn-quiet" disabled={music.busy} onClick={() => void music.remove()}>
        <Trash2 size={14} aria-hidden="true" />{words.label.music.remove}
      </button>}
    </div>
    <p className="caption">{words.hint.music.kinds}</p>
    {music.problem && <p className="music-said" data-tone="bad" role="alert">{words.hint.music.problem[music.problem]}</p>}
    {music.unkept && <p className="music-said" data-tone="warn" role="status">{words.hint.music.unkept[music.unkept]}</p>}
  </div>;
}
