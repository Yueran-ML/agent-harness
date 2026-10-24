import type { AssistantHandle } from '../src/useAssistant';

/**
 * An assistant in Feishu or in WeCom where a test is about something else: there is no service
 * program to ask, as in a replay. The settings are tried with them in page-assistant.test.tsx.
 */
export const noAssistant: AssistantHandle = {
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
  setThinking: async () => {},
  openFolder: async () => {},
};
