/**
 * What the tests that mount a part of the page share: the few things that a browser has and
 * the DOM of the tests does not, a workspace with whatever a test says of it, and logs that
 * are written as lists.
 *
 * A test file that mounts something begins with `// @vitest-environment jsdom`.
 */
import { vi } from 'vitest';
import type { SessionEvent, SessionStatus } from '../src/protocol';
import { emptyPreview, projectEvents } from '../src/state';
import type { GitHub } from '../src/useGitHub';
import type { Models, ServiceModels } from '../src/useModels';
import type { Workspace } from '../src/useWorkspace';

/** Give the DOM of the tests what the parts of the page ask a browser for, each as little as they need. */
export function likeABrowser(): void {
  class Observer {
    observe() {}

    unobserve() {}

    disconnect() {}
  }
  vi.stubGlobal('ResizeObserver', Observer);
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: false, media: query, addEventListener() {}, removeEventListener() {},
  }));
  Element.prototype.scrollIntoView = function scrollIntoView() {};
  Element.prototype.scrollTo = function scrollTo() {} as typeof Element.prototype.scrollTo;
  // A sheet is a dialog that is shown above everything else. Here it is enough that it is open.
  HTMLDialogElement.prototype.showModal = function showModal(this: HTMLDialogElement) { this.setAttribute('open', ''); };
  HTMLDialogElement.prototype.close = function close(this: HTMLDialogElement) { this.removeAttribute('open'); };
}

/** A log, written as a list of events. Each stands where it is listed, a tenth of a second after the one before. */
export type Written = [type: string, data?: Record<string, unknown>];
export const log = (...written: Written[]): SessionEvent[] =>
  written.map(([type, data = {}], seq) => ({ seq, type, data, time: 1000 + seq * 100 }));

const text = (words: string) => [{ type: 'text', text: words }];
export const begin = (turn: number): Written => ['turn/start', { turn }];
export const end = (turn: number, reason = 'completed', detail: string | null = null): Written =>
  ['turn/end', { turn, reason, detail }];
export const user = (words: string, source = 'user'): Written =>
  ['user/message', { source, message: { role: 'user', content: text(words) } }];
export const reply = (words: string, thought = ''): Written =>
  ['assistant/message', {
    message: { role: 'assistant', content: [...(thought ? [{ type: 'reasoning', text: thought }] : []), ...text(words)] },
    finish_reason: 'stop',
  }];
export const header = (provider: string): Written =>
  ['request/header', { config: { provider, model: 'a-model' }, tools: [] }];
export const failed = (code: string, message: string, status: number | null): Written =>
  ['assistant/attempt', { message: { role: 'assistant', content: [] }, failure: { code, message, status } }];
export const call = (id: string, name: string, args: Record<string, unknown>): Written =>
  ['tool/call', { call_id: id, name, arguments: JSON.stringify(args) }];
export const result = (id: string, words: string): Written =>
  ['tool/result', { message: { role: 'tool', call_id: id, content: text(words) }, error: null }];

/** What the service process says of a session, at rest unless a test says otherwise. */
export const statusOf = (given: Partial<SessionStatus> = {}): SessionStatus => ({
  session_id: 's-1',
  status: 'idle',
  detail: null,
  model: { provider: 'deepseek', model: 'deepseek-flash', context_window: 1_048_576 },
  sandbox: { mode: 'workspace-write', strength: 'none' },
  approval: 'ask',
  ...given,
});

/**
 * A workspace as the page holds one: a project is open, its service answers, a task is shown.
 *
 * Args:
 *   given: Whatever a test says of it. `events` is the log of the task: what the page works
 *     out of a log is worked out of that, unless the test gives it too.
 */
export function workspaceWith(given: Partial<Workspace> = {}): Workspace {
  const events = given.events ?? [];
  const closed = given.closed ?? null;
  const whole = {
    hello: { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work', commands: [] },
    sessionId: 's-1',
    sessions: [],
    events,
    status: statusOf(),
    preview: emptyPreview(),
    approval: null,
    outputs: [],
    connected: true,
    loading: false,
    hasProject: true,
    sending: false,
    cancelling: false,
    error: null,
    closed,
    lastWords: [],
    openedAt: events.length,
    folder: 'F:\\work',
    recent: [],
    services: null,
    checkingServices: false,
    skills: [],
    mode: 'service' as const,
    projection: projectEvents(events, closed !== null),
    reading: null,
    openSession: vi.fn(async () => {}),
    send: vi.fn(async () => true),
    prompt: vi.fn(async () => true),
    cancel: vi.fn(async () => {}),
    answer: vi.fn(),
    command: vi.fn(async () => {}),
    pickFolder: vi.fn(async () => null),
    pickRecent: vi.fn(async () => null),
    openFolder: vi.fn(async () => {}),
    setUpService: vi.fn(async () => null),
    refreshServices: vi.fn(async () => null),
    cancelStart: vi.fn(),
    reconnect: vi.fn(async () => {}),
    showLog: vi.fn(),
    clearError: vi.fn(),
    fail: vi.fn(),
  };
  return { ...whole, ...given } as unknown as Workspace;
}

/** A service as `columbina setup --list` tells of it. */
export const service = (
  name: string, title: string, given: { own?: boolean; key?: string; key_env?: string | null; key_page?: string | null } = {},
) => ({
  name,
  title,
  base_url: given.own ? 'http://10.0.0.5:8000/v1' : `https://api.${name}.example/v1`,
  own: given.own ?? false,
  model: 'a-model',
  context_window: 32_768,
  key_env: given.key_env === undefined ? 'A_KEY' : given.key_env,
  key: given.key ?? 'none',
  key_page: given.key_page ?? null,
});

/** What the page knows of the models of the services: nothing, unless a test says what of which. */
export function modelsWith(known: Record<string, Models> = {}): ServiceModels {
  return {
    of: (service: string) => known[service] ?? null,
    ask: vi.fn(async () => {}),
    freshen: vi.fn(),
  };
}

/** Where the repositories that the GitHub App of the tests may reach are chosen. */
export const INSTALL_URL = 'https://github.com/apps/columbina/installations/new';

/**
 * What the page knows of GitHub, as it holds it: nobody is signed in, and no sign-in runs,
 * unless a test says otherwise.
 */
export function gitHubWith(given: Partial<GitHub> = {}): GitHub {
  return {
    account: { kind: 'nobody', installUrl: INSTALL_URL, ended: null },
    skipped: false,
    checking: false,
    signIn: { kind: 'idle' },
    signedOut: null,
    signOutFailed: null,
    refresh: vi.fn(async () => {}),
    begin: vi.fn(async () => {}),
    cancel: vi.fn(),
    signOut: vi.fn(async () => true),
    skip: vi.fn(async () => {}),
    ...given,
  };
}
