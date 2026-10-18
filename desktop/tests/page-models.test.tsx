// @vitest-environment jsdom
/**
 * The model on a button, and the models of its service in the panel that the button opens: on
 * its own, and at the foot of the field.
 */
import { useState, type ComponentProps } from 'react';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Composer } from '../src/app/Composer';
import { ModelPicker } from '../src/app/ModelPicker';
import { LEAST_ROOM } from '../src/app/room';
import type { Failure, Listed } from '../src/key/form';
import type { Models, ServiceModels } from '../src/useModels';
import type { Workspace } from '../src/useWorkspace';
import { likeABrowser, modelsWith, service, standing, statusOf, workspaceWith } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  for (const left of document.querySelectorAll('body > button')) left.remove();
});

type Props = ComponentProps<typeof ModelPicker>;

const services: Listed = {
  use: 'deepseek',
  services: [
    service('deepseek', 'DeepSeek', { key: 'stored' }),
    service('ollama', 'Ollama', { key_env: null }),
    service('lmstudio', 'LM Studio', { key_env: null }),
    service('my-box', 'my-box', { own: true, key_env: null }),
    service('my-own', 'my-own', { own: true, key: 'stored' }),
  ],
};
const listed = (...models: string[]): Models => ({ kind: 'listed', models, at: 0 });
const failedWith = (failure: Failure, said = ''): Models => ({ kind: 'failed', failure, said, at: 0 });
const two = listed('deepseek-flash', 'deepseek-pro');

/** The picker of the service in use, with two models listed, unless a test says otherwise. */
function drawn(given: Partial<Props> = {}) {
  const calls = { onOpen: vi.fn(), onAsk: vi.fn(), onChoose: vi.fn(), onSettings: vi.fn() };
  const props: Props = {
    provider: 'deepseek', model: 'deepseek-flash', services, models: two, choice: 'free', replay: false, placement: 'up',
    ...calls, ...given,
  };
  const { rerender } = render(<ModelPicker {...props} />);
  return { ...calls, again: (more: Partial<Props>) => rerender(<ModelPicker {...props} {...more} />) };
}

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const opener = () => one('model-button') as HTMLButtonElement;
const open = () => fireEvent.click(opener());
const panel = () => marked('model-panel')[0] ?? null;
const filter = () => one('models-filter') as HTMLInputElement;
const options = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')];
/** What the list offers: a tick before the model in use, and an arrow after the one that the keyboard is on. */
const offered = () => options().map(option => `${option.getAttribute('aria-current') === 'true' ? '✓ ' : ''}${option.textContent}`
  + `${option.getAttribute('aria-selected') === 'true' ? ' <' : ''}`);
const notes = () => [...one('models-notes').querySelectorAll('p')].map(note => note.textContent);
const write = (text: string) => { fireEvent.change(filter(), { target: { value: text } }); };
/** Press a key, in the filter unless a test says where. Whether the page took the key, so that the browser does nothing with it. */
const press = (key: string, held: KeyboardEventInit = {}, at: Element = filter()) => !fireEvent.keyDown(at, { key, ...held });

const asking = 'Asking DeepSeek for its models.';
const whenIdle = 'The model can be changed when Columbina is at rest.';
const noMatch = 'No model matches.';

