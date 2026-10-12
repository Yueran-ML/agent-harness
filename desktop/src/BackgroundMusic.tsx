import { useCallback, useEffect, useRef, useState } from 'react';
import { LoaderCircle, Music2, Play, Trash2, Upload, Volume2, VolumeX } from 'lucide-react';
import './BackgroundMusic.css';

type Track = { name: string; blob: Blob; source?: 'local' };
type StoredSelection = Track | { disabled: true };
type Playback = 'empty' | 'loading' | 'playing' | 'blocked' | 'failed';
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
  const [error, setError] = useState('');
  const [storageWarning, setStorageWarning] = useState('');

  const play = useCallback(async () => {
    const element = audio.current;
    if (!element?.src) return;
    try {
      await element.play();
      if (element === audio.current && !element.paused) setPlayback('playing');
    } catch (failure) {
      if (element !== audio.current || (failure instanceof DOMException && failure.name === 'AbortError')) return;
      if (failure instanceof DOMException && failure.name === 'NotAllowedError') setPlayback('blocked');
      else { setPlayback('failed'); setError('This audio file could not be played. Choose another MP3, WAV, M4A, OGG, or FLAC file.'); }
    }
  }, []);

  useEffect(() => {
    let active = true;
    const restore = async () => {
      let saved: StoredSelection | undefined;
      try { saved = await musicStore<StoredSelection | undefined>('readonly', store => store.get('current')); }
      catch { if (active) setStorageWarning('Saved music preferences are unavailable. Any changes will apply to this window only.'); }
      if (!active || (saved && 'disabled' in saved)) return;
      if (saved && 'blob' in saved && saved.blob instanceof Blob && typeof saved.name === 'string') { setTrack(saved); return; }
      try {
        const local = await readLocalTrack();
        if (active && local) setTrack(local);
      } catch { if (active) setError('The local soundtrack could not be opened. Choose another audio file.'); }
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
    const onError = () => { setPlayback('failed'); setError('This audio file could not be played. Choose another MP3, WAV, M4A, OGG, or FLAC file.'); };
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
    if (!/\.(mp3|wav|m4a|ogg|flac)$/i.test(file.name)) { setError('Choose an MP3, WAV, M4A, OGG, or FLAC audio file.'); return; }
    if (file.size === 0 || file.size > 100 * 1024 * 1024) { setError('Choose a non-empty audio file smaller than 100 MB.'); return; }
    setBusy(true);
    setError('');
    const next = { name: file.name.replace(/\.[^.]+$/, ''), blob: file };
    try {
      await musicStore('readwrite', store => store.put(next, 'current'));
      setStorageWarning('');
    } catch { setStorageWarning('This file will play for this window only. There is not enough local storage, or storage is unavailable.'); }
    setTrack(next);
    setBusy(false);
  };
  const remove = async () => {
    setBusy(true);
    try {
      await musicStore('readwrite', store => store.put({ disabled: true }, 'current'));
      setTrack(null);
      setError('');
      setStorageWarning('');
    } catch {
      setTrack(null);
      setError('');
      setStorageWarning('Music was removed from this window, but the saved copy could not be cleared and may return after restarting.');
    }
    finally { setBusy(false); }
  };
  const restoreLocal = async () => {
    setBusy(true);
    setError('');
    try {
      const local = await readLocalTrack();
      if (!local) { setError('No local soundtrack is installed on this device. Choose a local audio file.'); return; }
      try { await musicStore('readwrite', store => store.delete('current')); setStorageWarning(''); }
      catch { setStorageWarning('The local soundtrack will play for this window. Your saved selection could not be changed.'); }
      setTrack(local);
    } catch { setError('The local soundtrack could not be opened. Choose another audio file.'); }
    finally { setBusy(false); }
  };

  const status = busy ? 'Loading music…' : !track ? 'No music selected' : playback === 'failed' ? 'Audio unavailable' : playback === 'blocked' ? 'Click Play to start music' : playback === 'loading' ? 'Loading music…' : muted ? 'Muted · loops continuously' : 'Playing · loops continuously';
  const fileInput = <input ref={input} className="music-file-input" type="file" accept={acceptedFiles} aria-label="Choose background music file" tabIndex={-1}
    onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importFile(file); }} />;
  return { hasTrack: !!track, name: track?.name, localTrack: track?.source === 'local', canRestoreLocal: !!window.columbina?.getLocalMusic, busy, muted, setMuted, playback, error, storageWarning, status, play, chooseFile, remove, restoreLocal, fileInput };
}

