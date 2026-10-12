// @vitest-environment jsdom
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Conversation } from '../src/app/Conversation';
import type { Workspace } from '../src/useWorkspace';
import {
  begin, call, end, failed, header, likeABrowser, log, reply, result, service, statusOf, user, workspaceWith,
} from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** Draw the conversation of a workspace, and return what a button under the end of a turn is given to. */
function drawn(workspace: Workspace) {
  const onAction = vi.fn();
  render(<Conversation workspace={workspace} onLink={() => {}} onAction={onAction} />);
  return onAction;
}
const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const buttonsOf = (part: HTMLElement) => [...part.querySelectorAll('button')].map(button => button.textContent);
const press = (part: HTMLElement, words: string) => {
  const button = [...part.querySelectorAll('button')].find(one => one.textContent === words);
  if (!button) throw new Error(`There is no button that says "${words}".`);
  fireEvent.click(button);
};

describe('a turn that failed for its key', () => {
  // The service of the profile has no key: the request fails before it is made, with no status.
  const noKey = log(
    begin(1), user('What is the mean of 1, 2 and 3?'), header('deepseek'),
    failed('AUTH', 'there is no API key for deepseek', null),
    end(1, 'error', 'AUTH: there is no API key for deepseek'),
  );
  const services = (use: string | null, key = 'none') => ({
    use, services: [service('deepseek', 'DeepSeek'), service('my-own', 'http://10.0.0.5:8000/v1', { own: true, key })],
  });
  const talkingTo = (provider: string) => statusOf({ model: { provider, model: 'a-model', context_window: 32_768 } });

  it('names the service that the request went to, and leads to the settings', () => {
    const onAction = drawn(workspaceWith({ events: noKey, services: services(null) }));
    const [ending] = marked('ending');
    expect(ending.querySelector('.ending-what')?.textContent).toBe('No API key is set up for DeepSeek.');
    // It is something that went wrong, and is announced as that.
    expect(ending.getAttribute('role')).toBe('alert');
    expect(ending.dataset.tone).toBe('bad');
    // The key is known to be missing: trying again would fail the same way.
    expect(buttonsOf(ending)).toEqual(['Open model settings']);
    press(ending, 'Open model settings');
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onAction.mock.calls[0][0]).toBe('openModelSettings');
  });

  it('keeps the name of that service once another is in use, and can be tried again then', () => {
    const onAction = drawn(workspaceWith({ events: noKey, services: services('my-own'), status: talkingTo('my-own') }));
    const [ending] = marked('ending');
    // The failure was of DeepSeek, whatever is in use by now.
    expect(ending.querySelector('.ending-what')?.textContent).toBe('No API key is set up for DeepSeek.');
    expect(buttonsOf(ending)).toEqual(['Open model settings', 'Try again']);
    press(ending, 'Try again');
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onAction.mock.calls[0][0]).toBe('retry');
    // The turn goes along, so that its task can be given again.
    expect(onAction.mock.calls[0][1]).toMatchObject({ kind: 'turn', number: 1, provider: 'deepseek' });
  });

  it('can be tried again as soon as a key is there for the service that failed', () => {
    drawn(workspaceWith({
      events: noKey,
      services: { use: 'deepseek', services: [service('deepseek', 'DeepSeek', { key: 'stored' })] },
    }));
    expect(buttonsOf(marked('ending')[0])).toEqual(['Open model settings', 'Try again']);
  });

  it('names a service of the user by the name that the user gave it, where that one turned a request down', () => {
    const refused = log(
      begin(1), user('ask'), header('my-own'),
      failed('AUTH', 'Incorrect API key provided.', 401), end(1, 'error', 'AUTH: Incorrect API key provided.'),
    );
    drawn(workspaceWith({ events: refused, services: services('my-own'), status: talkingTo('my-own') }));
    const [ending] = marked('ending');
    expect(ending.querySelector('.ending-what')?.textContent).toBe('my-own did not accept the API key.');
    expect(ending.querySelector('.ending-why')?.textContent).toContain('mistyped');
  });

  it('says it without a name where the log names no service', () => {
    const unnamed = log(
      begin(1), user('ask'), failed('AUTH', 'no key', null), end(1, 'error', 'AUTH: no key'),
    );
    drawn(workspaceWith({ events: unnamed, services: services(null) }));
    expect(marked('ending')[0].querySelector('.ending-what')?.textContent).toBe('No API key is set up for the model service.');
  });

  it('offers nothing while the agent is at work, and nothing once the service process is gone', () => {
    drawn(workspaceWith({ events: noKey, services: services('my-own'), status: statusOf({ status: 'running' }) }));
    expect(buttonsOf(marked('ending')[0])).toEqual([]);
    cleanup();
    drawn(workspaceWith({ events: noKey, services: services('my-own'), connected: false }));
    expect(buttonsOf(marked('ending')[0])).toEqual([]);
  });

  it('offers nothing under a turn that another turn came after', () => {
    drawn(workspaceWith({
      events: log(
        begin(1), user('first'), header('deepseek'), failed('AUTH', 'no key', null), end(1, 'error', 'AUTH: no key'),
        begin(2), user('second'), reply('Done.'), end(2),
      ),
      services: services('my-own'),
    }));
    const [ending] = marked('ending');
    expect(ending.querySelector('.ending-what')?.textContent).toBe('No API key is set up for DeepSeek.');
    expect(buttonsOf(ending)).toEqual([]);
  });
});