describe('the button of the model', () => {
  it('shows the model in use, names it for whoever cannot see it, and says its service when pointed at', () => {
    drawn();
    expect(opener().textContent).toBe('deepseek-flash');
    expect(opener().getAttribute('aria-label')).toBe('Model: deepseek-flash');
    expect(opener().title).toBe('DeepSeek · deepseek-flash');
    expect(opener().getAttribute('aria-haspopup')).toBe('dialog');
    expect(opener().getAttribute('aria-expanded')).toBe('false');
    expect(opener().hasAttribute('aria-controls')).toBe(false);
    expect(panel()).toBeNull();
    open();
    expect(opener().getAttribute('aria-expanded')).toBe('true');
    expect(opener().getAttribute('aria-controls')).toBe(panel()!.id);
    expect(panel()!.getAttribute('role')).toBe('dialog');
    expect(panel()!.getAttribute('aria-label')).toBe('Model');
    expect(panel()!.dataset.placement).toBe('up');
    // It is no higher than the room that it has, which the DOM of the tests makes the least.
    expect(panel()!.style.maxHeight).toBe(`${LEAST_ROOM}px`);
    // Pressed again, it closes what it opened.
    open();
    expect(panel()).toBeNull();
    expect(opener().getAttribute('aria-expanded')).toBe('false');
  });

  it('opens below its button where it has too little room above and more below', () => {
    // A panel of 400 has 92 above its button, and more below it within the window.
    const putBack = standing(element => (element.dataset.test === 'model-panel' ? [-300, 100]
      : element.classList.contains('model-picker') ? [108, 140] : null));
    try {
      drawn();
      open();
      expect(panel()!.dataset.placement).toBe('down');
      expect(panel()!.style.maxHeight).toBe(`${window.innerHeight - 140 - 8 - 8}px`);
    } finally {
      putBack();
    }
  });

  it('calls a service that the list does not have by its name', () => {
    drawn({ provider: 'elsewhere', model: 'tiny', services: null });
    expect(opener().title).toBe('elsewhere · tiny');
  });

  it('cannot be pressed where nothing can be chosen, other than in a replay, which says why', () => {
    const { again } = drawn({ choice: 'off' });
    expect(opener().disabled).toBe(true);
    again({ choice: 'busy' });
    expect(opener().disabled).toBe(false);
    again({ choice: 'off', replay: true });
    expect(opener().disabled).toBe(false);
  });
});

