// @vitest-environment jsdom
import { useState } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApprovalCard } from '../src/app/ApprovalCard';
import { Composer } from '../src/app/Composer';
import { projectEvents } from '../src/state';
import type { PendingApproval, Workspace } from '../src/useWorkspace';
import { likeABrowser, modelsWith, statusOf, workspaceWith } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  for (const left of document.querySelectorAll('dialog, textarea')) left.remove();
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const off = (name: string) => (one(name) as HTMLButtonElement).disabled;
const atWork = statusOf({ status: 'running' });

describe('a question of the agent', () => {
  const asked = (given: Partial<PendingApproval> = {}): PendingApproval => ({
    requestId: 7, session_id: 's-1', call_id: 'c-1', tool: 'bash', summary: 'Run a command.',
    reason: 'The command is not among those that run without a question.',
    target: 'npm test', standing: 'this command, word for word', ...given,
  });
  const card = (approval = asked(), cancelling = false) => {
    const calls = { onAnswer: vi.fn(), onStop: vi.fn() };
    const drawn = render(<ApprovalCard approval={approval} cancelling={cancelling} {...calls} />);
    return { calls, ...drawn };
  };
  /** Let the first moment of a card pass, in which nothing answers it. */
  const settle = () => act(() => { vi.advanceTimersByTime(500); });
  /** Press Enter with other keys held down, anywhere on the page. Whether the page took the key. */
  const enter = (held: KeyboardEventInit = { ctrlKey: true }) => !fireEvent.keyDown(document.body, { key: 'Enter', ...held });
  const waits = 'Columbina waits for your answer.';
  const sent = 'Your answer is on its way.';
  const said = () => one('consent').querySelector('[role="status"]')?.textContent;
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-12T12:00:00Z'));
  });

  it('says what is asked for, why, and for what a longer yes would hold', () => {
    card();
    const consent = one('consent');
    expect(consent.querySelector('h2')?.textContent).toBe('Your consent is needed');
    expect(consent.getAttribute('aria-labelledby')).toBe(consent.querySelector('h2')?.id);
    expect(said()).toBe(waits);
    expect(consent.querySelector('.consent-what')?.textContent).toBe('Columbina wants to run this command.');
    // Said once to whoever cannot see the card appear.
    expect(consent.querySelector('[role="alert"]')?.textContent).toBe('Your consent is needed Columbina wants to run this command.');
    expect(one('consent-target').textContent).toBe('npm test');
    // What is asked about can be longer than its box: the keyboard reaches it to scroll it.
    expect(one('consent-target').tabIndex).toBe(0);
    expect(consent.querySelector('details summary')?.textContent).toBe('Why it asks');
    expect(consent.querySelector('details .consent-why')?.textContent).toBe('The command is not among those that run without a question.');
    expect([...consent.querySelectorAll('button')].map(each => each.textContent))
      .toEqual(['Stop the turn', 'Do not allow', 'Allow, and do not ask again', 'Allow this onceCtrlEnter']);
    const scope = 'Not asking again holds for this command, word for word, in this task, until the project is closed.';
    expect(consent.querySelector('.consent-scope')?.textContent).toBe(scope);
    expect(document.getElementById(one('consent-always').getAttribute('aria-describedby')!)?.textContent).toBe(scope);
  });

  it('says what is wanted by the kind of tool that asks', () => {
    const wanted = (tool: string, target: string | null = 'notes.md') => {
      cleanup();
      card(asked({ tool, target, standing: null }));
      return [one('consent').querySelector('.consent-what')?.textContent, one('consent-target').textContent];
    };
    expect(wanted('bash', 'npm test')).toEqual(['Columbina wants to run this command.', 'npm test']);
    expect(wanted('powershell', 'npm test')).toEqual(['Columbina wants to run this command.', 'npm test']);
    expect(wanted('write')).toEqual(['Columbina wants to change this file.', 'notes.md']);
    expect(wanted('edit')).toEqual(['Columbina wants to change this file.', 'notes.md']);
    expect(wanted('fetch', 'https://example.org')).toEqual(['Columbina wants to use fetch.', 'https://example.org']);
    // A call that is about no one thing is shown as the service sums it up.
    expect(wanted('fetch', null)).toEqual(['Columbina wants to use fetch.', 'Run a command.']);
    // What is to be done on GitHub is said as such, with all of what is to be done below it.
    const comment = 'octocat/hello#12: comment\n\nLooks good.';
    expect(wanted('github_comment', comment)).toEqual(['Columbina wants to write a comment on GitHub.', comment]);
    expect(wanted('github_push', 'push fix-parser to octocat/hello')[0]).toBe('Columbina wants to push a branch to GitHub.');
  });

  it('offers no longer yes where the service offers none, and no reason where it gave none', () => {
    card(asked({ standing: null, reason: '' }));
    expect(marked('consent-always')).toEqual([]);
    expect(one('consent').querySelector('.consent-scope')).toBeNull();
    expect(one('consent').querySelector('details')).toBeNull();
    expect([...one('consent').querySelectorAll('button')].map(each => each.textContent))
      .toEqual(['Stop the turn', 'Do not allow', 'Allow this onceCtrlEnter']);
  });

  it('says for what a longer yes holds, as the service says it', () => {
    const scope = (given: Partial<PendingApproval>) => {
      cleanup();
      card(asked(given));
      return one('consent').querySelector('.consent-scope')?.textContent;
    };
    expect(scope({ tool: 'write', target: 'notes.md', standing: 'write on notes.md' }))
      .toBe('Not asking again holds for write on notes.md in this task, until the project is closed.');
    expect(scope({ tool: 'fetch', target: null, standing: 'any call of fetch' }))
      .toBe('Not asking again holds for every call of fetch in this task, until the project is closed.');
    expect(scope({ standing: 'something that is said in a new way' }))
      .toBe('Not asking again holds for the same call in this task, until the project is closed.');
  });

  it('is answered by nothing in its first moment, neither by a click nor by a key', () => {
    const { calls } = card();
    expect(['consent-reject', 'consent-always', 'consent-once'].map(off)).toEqual([true, true, true]);
    for (const name of ['consent-reject', 'consent-always', 'consent-once']) fireEvent.click(one(name));
    // The key is the card's all the same: it does not go on to send what is written in the field.
    expect(enter()).toBe(true);
    act(() => { vi.advanceTimersByTime(499); });
    expect(['consent-reject', 'consent-always', 'consent-once'].map(off)).toEqual([true, true, true]);
    expect(enter()).toBe(true);
    expect(calls.onAnswer).not.toHaveBeenCalled();
    expect(said()).toBe(waits);
    act(() => { vi.advanceTimersByTime(1); });
    expect(['consent-reject', 'consent-always', 'consent-once'].map(off)).toEqual([false, false, false]);
    expect(enter()).toBe(true);
    expect(calls.onAnswer.mock.calls).toEqual([['allowed-once', false]]);
  });

  it('is answered once, whatever is pressed after the answer', () => {
    const { calls } = card();
    settle();
    fireEvent.click(one('consent-once'));
    expect(calls.onAnswer.mock.calls).toEqual([['allowed-once', false]]);
    expect(said()).toBe(sent);
    expect(['consent-reject', 'consent-always', 'consent-once'].map(off)).toEqual([true, true, true]);
    enter();
    fireEvent.click(one('consent-reject'));
    expect(calls.onAnswer).toHaveBeenCalledTimes(1);
  });

  it('takes a no, and a yes that holds for longer, from their buttons', () => {
    const answered = (name: string) => {
      cleanup();
      const { calls } = card();
      settle();
      fireEvent.click(one(name));
      return calls.onAnswer.mock.calls;
    };
    expect(answered('consent-reject')).toEqual([['rejected', false]]);
    expect(answered('consent-always')).toEqual([['allowed-once', true]]);
    expect(answered('consent-once')).toEqual([['allowed-once', false]]);
  });

  it('takes Enter for a yes only with Ctrl or its like held down, and with nothing else', () => {
    const { calls } = card();
    settle();
    // Enter alone belongs to the field, and says yes to nothing.
    expect(enter({})).toBe(false);
    expect(enter({ ctrlKey: true, shiftKey: true })).toBe(false);
    expect(enter({ ctrlKey: true, altKey: true })).toBe(false);
    expect(enter({ ctrlKey: true, repeat: true })).toBe(false);
    expect(enter({ ctrlKey: true, isComposing: true })).toBe(false);
    expect(!fireEvent.keyDown(document.body, { key: 'y', ctrlKey: true })).toBe(false);
    expect(calls.onAnswer).not.toHaveBeenCalled();
    expect(enter({ metaKey: true })).toBe(true);
    expect(calls.onAnswer.mock.calls).toEqual([['allowed-once', false]]);
  });

  it('names the key that is held with Enter as the system does: ⌘ on macOS, Ctrl elsewhere', () => {
    const named = () => [...one('consent').querySelectorAll('kbd')].map(each => each.textContent);
    card();
    expect(named()).toEqual(['Ctrl', 'Enter']);
    for (const [platform, held] of [['darwin', '⌘'], ['win32', 'Ctrl'], ['linux', 'Ctrl']]) {
      cleanup();
      vi.stubGlobal('columbina', { platform });
      card();
      expect(named(), platform).toEqual([held, 'Enter']);
    }
  });

  it('leaves the key to a sheet that is open above it', () => {
    const { calls } = card();
    settle();
    const sheet = document.createElement('dialog');
    sheet.setAttribute('open', '');
    document.body.append(sheet);
    expect(enter()).toBe(false);
    expect(calls.onAnswer).not.toHaveBeenCalled();
    sheet.remove();
    expect(enter()).toBe(true);
    expect(calls.onAnswer).toHaveBeenCalledTimes(1);
  });

  it('stops the turn from the first moment on, and says when it is being stopped', () => {
    const { calls, rerender } = card();
    expect(one('consent-stop').textContent).toBe('Stop the turn');
    fireEvent.click(one('consent-stop'));
    expect(calls.onStop).toHaveBeenCalledTimes(1);
    expect(calls.onAnswer).not.toHaveBeenCalled();
    rerender(<ApprovalCard approval={asked()} cancelling {...calls} />);
    expect(one('consent-stop').textContent).toBe('Stopping');
    expect(off('consent-stop')).toBe(true);
  });

  it('does not take the keyboard from whoever is writing', () => {
    const field = document.createElement('textarea');
    document.body.append(field);
    field.focus();
    card();
    settle();
    expect(document.activeElement).toBe(field);
  });

  it('remembers nothing for the next question, which has a card of its own', () => {
    const calls = { onAnswer: vi.fn(), onStop: vi.fn() };
    const { rerender } = render(<ApprovalCard key={7} approval={asked()} cancelling={false} {...calls} />);
    settle();
    fireEvent.click(one('consent-once'));
    expect(said()).toBe(sent);
    rerender(<ApprovalCard key={8} approval={asked({ requestId: 8, target: 'npm run build' })} cancelling={false} {...calls} />);
    expect(one('consent-target').textContent).toBe('npm run build');
    expect(said()).toBe(waits);
    // The new card has a first moment of its own.
    expect(off('consent-once')).toBe(true);
    enter();
    expect(calls.onAnswer).toHaveBeenCalledTimes(1);
    settle();
    enter();
    expect(calls.onAnswer).toHaveBeenCalledTimes(2);
  });

  it('listens to no key once it is gone', () => {
    const { calls, unmount } = card();
    settle();
    unmount();
    expect(enter()).toBe(false);
    expect(calls.onAnswer).not.toHaveBeenCalled();
  });
});

