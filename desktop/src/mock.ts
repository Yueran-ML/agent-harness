import type { EngineBridge } from './bridge';
import { createReplayService } from './replay.shared.js';
import type { RpcMessage } from './protocol';
import type { EngineAnswer, EngineEvent, Media, Project, ProxyEvent, TransportState } from './studio/engine';
import type { Transport } from './transport';

export function createMockTransport(options: { cwd?: string; delayMs?: number } = {}): Transport {
  const listeners = new Set<(message: RpcMessage) => void>();
  const closeListeners = new Set<(reason: string) => void>();
  let closed = false;
  const service = createReplayService((message: RpcMessage) => {
    queueMicrotask(() => { if (!closed) for (const listener of listeners) listener(message); });
  }, options);
  return {
    send(message) {
      if (closed) throw new Error('The replay service is closed.');
      void service.receive(message);
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    onClose(listener) { closeListeners.add(listener); return () => closeListeners.delete(listener); },
    close() {
      if (closed) return;
      service.close(); closed = true;
      for (const listener of closeListeners) listener('The replay service is closed.');
      listeners.clear(); closeListeners.clear();
    },
  };
}

const digest = (seed: string) => seed.repeat(64).slice(0, 64);
const identity = (seed: string, size: number) => ({ size, modified_us: 1_790_000_000_000_000, digest: digest(seed) });

/** The media of the replay: a camera's film, an interview, a phone's film held upright, and music. */
const replayMedia: Media[] = [
  {
    id: 'm1', path: 'C:\\media\\harbour-4k.mp4', identity: identity('a', 418_000_000),
    facts: {
      duration: { ticks: 375 * 1001, scale: 30000 },
      video: { width: 3840, height: 2160, rate: { num: 30000, den: 1001 }, codec: 'hevc', bits: 10, rotation: 0 },
      audio: { rate: 48000, channels: 2, codec: 'aac' },
    },
  },
  {
    id: 'm2', path: 'C:\\media\\interview.mov', identity: identity('b', 2_100_000_000),
    facts: {
      duration: { ticks: 95 * 25 * 512, scale: 12800 },
      video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'prores', bits: 10, rotation: 0 },
      audio: { rate: 48000, channels: 2, codec: 'pcm_s24le' },
    },
  },
  {
    id: 'm3', path: 'C:\\media\\phone\\IMG_0412.MOV', identity: identity('c', 96_000_000),
    facts: {
      duration: { ticks: 8 * 60000, scale: 60000 },
      video: { width: 1920, height: 1080, rate: { num: 60000, den: 1001 }, codec: 'hevc', bits: 8, rotation: 90 },
      audio: { rate: 44100, channels: 2, codec: 'aac' },
    },
  },
  {
    id: 'm4', path: 'C:\\media\\theme.wav', identity: identity('d', 11_000_000),
    facts: { duration: { ticks: 63 * 48000, scale: 48000 }, video: null, audio: { rate: 48000, channels: 2, codec: 'pcm_s16le' } },
  },
];

/**
 * An engine of the studio for the replay in a browser, which has no shell and no engine: a
 * project of a few media, a transport that says where it is and plays nothing, and proxies that
 * are being made or failed. The view is nobody's: nothing is drawn.
 */
export function createMockEngine(): EngineBridge {
  const listeners = new Set<(event: EngineEvent) => void>();
  const project: Project = { schema: 1, next_id: 5, media: replayMedia, timelines: [] };
  const session = 1;
  const lastOf = (media: Media) => {
    const rate = media.facts.video?.rate ?? { num: 25, den: 1 };
    const { ticks, scale } = media.facts.duration;
    return Math.max(0, Math.round((ticks / scale) * (rate.num / rate.den)) - 1);
  };
  let state: TransportState = {
    media: 'm1', frame: 37, playing: false, speed: 1, exact: true, shown: 37, last: lastOf(replayMedia[0]),
    sound: 'off', source: 'original', proxies: true,
  };
  const tell = (event: EngineEvent) => queueMicrotask(() => { for (const listener of [...listeners]) listener(event); });
  const notify = (method: string, params: unknown) => tell({ kind: 'notification', session, method, params });
  const proxy = (told: ProxyEvent) => notify('media.proxy', told);
  const loaded = () => replayMedia.find(media => media.id === state.media) ?? null;
  const change = (next: Partial<TransportState>) => {
    state = { ...state, ...next };
    const media = loaded();
    const heard = media?.facts.audio ? (state.playing && state.speed === 1 ? 'on' : 'off') : 'none';
    state = { ...state, sound: heard, shown: media?.facts.video ? state.frame : null };
    notify('transport.state', state);
  };
  const ok = (result: unknown): EngineAnswer => ({ ok: true, result });
  const clamp = (frame: number) => Math.max(0, Math.min(state.last ?? 0, Math.round(frame)));

  return {
    async open(cwd) {
      const opened = { session, project: `${cwd}\\.columbina\\studio`, owned: true, pid: 4242, revision: 0 };
      tell({ kind: 'opened', ...opened });
      proxy({ media: 'm1', state: 'running', progress: 0.4 });
      proxy({ media: 'm3', state: 'failed', message: 'ffmpeg wrote no frame for 60 s' });
      return opened;
    },
    async call(method, params = {}) {
      switch (method) {
        case 'project.get':
          return ok({ revision: 0, next_id: project.next_id, project });
        case 'transport.state':
          return ok(state);
        case 'transport.load': {
          const media = replayMedia.find(each => each.id === params.media);
          if (!media) return { ok: false, error: { code: -32602, message: 'there is no such media', data: { reason: 'missing' } } };
          change({ media: media.id, frame: 0, playing: false, speed: 1, exact: true, last: lastOf(media), source: 'original' });
          return ok(state);
        }
        case 'transport.play':
          change({ playing: true, speed: typeof params.speed === 'number' ? params.speed : 1 });
          return ok({});
        case 'transport.pause':
          change({ playing: false });
          return ok({});
        case 'transport.seek':
          change({ frame: clamp(Number(params.frame)), exact: params.scrubbing !== true, playing: params.scrubbing === true ? false : state.playing });
          return ok({});
        case 'transport.step':
          change({ frame: clamp((state.frame ?? 0) + Number(params.frames)), playing: false, exact: true });
          return ok({});
        case 'transport.proxies':
          change({ proxies: params.on === true });
          return ok({});
        case 'media.make_proxy':
          proxy({ media: String(params.media), state: 'queued' });
          return ok({});
        default:
          return { ok: false, error: { code: -32601, message: `the replay has no ${method}`, data: null } };
      }
    },
    async importFiles() { return ok(null) as EngineAnswer<null>; },
    async viewRect() {},
    async viewShow() {},
    async close() {},
    onEvent(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}