describe('the panel of the models', () => {
  it('asks for the models each time it opens, and has the keyboard in its filter', () => {
    const { onOpen } = drawn();
    open();
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(filter());
    expect(filter().placeholder).toBe('Filter, or write the name of a model');
    expect(filter().getAttribute('aria-label')).toBe('Filter, or write the name of a model');
    expect(filter().getAttribute('role')).toBe('combobox');
    open();
    open();
    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it('opens with an empty filter, whatever was written the time before', () => {
    drawn();
    open();
    write('pro');
    open();
    open();
    expect(filter().value).toBe('');
    expect(offered()).toEqual(['✓ deepseek-flash <', 'deepseek-pro']);
  });

  it('names the service, with where it is for one of the user\'s own, and asks it again from its head', () => {
    const { onAsk, again } = drawn();
    open();
    const head = () => panel()!.querySelector('.model-service')!;
    expect(head().textContent).toBe('DeepSeek');
    expect(head().getAttribute('title')).toBeNull();
    expect(one('models-again').getAttribute('aria-label')).toBe('List the models again');
    expect(one('models-again').querySelector('svg')?.classList.contains('spin')).toBe(false);
    fireEvent.click(one('models-again'));
    expect(onAsk).toHaveBeenCalledTimes(1);
    again({ provider: 'my-own', model: 'tiny', models: listed('tiny') });
    expect(head().textContent).toBe('my-own');
    expect(head().getAttribute('title')).toBe('http://10.0.0.5:8000/v1');
    // It is not asked twice at once. The button stays, and so does the keyboard on it.
    again({ models: { kind: 'asking', models: ['tiny'] } });
    expect(one('models-again').querySelector('svg')?.classList.contains('spin')).toBe(true);
    fireEvent.click(one('models-again'));
    expect(onAsk).toHaveBeenCalledTimes(1);
    expect((one('models-again') as HTMLButtonElement).disabled).toBe(false);
  });

  it('lists the models in the order of the service, marks the one in use, and begins at it', () => {
    drawn({ models: listed('deepseek-pro', 'deepseek-flash', 'deepseek-old') });
    open();
    expect(offered()).toEqual(['deepseek-pro', '✓ deepseek-flash <', 'deepseek-old']);
    const list = panel()!.querySelector('[role="listbox"]')!;
    expect(list.getAttribute('aria-label')).toBe('Model');
    expect(filter().getAttribute('aria-controls')).toBe(list.id);
    expect(filter().getAttribute('aria-expanded')).toBe('true');
    expect(filter().getAttribute('aria-activedescendant')).toBe(options()[1].id);
    // The tick is a picture, beside the model in use and no other, which is said to whoever cannot see it.
    expect(options().map(option => !!option.querySelector('svg'))).toEqual([false, true, false]);
    expect(options()[1].querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    // Each says its whole name when it is pointed at: a long one is cut short.
    expect(options().map(option => option.title)).toEqual(['deepseek-pro', 'deepseek-flash', 'deepseek-old']);
    // Where the model in use is none of them, the keyboard begins at the first.
    cleanup();
    drawn({ model: 'deepseek-gone' });
    open();
    expect(offered()).toEqual(['deepseek-flash <', 'deepseek-pro']);
  });

  it('keeps what the filter finds, the closest first, and the keyboard begins at the first', () => {
    drawn({ models: listed('deepseek-flash', 'deepseek-pro', 'gpt-pro', 'pro') });
    open();
    write(' PRO ');
    expect(offered()).toEqual(['pro <', 'deepseek-pro', 'gpt-pro']);
    write('');
    expect(offered()).toEqual(['✓ deepseek-flash <', 'deepseek-pro', 'gpt-pro', 'pro']);
  });

  it('begins at the closest model once something is written, also where the model in use is among those kept', () => {
    drawn({ model: 'deepseek-pro' });
    open();
    expect(offered()).toEqual(['deepseek-flash', '✓ deepseek-pro <']);
    write('deepseek');
    expect(offered()).toEqual(['deepseek-flash <', '✓ deepseek-pro', 'Use deepseek']);
  });

  it('goes back to the closest model when more is written, wherever the keyboard was', () => {
    drawn();
    open();
    press('ArrowDown');
    expect(offered()).toEqual(['✓ deepseek-flash', 'deepseek-pro <']);
    write('deepseek');
    expect(offered()).toEqual(['✓ deepseek-flash <', 'deepseek-pro', 'Use deepseek']);
  });

  it('leaves the keys of a button in the panel to the button', () => {
    const { onChoose } = drawn();
    open();
    const again = one('models-again');
    again.focus();
    for (const key of ['Enter', 'ArrowDown', 'ArrowUp']) expect(press(key, {}, again), key).toBe(false);
    expect(onChoose).not.toHaveBeenCalled();
    expect(panel()).not.toBeNull();
    expect(offered()).toEqual(['✓ deepseek-flash <', 'deepseek-pro']);
  });

  it('offers what is written as a model of its own, after those that the filter keeps', () => {
    drawn();
    open();
    write(' deepseek-old ');
    expect(offered()).toEqual(['Use deepseek-old <']);
    expect(notes()).toEqual([noMatch]);
    // The name is written as names of models are, and the words around it as words are.
    expect(options()[0].querySelector('.mono')?.textContent).toBe('deepseek-old');
    write('deepseek');
    expect(offered()).toEqual(['✓ deepseek-flash <', 'deepseek-pro', 'Use deepseek']);
    expect(notes()).toEqual([]);
    // A model that the service lists is not offered twice, in whatever case it is written.
    write('DeepSeek-Pro');
    expect(offered()).toEqual(['deepseek-pro <']);
  });

  it('does not offer what looks like a key or is no name, and says why', () => {
    drawn();
    open();
    write('sk-0123456789abcdef');
    expect(offered()).toEqual([]);
    expect(notes()).toEqual(['This looks like a key, and not like the name of a model.']);
    expect(panel()!.querySelector('[role="listbox"]')).toBeNull();
    expect(filter().getAttribute('aria-expanded')).toBe('false');
    expect(filter().hasAttribute('aria-controls')).toBe(false);
    expect(filter().hasAttribute('aria-activedescendant')).toBe(false);
    write('two words');
    expect(offered()).toEqual([]);
    expect(notes()).toEqual([noMatch]);
  });

  it('moves through the models with the arrows, round the ends, and with the pointer', () => {
    drawn({ models: listed('a-1', 'a-2', 'a-3'), model: 'a-2' });
    open();
    expect(press('ArrowDown')).toBe(true);
    expect(offered()).toEqual(['a-1', '✓ a-2', 'a-3 <']);
    press('ArrowDown');
    expect(offered()).toEqual(['a-1 <', '✓ a-2', 'a-3']);
    expect(press('ArrowUp')).toBe(true);
    expect(offered()).toEqual(['a-1', '✓ a-2', 'a-3 <']);
    fireEvent.mouseEnter(options()[0]);
    expect(offered()).toEqual(['a-1 <', '✓ a-2', 'a-3']);
    expect(filter().getAttribute('aria-activedescendant')).toBe(options()[0].id);
    // The other keys are those of the filter.
    for (const key of ['a', 'Home', 'End', 'ArrowLeft', 'Tab']) expect(press(key), key).toBe(false);
  });

  it('keeps the model that the keyboard is on in sight, once the panel has its height', () => {
    // The model that was brought into sight, and how high the panel was at that moment.
    const seen: [Element, string | undefined][] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) {
      seen.push([this, this.closest<HTMLElement>('[data-test="model-panel"]')?.style.maxHeight]);
    };
    drawn({ models: listed('a-1', 'a-2', 'a-3'), model: 'a-1' });
    open();
    expect(seen.at(-1)).toEqual([options()[0], `${LEAST_ROOM}px`]);
    press('ArrowUp');
    expect(seen.at(-1)).toEqual([options()[2], `${LEAST_ROOM}px`]);
  });

  it('chooses with Enter or a click, closes, and gives the keyboard on', () => {
    const afterChoice = vi.fn();
    const { onChoose } = drawn({ afterChoice });
    open();
    press('ArrowDown');
    expect(press('Enter')).toBe(true);
    expect(onChoose.mock.calls).toEqual([['deepseek-pro']]);
    expect(panel()).toBeNull();
    expect(afterChoice).toHaveBeenCalledTimes(1);
    open();
    // A press on a model leaves the keyboard in the filter.
    expect(fireEvent.mouseDown(options()[1])).toBe(false);
    fireEvent.click(options()[1]);
    expect(onChoose.mock.calls).toEqual([['deepseek-pro'], ['deepseek-pro']]);
    expect(afterChoice).toHaveBeenCalledTimes(2);
    open();
    write('deepseek-old');
    press('Enter');
    expect(onChoose.mock.calls.at(-1)).toEqual(['deepseek-old']);
  });

  it('changes nothing where the model in use is chosen, and gives the keyboard back to the button where nothing else takes it', () => {
    const { onChoose } = drawn();
    open();
    press('Enter');
    expect(onChoose).not.toHaveBeenCalled();
    expect(panel()).toBeNull();
    expect(document.activeElement).toBe(opener());
  });

  it('takes Enter in the filter whatever is held down, so that no form around it is sent', () => {
    const { onChoose } = drawn();
    open();
    expect(press('Enter', { shiftKey: true })).toBe(true);
    expect(press('Enter', { ctrlKey: true })).toBe(true);
    write('sk-0123456789abcdef');
    expect(press('Enter')).toBe(true);
    expect(onChoose).not.toHaveBeenCalled();
    expect(panel()).not.toBeNull();
    // The Enter that ends what an input method puts together is the input method's.
    write('deepseek');
    expect(press('Enter', { isComposing: true })).toBe(false);
    expect(press('Enter', { keyCode: 229 })).toBe(false);
    expect(onChoose).not.toHaveBeenCalled();
  });

  it('closes with Esc, from wherever the keyboard is in it, and gives the keyboard back to the button', () => {
    const seen = vi.fn();
    document.addEventListener('keydown', seen);
    drawn();
    open();
    expect(press('Escape')).toBe(true);
    expect(panel()).toBeNull();
    expect(document.activeElement).toBe(opener());
    // Esc closes the panel and nothing that holds it, such as the settings.
    expect(seen).not.toHaveBeenCalled();
    open();
    one('models-again').focus();
    expect(press('Escape', {}, one('models-again'))).toBe(true);
    expect(panel()).toBeNull();
    document.removeEventListener('keydown', seen);
  });

  it('closes at a click elsewhere and where the keyboard goes elsewhere, and not for what is done in it', () => {
    drawn();
    const elsewhere = document.body.appendChild(document.createElement('button'));
    open();
    fireEvent.pointerDown(panel()!.querySelector('.model-service')!);
    fireEvent.pointerDown(opener());
    expect(panel()).not.toBeNull();
    fireEvent.pointerDown(elsewhere);
    expect(panel()).toBeNull();
    open();
    fireEvent.focusOut(filter(), { relatedTarget: one('models-again') });
    // Where the window loses the keyboard, the panel is there when it comes back.
    fireEvent.focusOut(filter(), { relatedTarget: null });
    expect(panel()).not.toBeNull();
    fireEvent.focusOut(filter(), { relatedTarget: elsewhere });
    expect(panel()).toBeNull();
  });

  it('closes where nothing can be chosen any more', () => {
    const { again } = drawn();
    open();
    again({ choice: 'busy' });
    expect(panel()).not.toBeNull();
    again({ choice: 'off' });
    expect(panel()).toBeNull();
  });

  it('lets the models be looked at while Columbina is at work, and none be chosen', () => {
    const { onChoose } = drawn({ choice: 'busy' });
    open();
    expect(notes()).toEqual([whenIdle]);
    expect(options().map(option => option.getAttribute('aria-disabled'))).toEqual(['true', 'true']);
    press('ArrowDown');
    press('Enter');
    fireEvent.click(options()[0]);
    expect(onChoose).not.toHaveBeenCalled();
    expect(panel()).not.toBeNull();
    expect(offered()).toEqual(['✓ deepseek-flash', 'deepseek-pro <']);
  });

  it('says that the service is asked while nothing is known of its models, and shows what was listed before', () => {
    const { again } = drawn({ models: null });
    open();
    expect(notes()).toEqual([asking]);
    expect(panel()!.querySelector('.model-asking .moon')).not.toBeNull();
    expect(panel()!.querySelector('[role="listbox"]')).toBeNull();
    again({ models: { kind: 'asking', models: [] } });
    expect(notes()).toEqual([asking]);
    again({ models: { kind: 'asking', models: ['deepseek-flash'] } });
    expect(notes()).toEqual([]);
    expect(offered()).toEqual(['✓ deepseek-flash <']);
    again({ models: two });
    expect(one('models-again').querySelector('svg')?.classList.contains('spin')).toBe(false);
  });

  it('says why the service listed no models, by what went wrong', () => {
    const { again } = drawn();
    open();
    const told: [Failure, string][] = [
      ['network', 'The service could not be reached. See whether this computer is online, and that the proxy is on if there is one.'],
      ['timeout', 'The service did not answer in time. Try again in a moment.'],
      ['key', 'The service does not take the key that is set up for it.'],
      ['forbidden', 'The service takes the key and does not let it list the models: the account may have no money left, '
        + 'or the key no right to them.'],
      ['address', 'Nothing that serves models answers at the address of this service.'],
      ['busy', 'The service is busy. Try again in a moment.'],
      ['unknown', 'The models could not be listed.'],
    ];
    for (const [failure, sentence] of told) {
      again({ models: failedWith(failure, 'what the program said') });
      // What the program said stands beside a failure that has no words of its own.
      expect(notes(), failure).toEqual([failure === 'unknown' ? `${sentence} What the program said.` : sentence]);
    }
    again({ models: failedWith('unknown') });
    expect(notes()).toEqual(['The models could not be listed.']);
  });

  it('says of a service on this computer that does not answer how to start it', () => {
    const { again } = drawn({ provider: 'ollama', model: 'qwen3:8b', models: failedWith('network') });
    open();
    expect(notes()).toEqual(['Ollama does not answer on this computer. Start it, for instance with ollama serve, and try again.']);
    again({ provider: 'lmstudio' });
    expect(notes()).toEqual(['LM Studio does not answer on this computer. Start its local server, and try again.']);
    again({ provider: 'my-box' });
    expect(notes()).toEqual(['The service does not answer on this computer. Start it, and try again.']);
    // One that takes a key is somewhere else, and one that answers too late is running.
    again({ provider: 'my-own' });
    expect(notes()[0]).toMatch(/^The service could not be reached\./);
    again({ provider: 'ollama', models: failedWith('timeout') });
    expect(notes()).toEqual(['The service did not answer in time. Try again in a moment.']);
  });

  it('asks again from where it says what went wrong, gives the keyboard to the filter, and offers what is written', () => {
    const { onAsk } = drawn({ models: failedWith('network') });
    open();
    const again = one('models-try-again');
    expect(again.textContent).toBe('Try again');
    again.focus();
    fireEvent.click(again);
    expect(onAsk).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(filter());
    write('deepseek-old');
    expect(offered()).toEqual(['Use deepseek-old <']);
  });

  it('says where the service lists no model, and offers what is written', () => {
    drawn({ models: listed() });
    open();
    expect(notes()).toEqual(['DeepSeek lists no model.']);
    expect(panel()!.querySelector('[role="listbox"]')).toBeNull();
    write('deepseek-old');
    expect(notes()).toEqual(['DeepSeek lists no model.']);
    expect(offered()).toEqual(['Use deepseek-old <']);
  });

  it('says in a replay that no service is asked there, and asks none', () => {
    const { onOpen } = drawn({ replay: true, choice: 'off', models: null });
    open();
    expect(notes()).toEqual(['The models of a service are listed where the application runs the service itself, not in a replay.']);
    expect(marked('models-filter')).toEqual([]);
    expect(marked('models-again')).toEqual([]);
    expect(options()).toEqual([]);
    expect(onOpen).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(panel());
    expect(press('Escape', {}, panel()!)).toBe(true);
    expect(panel()).toBeNull();
  });

  it('leads to the settings of the model, and gives the keyboard to the button first', () => {
    const onSettings = vi.fn(() => {
      // The settings take note of where the keyboard was, to give it back when they close.
      expect(document.activeElement).toBe(opener());
    });
    drawn({ onSettings });
    open();
    expect(one('model-settings').textContent).toBe('Model settings');
    fireEvent.click(one('model-settings'));
    expect(onSettings).toHaveBeenCalledTimes(1);
    expect(panel()).toBeNull();
    cleanup();
    drawn({ onSettings: undefined });
    open();
    expect(marked('model-settings')).toEqual([]);
  });
});

describe('the model at the foot of the field', () => {
  const able = {
    protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work',
    commands: [{ name: 'model', summary: 'Show the model, or name another.' }, { name: 'compact', summary: 'Sum up.' }],
  };
  /** The field with what is written in it kept by whoever shows it, as the page does. */
  function Held({ workspace, models, calls }: {
    workspace: Workspace;
    models: ServiceModels;
    calls: { onChooseModel: (model: string) => void; onModelSettings: () => void };
  }) {
    const [draft, setDraft] = useState('');
    const [field] = useState(() => ({ current: null as HTMLTextAreaElement | null }));
    return <Composer
      workspace={workspace} draft={draft} onDraft={setDraft} onSend={async () => true} field={field}
      returned={false} declined={false} commandsAsked={0} models={models} {...calls}
      attachments={[]} refused={[]} onRemoveAttachment={vi.fn()}
    />;
  }
  const field = (given: Partial<Workspace> = {}, models = modelsWith({ deepseek: two })) => {
    const calls = { onChooseModel: vi.fn(), onModelSettings: vi.fn() };
    const drawnField = render(<Held workspace={workspaceWith({ hello: able, services, ...given })} models={models} calls={calls} />);
    return {
      ...calls,
      models,
      again: (more: Partial<Workspace>) => drawnField.rerender(
        <Held workspace={workspaceWith({ hello: able, services, ...more })} models={models} calls={calls} />,
      ),
    };
  };
  const box = () => one('composer') as HTMLTextAreaElement;

  it('stands after the button of the commands, and opens above the field', () => {
    field();
    const row = opener().closest('.composer-row')!;
    const buttons = [...row.querySelectorAll('button')];
    expect(buttons.indexOf(opener())).toBe(1);
    expect(buttons[0].getAttribute('aria-label')).toBe('Commands');
    open();
    expect(panel()!.dataset.placement).toBe('up');
  });

  it('asks for the models of the service in use, and puts the list of commands away', () => {
    const { models } = field();
    fireEvent.change(box(), { target: { value: '/' } });
    expect(marked('commands')).toHaveLength(1);
    open();
    expect(models.freshen).toHaveBeenCalledWith('deepseek');
    expect(marked('commands')).toEqual([]);
    fireEvent.click(one('models-again'));
    expect(models.ask).toHaveBeenCalledWith('deepseek');
  });

  it('hands on the model that is chosen, and gives the keyboard back to the field', () => {
    const { onChooseModel } = field();
    open();
    fireEvent.click(options()[1]);
    expect(onChooseModel.mock.calls).toEqual([['deepseek-pro']]);
    expect(document.activeElement).toBe(box());
  });

  it('leads to the settings of the model', () => {
    const { onModelSettings } = field();
    open();
    fireEvent.click(one('model-settings'));
    expect(onModelSettings).toHaveBeenCalledTimes(1);
  });

  it('is there once the service has said which model it asks, and can be used as the field can', () => {
    const { again } = field({ status: null });
    expect(marked('model-button')).toEqual([]);
    const choice = () => {
      if (opener().disabled) return 'off';
      open();
      const said = notes().includes(whenIdle) ? 'busy' : 'free';
      open();
      return said;
    };
    for (const [given, expected] of [
      [{}, 'free'],
      [{ status: statusOf({ status: 'running' }) }, 'busy'],
      [{ sending: true }, 'busy'],
      [{ loading: true }, 'off'],
      [{ connected: false }, 'off'],
      [{ status: statusOf({ status: 'stopped' }) }, 'off'],
      [{ hasProject: false }, 'off'],
      // A service process that cannot be told of another model.
      [{ hello: { ...able, commands: [able.commands[1]] } }, 'off'],
    ] as [Partial<Workspace>, string][]) {
      again(given);
      expect(choice(), JSON.stringify(given)).toBe(expected);
    }
  });

  it('shows the model that the service says it asks, and says in a replay that no list is there', () => {
    const { again } = field({ status: statusOf({ model: { provider: 'ollama', model: 'qwen3:8b', context_window: null } }) });
    expect(opener().textContent).toBe('qwen3:8b');
    expect(opener().title).toBe('Ollama · qwen3:8b');
    again({ mode: 'mock' });
    open();
    expect(notes()).toEqual(['The models of a service are listed where the application runs the service itself, not in a replay.']);
  });
});
