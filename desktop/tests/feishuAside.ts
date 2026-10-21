import type { Feishu } from '../src/useFeishu';

/**
 * The assistant in Feishu where a test is about something else: there is no service program to
 * ask, as in a replay. The settings are tried with it in page-feishu.test.tsx.
 */
export const noFeishu: Feishu = {
  assistant: { kind: 'unavailable' },
  busy: null,
  failure: null,
  forgotten: false,
  refresh: async () => {},
  start: async () => {},
  stop: async () => {},
  forget: async () => false,
  setUp: async () => {},
  showLog: () => {},
  setWritable: async () => {},
  openFolder: async () => {},
};