describe('what a command answered', () => {
  const two = log(
    begin(1), user('first'), reply('one'), end(1),
    begin(2), user('second'), reply('two'), end(2),
  );
  const order = () => [...document.querySelector('[data-test="talk"]')!.children].map((part) => {
    const kind = (part as HTMLElement).dataset.test;
    if (kind === 'turn') return `turn: ${part.querySelector('[data-test="said"]')?.textContent}`;
    if (kind === 'command-said') return `answer: ${part.querySelector('code')?.textContent}`;
    return part.tagName;
  });

  it('stands where it was asked, between the turns', () => {
    drawn(workspaceWith({
      events: two,
      outputs: [
        { id: 1, line: '/model', text: 'The model is a-model.', after: 3 },
        { id: 2, line: '/help', text: 'The commands.', after: -1 },
        { id: 3, line: '/todos', text: 'Nothing to do.', after: 7 },
      ],
    }));
    expect(order()).toEqual(['answer: /help', 'turn: first', 'answer: /model', 'turn: second', 'answer: /todos']);
  });

  it('is drawn as text, whatever it holds', () => {
    drawn(workspaceWith({ events: two, outputs: [{ id: 1, line: '/help', text: '<b>bold</b> & **stars**', after: 7 }] }));
    const [answer] = marked('command-said');
    expect(answer.querySelector('pre')?.textContent).toBe('<b>bold</b> & **stars**');
    expect(answer.querySelector('b')).toBeNull();
  });
});

describe('the way to the answer of a turn', () => {
  it('is put away under one line, and holds what the model thought before it answered', () => {
    drawn(workspaceWith({ events: log(begin(1), user('ask'), reply('Done.', 'first this, then that'), end(1)) }));
    const [toggle] = marked('steps-toggle');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(marked('last-thought')).toEqual([]);
    // The answer is the words of the model, and not what it thought.
    expect(marked('answer')[0].textContent).toContain('Done.');
    expect(marked('answer')[0].textContent).not.toContain('first this');

    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    const [thought] = marked('last-thought');
    expect(thought.querySelector('summary')?.textContent).toBe('Thought');
    expect(thought.querySelector('.thought')?.textContent).toBe('first this, then that');

    // The user put it away again, and it stays away.
    fireEvent.click(toggle);
    expect(marked('last-thought')).toEqual([]);
  });

  it('is no line at all where the model answered at once', () => {
    drawn(workspaceWith({ events: log(begin(1), user('ask'), reply('Done.'), end(1)) }));
    expect(marked('steps-toggle')).toEqual([]);
    expect(marked('answer')[0].textContent).toContain('Done.');
  });

  it('is shown in full while the turn has no answer', () => {
    drawn(workspaceWith({
      events: log(begin(1), user('ask'), call('a', 'read', { path: 'calc.py' }), result('a', 'def mean(values): ...')),
      status: statusOf({ status: 'running' }),
    }));
    expect(marked('steps-toggle')).toEqual([]);
    expect(marked('tool')).toHaveLength(1);
  });
});