describe('the field', () => {
  const commands = [
    { name: 'compact', summary: 'Sum up what was said.' },
    { name: 'model', summary: 'Say or change the model.' },
    { name: 'permissions', summary: 'Say or change what the task may do.' },
    { name: 'plan', summary: 'Plan before anything is changed.' },
  ];
  const able = { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work', commands };
  const keysIdle = 'Enter sends. Shift+Enter starts a new line. A slash opens the commands.';
  const keysRunning = 'What you send now is read at the next step. Esc, pressed twice, stops the turn.';
  const escAgain = 'Press Esc once more to stop the turn.';

  /** The field with what is written in it kept by whoever shows it, as the page does. */
  function Held({ workspace, first = '', sent, returned = false, declined = false, commandsAsked = 0 }: {
    workspace: Workspace;
    first?: string;
    sent: (text: string) => Promise<boolean>;
    returned?: boolean;
    declined?: boolean;
    commandsAsked?: number;
  }) {
    const [draft, setDraft] = useState(first);
    const [field] = useState(() => ({ current: null as HTMLTextAreaElement | null }));
    return <Composer
      workspace={workspace} draft={draft} onDraft={setDraft} onSend={sent} field={field}
      returned={returned} declined={declined} commandsAsked={commandsAsked}
      models={modelsWith()} onChooseModel={vi.fn()} onModelSettings={vi.fn()}
      attachments={[]} refused={[]} onRemoveAttachment={vi.fn()}
    />;
  }
  const taken = () => vi.fn(async (_text: string) => true);
  const box = () => one('composer') as HTMLTextAreaElement;
  const write = (text: string) => { fireEvent.change(box(), { target: { value: text } }); };
  /** Press a key in the field. Whether the field took the key, so that the browser does nothing with it. */
  const press = (key: string, held: KeyboardEventInit = {}) => !fireEvent.keyDown(box(), { key, ...held });
  const hint = () => one('composer-hint').textContent;
  const options = () => [...document.querySelectorAll<HTMLElement>('[data-test="commands"] [role="option"]')];
  /** The commands that the list offers, each with whether it is the chosen one. Null where there is no list. */
  const list = () => (marked('commands').length === 0 ? null
    : options().map(option => [option.querySelector('code')?.textContent, option.getAttribute('aria-selected')]));
  const commandsButton = () => document.querySelector<HTMLButtonElement>('button[aria-label="Commands"]')!;

  it('sends what is written, by Enter and by its button, and nothing that is made of blanks', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith()} sent={sent} />);
    expect(box().placeholder).toBe('Give Columbina something to do');
    expect(hint()).toBe(keysIdle);
    expect(off('send')).toBe(true);
    write('   ');
    expect(off('send')).toBe(true);
    // Enter is the field's, also where there is nothing to send: it starts no line.
    await act(async () => { expect(press('Enter')).toBe(true); });
    expect(sent).not.toHaveBeenCalled();
    write('Fix the parser.');
    expect(off('send')).toBe(false);
    await act(async () => { expect(press('Enter')).toBe(true); });
    expect(sent.mock.calls).toEqual([['Fix the parser.']]);
    await act(async () => { fireEvent.click(one('send')); });
    expect(sent.mock.calls).toEqual([['Fix the parser.'], ['Fix the parser.']]);
    expect(one('send').getAttribute('aria-label')).toBe('Send');
  });

  it('sends a line once, however often it is sent before it was taken', async () => {
    let take = (_taken: boolean) => {};
    const sent = vi.fn((_text: string) => new Promise<boolean>(resolve => { take = resolve; }));
    render(<Held workspace={workspaceWith()} first="Fix the parser." sent={sent} />);
    await act(async () => {
      press('Enter');
      press('Enter');
      fireEvent.click(one('send'));
    });
    expect(sent).toHaveBeenCalledTimes(1);
    await act(async () => { take(true); });
    await act(async () => { press('Enter'); });
    expect(sent).toHaveBeenCalledTimes(2);
  });

  it('leaves to the browser a key that is meant for something else', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith()} first="Fix the parser." sent={sent} />);
    await act(async () => {
      // Shift+Enter starts a new line, and Ctrl+Enter answers a question.
      expect(press('Enter', { shiftKey: true })).toBe(false);
      expect(press('Enter', { ctrlKey: true })).toBe(false);
      // An input method that is putting characters together ends its work with Enter.
      expect(press('Enter', { isComposing: true })).toBe(false);
      expect(press('Enter', { keyCode: 229 })).toBe(false);
      // A key that is held down is one press.
      expect(press('Enter', { repeat: true })).toBe(false);
      expect(press('Escape')).toBe(false);
      expect(press('a')).toBe(false);
    });
    expect(sent).not.toHaveBeenCalled();
  });

  it('opens the list of commands at a slash, and narrows it with what is written', () => {
    render(<Held workspace={workspaceWith({ hello: able })} sent={taken()} />);
    expect(list()).toBeNull();
    expect([box().getAttribute('aria-expanded'), box().getAttribute('aria-controls'), box().getAttribute('aria-activedescendant')])
      .toEqual(['false', null, null]);
    write('/');
    expect(list()).toEqual([['/compact', 'true'], ['/model', 'false'], ['/permissions', 'false'], ['/plan', 'false']]);
    expect([box().getAttribute('aria-expanded'), box().getAttribute('aria-controls'), box().getAttribute('aria-activedescendant')])
      .toEqual(['true', 'commands', 'command-compact']);
    expect(one('commands').id).toBe('commands');
    expect(one('commands').getAttribute('aria-label')).toBe('Commands');
    expect(one('commands').querySelector('.commands-note')?.textContent)
      .toBe('A command is carried out by Columbina itself. The model does not see it.');
    expect(document.querySelector('#command-model span')?.textContent).toBe('Say or change the model.');
    // The keyboard stays in the field, so no command of the list is a stop for Tab.
    expect(options().map(option => option.tabIndex)).toEqual([-1, -1, -1, -1]);
    write('/P');
    expect(list()).toEqual([['/permissions', 'true'], ['/plan', 'false']]);
    write('/pe');
    expect(list()).toEqual([['/permissions', 'true']]);
    // No list where no command begins so, where the name is over, or where the slash is not the beginning.
    write('/pex');
    expect(list()).toBeNull();
    write('/permissions ');
    expect(list()).toBeNull();
    write('say /p');
    expect(list()).toBeNull();
    write('/lan');
    expect(list()).toBeNull();
  });

  it('is where the list is worked from, and takes the chosen command by Enter or by Tab', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith({ hello: able })} sent={sent} />);
    const chosen = () => box().getAttribute('aria-activedescendant');
    write('/');
    expect(press('ArrowDown')).toBe(true);
    expect(chosen()).toBe('command-model');
    expect(list()).toEqual([['/compact', 'false'], ['/model', 'true'], ['/permissions', 'false'], ['/plan', 'false']]);
    // The list goes round at both of its ends, and a key that is held down goes on through it.
    press('ArrowUp');
    press('ArrowUp');
    expect(chosen()).toBe('command-plan');
    press('ArrowDown');
    expect(chosen()).toBe('command-compact');
    press('ArrowDown', { repeat: true });
    expect(chosen()).toBe('command-model');
    // Taking a command is one deed, however long the key is held down.
    expect(press('Enter', { repeat: true })).toBe(false);
    expect(box().value).toBe('/');
    await act(async () => { expect(press('Enter')).toBe(true); });
    expect(box().value).toBe('/model ');
    expect(document.activeElement).toBe(box());
    expect(list()).toBeNull();
    expect(sent).not.toHaveBeenCalled();
    // The line is sent by the next Enter.
    await act(async () => { press('Enter'); });
    expect(sent.mock.calls).toEqual([['/model ']]);
    write('/pl');
    expect(press('Tab')).toBe(true);
    expect(box().value).toBe('/plan ');
    // Tab is the browser's again once the list is closed.
    expect(press('Tab')).toBe(false);
  });

  it('begins at the first command again whenever what is written changes', () => {
    render(<Held workspace={workspaceWith({ hello: able })} sent={taken()} />);
    write('/');
    press('ArrowDown');
    press('ArrowDown');
    press('ArrowDown');
    expect(box().getAttribute('aria-activedescendant')).toBe('command-plan');
    write('/p');
    expect(list()).toEqual([['/permissions', 'true'], ['/plan', 'false']]);
  });

  it('closes the list by Esc for what is written, and opens it again when that changes', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith({ hello: able })} sent={sent} />);
    write('/p');
    expect(list()).toEqual([['/permissions', 'true'], ['/plan', 'false']]);
    expect(press('Escape')).toBe(true);
    expect(list()).toBeNull();
    expect(box().value).toBe('/p');
    // Enter sends the line as it stands now.
    await act(async () => { press('Enter'); });
    expect(sent.mock.calls).toEqual([['/p']]);
    write('/pe');
    expect(list()).toEqual([['/permissions', 'true']]);
    write('/p');
    expect(list()).toEqual([['/permissions', 'true'], ['/plan', 'false']]);
  });

  it('takes a command by a click, and marks the one that the pointer is on', () => {
    render(<Held workspace={workspaceWith({ hello: able })} sent={taken()} />);
    write('/');
    fireEvent.mouseEnter(options()[2]);
    expect(list()).toEqual([['/compact', 'false'], ['/model', 'false'], ['/permissions', 'true'], ['/plan', 'false']]);
    fireEvent.click(options()[1]);
    expect(box().value).toBe('/model ');
    expect(document.activeElement).toBe(box());
  });

  it('opens the list from its button and when it is asked for from outside', () => {
    const sent = taken();
    const workspace = workspaceWith({ hello: able });
    const { rerender } = render(<Held workspace={workspace} sent={sent} />);
    expect(commandsButton().getAttribute('title')).toBe('Commands');
    fireEvent.click(commandsButton());
    expect(box().value).toBe('/');
    expect(list()).toHaveLength(4);
    expect(document.activeElement).toBe(box());
    press('Escape');
    expect(list()).toBeNull();
    // Asked for again: the list that was closed is open, and what is written is as it was.
    box().blur();
    rerender(<Held workspace={workspace} sent={sent} commandsAsked={1} />);
    expect(list()).toHaveLength(4);
    expect(box().value).toBe('/');
    expect(document.activeElement).toBe(box());
    // The button opens a closed list too.
    press('Escape');
    fireEvent.click(commandsButton());
    expect(list()).toHaveLength(4);
    // Whatever else is written stays, and an empty field gets its slash.
    write('Fix the parser.');
    rerender(<Held workspace={workspace} sent={sent} commandsAsked={2} />);
    expect(box().value).toBe('Fix the parser.');
    expect(list()).toBeNull();
    write('  ');
    rerender(<Held workspace={workspace} sent={sent} commandsAsked={3} />);
    expect(box().value).toBe('/');
    expect(list()).toHaveLength(4);
  });

  it('writes no slash for a command while the agent is at work', () => {
    const sent = taken();
    const workspace = workspaceWith({ hello: able, status: atWork });
    const { rerender } = render(<Held workspace={workspace} sent={sent} />);
    expect(commandsButton().disabled).toBe(true);
    rerender(<Held workspace={workspace} sent={sent} commandsAsked={1} />);
    expect(box().value).toBe('');
    expect(document.activeElement).toBe(box());
  });

  it('stops the turn by its button, and by Esc pressed twice within a moment', () => {
    vi.useFakeTimers();
    const sent = taken();
    const workspace = workspaceWith({ status: atWork });
    const { rerender } = render(<Held workspace={workspace} sent={sent} />);
    expect(box().placeholder).toBe('Add something for its next step');
    expect(box().form?.dataset.running).toBe('true');
    expect(hint()).toBe(keysRunning);
    expect(one('stop').textContent).toBe('Stop');
    fireEvent.click(one('stop'));
    expect(workspace.cancel).toHaveBeenCalledTimes(1);

    expect(press('Escape')).toBe(true);
    expect(hint()).toBe(escAgain);
    expect(workspace.cancel).toHaveBeenCalledTimes(1);
    expect(press('Escape')).toBe(true);
    expect(workspace.cancel).toHaveBeenCalledTimes(2);
    expect(hint()).toBe(keysRunning);

    // The first Esc of two is forgotten after a moment.
    press('Escape');
    act(() => { vi.advanceTimersByTime(1999); });
    expect(hint()).toBe(escAgain);
    act(() => { vi.advanceTimersByTime(1); });
    expect(hint()).toBe(keysRunning);
    press('Escape');
    expect(hint()).toBe(escAgain);
    expect(workspace.cancel).toHaveBeenCalledTimes(2);

    // It is forgotten too when the turn is over: it is not the first of two for the next turn.
    rerender(<Held workspace={{ ...workspace, status: statusOf() }} sent={sent} />);
    expect(hint()).toBe(keysIdle);
    expect(marked('stop')).toEqual([]);
    expect(box().form?.dataset.running).toBeUndefined();
    expect(press('Escape')).toBe(false);
    rerender(<Held workspace={workspace} sent={sent} />);
    expect(hint()).toBe(keysRunning);
    press('Escape');
    expect(hint()).toBe(escAgain);
    expect(workspace.cancel).toHaveBeenCalledTimes(2);
  });

  it('says that the turn is being stopped, and takes no Esc for stopping it again', () => {
    const workspace = workspaceWith({ status: atWork, cancelling: true });
    render(<Held workspace={workspace} sent={taken()} />);
    expect(one('stop').textContent).toBe('Stopping');
    expect(off('stop')).toBe(true);
    expect(hint()).toBe('Stopping');
    expect(press('Escape')).toBe(false);
    expect(workspace.cancel).not.toHaveBeenCalled();
  });

  it('keeps a command until the agent is at rest, and sends anything else to be read at the next step', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith({ status: atWork, hello: able })} first="/model deepseek-pro" sent={sent} />);
    expect(off('send')).toBe(true);
    expect(hint()).toBe('A command waits until Columbina is at rest.');
    await act(async () => { press('Enter'); });
    expect(sent).not.toHaveBeenCalled();
    write('  /model deepseek-pro');
    expect(hint()).toBe('A command waits until Columbina is at rest.');
    write('Mind the tests.');
    expect(off('send')).toBe(false);
    expect(hint()).toBe(keysRunning);
    await act(async () => { press('Enter'); });
    expect(sent.mock.calls).toEqual([['Mind the tests.']]);
  });

  it('sends nothing while something is being sent', async () => {
    const sent = taken();
    render(<Held workspace={workspaceWith({ sending: true })} first="Fix the parser." sent={sent} />);
    expect(off('send')).toBe(true);
    await act(async () => { press('Enter'); });
    expect(sent).not.toHaveBeenCalled();
  });

  it('says in the field why nothing can be written there', () => {
    const told = (given: Partial<Workspace>) => {
      cleanup();
      render(<Held workspace={workspaceWith({ hello: able, ...given })} first="/" sent={taken()} />);
      expect(document.querySelector('label[for="composer"]')?.textContent).toBe(box().placeholder);
      return [box().placeholder, box().disabled, hint(), box().form?.dataset.off ?? null, list() !== null, commandsButton().disabled];
    };
    expect(told({})).toEqual(['Give Columbina something to do', false, keysIdle, null, true, false]);
    expect(told({ hasProject: false, hello: null, status: null, sessionId: null }))
      .toEqual(['Open a project first', true, '', 'true', false, true]);
    expect(told({ connected: false, closed: 'It ended with the code 1.' }))
      .toEqual(['The service process has ended.', true, '', 'true', false, true]);
    expect(told({ connected: false, closed: '', hello: null }))
      .toEqual(['The service process could not be started.', true, '', 'true', false, true]);
    expect(told({ status: statusOf({ status: 'stopped' }) })).toEqual(['This task has stopped', true, '', 'true', false, true]);
    expect(told({ connected: false, loading: true })).toEqual(['Columbina is starting', true, '', 'true', false, true]);
    expect(told({ loading: true })).toEqual(['The task is being opened', true, '', 'true', false, true]);
  });

  it('says what became of what was sent and of what was asked', () => {
    const said = (given: { returned?: boolean; declined?: boolean; status?: Workspace['status'] }) => {
      cleanup();
      render(<Held workspace={workspaceWith({ status: given.status ?? statusOf() })} sent={taken()} returned={given.returned} declined={given.declined} />);
      return hint();
    };
    const back = 'What you had sent was not read. It is back in the field.';
    expect(said({ returned: true })).toBe(back);
    expect(said({ declined: true, status: atWork })).toBe('It was not done. You can tell Columbina how to go about it instead.');
    // Once the turn is over there is nothing left to tell it.
    expect(said({ declined: true })).toBe(keysIdle);
    expect(said({ returned: true, declined: true, status: atWork })).toBe(back);
  });

  it('says of the first Esc before anything else, and of stopping before that', () => {
    const workspace = workspaceWith({ status: atWork });
    const sent = taken();
    const { rerender } = render(<Held workspace={workspace} sent={sent} returned declined />);
    press('Escape');
    expect(hint()).toBe(escAgain);
    rerender(<Held workspace={{ ...workspace, cancelling: true }} sent={sent} returned declined />);
    expect(hint()).toBe('Stopping');
  });

  it('shows what a person sent and nothing has read yet', () => {
    const entry = (words: string, source: string) => ({ source, message: { role: 'user', content: [{ type: 'text', text: words }] } });
    const projection = {
      ...projectEvents([], false),
      queues: {
        'next-turn': [entry('Then run the tests.', 'user'), entry('A reminder that a plugin put in.', 'plugin')],
        'next-step': [entry('Mind the build.', 'steer')],
      },
    };
    render(<Held workspace={workspaceWith({ status: atWork, projection })} sent={taken()} />);
    expect(one('queue').getAttribute('aria-label')).toBe('Waiting');
    expect([...one('queue').querySelectorAll('li')].map(row => [...row.querySelectorAll('span')].map(part => part.textContent)))
      .toEqual([['Then run the tests.', 'Waiting'], ['Mind the build.', 'Waiting']]);
    cleanup();
    render(<Held workspace={workspaceWith()} sent={taken()} />);
    expect(marked('queue')).toEqual([]);
  });

  it('is as high as what is written in it, up to a limit, and scrolls only from there on', () => {
    let high = 64;
    Object.defineProperty(HTMLTextAreaElement.prototype, 'scrollHeight', { configurable: true, get: () => high });
    try {
      render(<Held workspace={workspaceWith()} sent={taken()} />);
      const shape = () => [box().style.height, box().style.overflowY];
      expect(shape()).toEqual(['64px', 'hidden']);
      high = 220;
      write('More.');
      expect(shape()).toEqual(['220px', 'hidden']);
      high = 221;
      write('More still.');
      expect(shape()).toEqual(['220px', 'auto']);
      high = 40;
      write('');
      expect(shape()).toEqual(['40px', 'hidden']);
    } finally {
      Reflect.deleteProperty(HTMLTextAreaElement.prototype, 'scrollHeight');
    }
  });
});
