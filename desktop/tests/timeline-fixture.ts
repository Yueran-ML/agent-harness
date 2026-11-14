import { emptyGrade, type Clip, type Media, type Timeline } from '../src/studio/engine';

export const clip = (id: string, start = 0, length = 100, fields: Partial<Clip> = {}): Clip => ({
  id, start, length, source: { kind: 'media', media: 'm1', in: { ticks: 0, scale: 25 } }, enabled: true,
  speed: { num: 1, den: 1 }, reverse: false, link: null,
  transform: { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 }, gain_db: 0, pan: 0, fade_in: 0, fade_out: 0,
  grade: emptyGrade(), group: null, ...fields,
});
export const media: Media = {
  id: 'm1', name: 'Interview', bin: null, mark_in: null, mark_out: null, path: 'C:/media/interview.mov',
  identity: { size: 1000, modified_us: 0, digest: '0'.repeat(64) },
  facts: { duration: { ticks: 100000, scale: 25 }, video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, bits: 10, codec: 'prores', rotation: 0 }, audio: { rate: 48000, channels: 2, codec: 'pcm' } },
};
export const timeline = (clips = [clip('c1'), clip('c2', 100)]): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips, transitions: [] }],
  grade: emptyGrade(),
});