describe('the line of a call', () => {
  // The events of a log are a tenth of a second apart, from one second on: the call below is made at 1.2 s.
  const waiting = log(begin(1), user('wait'), call('a', 'bash', { command: 'sleep 30' }));
  const atWork = statusOf({ status: 'running' });

  it('says how long the call has run, in whole seconds that move as the time does', () => {
    vi.useFakeTimers();
    vi.setSystemTime(5000);
    drawn(workspaceWith({ events: waiting, status: atWork }));
    expect(marked('tool-time').map(time => time.textContent)).toEqual(['3 s']);
    act(() => { vi.advanceTimersByTime(1500); });
    expect(marked('tool-time').map(time => time.textContent)).toEqual(['5 s']);
  });

  it('says no time in the first second, which is over before it could be read', () => {
    vi.useFakeTimers();
    vi.setSystemTime(1900);
    drawn(workspaceWith({ events: waiting, status: atWork }));
    expect(marked('tool-time')).toEqual([]);
    act(() => { vi.advanceTimersByTime(600); });
    expect(marked('tool-time').map(time => time.textContent)).toEqual(['1 s']);
  });

  it('says what the call took once it is through, and stops counting', () => {
    vi.useFakeTimers();
    vi.setSystemTime(60_000);
    const through = log(begin(1), user('wait'), call('a', 'bash', { command: 'sleep 2' }), ...Array.from({ length: 20 }, () => ['step/tick'] as [string]), result('a', 'ok'));
    drawn(workspaceWith({ events: through, status: atWork }));
    // Twenty-one events of a tenth of a second lie between the call and its result.
    expect(marked('tool-time').map(time => time.textContent)).toEqual(['2.1 s']);
    act(() => { vi.advanceTimersByTime(5000); });
    expect(marked('tool-time').map(time => time.textContent)).toEqual(['2.1 s']);
  });

  it('names the kind of subagent that a task was handed to', () => {
    drawn(workspaceWith({
      events: log(
        begin(1), user('review it'),
        call('a', 'subagent', { description: 'Review calc.py', prompt: 'Read calc.py.', type: 'reviewer' }),
        call('b', 'subagent', { description: 'Look around', prompt: 'Look.' }),
      ),
      status: atWork,
    }));
    const [named, plain] = marked('tool');
    expect(named.querySelector('.tool-object')?.textContent).toBe('Review calc.py');
    expect(named.querySelector('[data-test="tool-kind"]')?.textContent).toBe('reviewer');
    expect(named.querySelector('[data-test="tool-kind"]')?.getAttribute('title')).toBe('Kind of subagent');
    // A task that names no kind goes to a subagent like the agent itself, and says nothing of a kind.
    expect(plain.querySelector('[data-test="tool-kind"]')).toBeNull();
  });

  it('shows the last lines of what came back, and all of them when that is asked for', () => {
    const lines = Array.from({ length: 300 }, (_line, index) => `line ${index}`).join('\n');
    drawn(workspaceWith({ events: log(begin(1), user('count'), call('a', 'bash', { command: 'count' }), result('a', lines), reply('Counted.'), end(1)) }));
    fireEvent.click(marked('steps-toggle')[0]);
    const shown = () => marked('tool-output')[0].textContent!.split('\n');
    expect(marked('tool-all').map(button => button.textContent)).toEqual(['Show all 300 lines']);
    expect(shown()).toHaveLength(81);
    expect(shown().slice(0, 2)).toEqual(['…', 'line 220']);
    expect(shown().at(-1)).toBe('line 299');

    fireEvent.click(marked('tool-all')[0]);
    expect(shown()).toHaveLength(300);
    expect(shown()[0]).toBe('line 0');
    expect(marked('tool-all')).toEqual([]);
  });

  it('offers nothing more where everything that came back is shown', () => {
    drawn(workspaceWith({ events: log(begin(1), user('read'), call('a', 'read', { path: 'calc.py' }), result('a', 'one\ntwo'), reply('Read.'), end(1)) }));
    fireEvent.click(marked('steps-toggle')[0]);
    expect(marked('tool-all')).toEqual([]);
    expect(marked('tool-output')[0].textContent).toBe('one\ntwo');
  });
});