export type BackgroundMusic = ReturnType<typeof useBackgroundMusic>;

export function MusicTitlebar({ music }: { music: BackgroundMusic }) {
  const action = music.hasTrack && music.playback === 'blocked' ? 'Play background music' : music.hasTrack ? `Change background music: ${music.name}` : 'Choose background music';
  return <div className="titlebar-music" data-startup-control>
    {music.fileInput}
    <button className="titlebar-music-track" disabled={music.busy} aria-label={action} title={`${action} · ${music.status}`}
      onClick={() => music.playback === 'blocked' && music.hasTrack ? void music.play() : music.chooseFile()}>
      {music.busy ? <LoaderCircle size={13} className="spin" /> : music.playback === 'blocked' ? <Play size={13} /> : <Music2 size={13} />}<span>{music.hasTrack ? music.name : 'Choose music'}</span>
    </button>
    <button className="titlebar-music-mute" aria-label={music.muted ? 'Turn sound on' : 'Mute sound'} aria-pressed={music.muted} title={music.muted ? 'Turn sound on' : 'Mute sound'} onClick={() => music.setMuted(!music.muted)}>
      {music.muted ? <VolumeX size={14} /> : <Volume2 size={14} />}
    </button>
  </div>;
}

export function MusicSettings({ music }: { music: BackgroundMusic }) {
  return <section className="settings-section music-settings">
    <div className="settings-section-heading"><h3>Background music</h3><p>Plays on repeat through the opening and your workspace.</p></div>
    <div className="music-track-card"><span className="music-track-icon"><Music2 size={19} /></span><div><strong>{music.name || 'Choose your soundtrack'}</strong><span role="status">{music.status}</span></div>
      {music.hasTrack && <button className="icon-button" onClick={() => music.setMuted(!music.muted)} aria-label={music.muted ? 'Unmute background music' : 'Mute background music'} title={music.muted ? 'Unmute background music' : 'Mute background music'}>{music.muted ? <VolumeX size={17} /> : <Volume2 size={17} />}</button>}
    </div>
    <div className="music-actions"><button className="secondary-button" onClick={music.chooseFile} disabled={music.busy}><Upload size={14} />{music.hasTrack ? 'Change music' : 'Choose local audio'}</button>
      {music.hasTrack && music.playback === 'blocked' && <button className="secondary-button" onClick={() => void music.play()}><Play size={14} />Play</button>}
      {music.canRestoreLocal && !music.localTrack && <button className="secondary-button" disabled={music.busy} onClick={() => void music.restoreLocal()}><Music2 size={14} />Use local soundtrack</button>}
      {music.hasTrack && <button className="secondary-button" disabled={music.busy} onClick={() => void music.remove()}><Trash2 size={14} />Remove</button>}
    </div>
    <p className="field-hint">MP3, WAV, M4A, OGG, or FLAC · up to 100 MB. Saved on this device. Sound starts on; your mute preference is remembered.</p>
    <p className="field-hint">{music.localTrack ? 'This soundtrack is stored on this device and is not included in the app package.' : 'No soundtrack is bundled. You can choose your own local copy of “Lullaby of the New Moon (I): Somnias a Luna” or another track.'}</p>
    {music.error && <p className="music-error" role="alert">{music.error}</p>}
    {music.storageWarning && <p className="music-warning" role="status">{music.storageWarning}</p>}
  </section>;
}
