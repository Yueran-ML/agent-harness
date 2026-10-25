import { useEffect, useState } from 'react';
import { AFTER_MS, petStateOf } from '../pet/state';
import type { Workspace } from '../useWorkspace';
import type { Messages } from '../messages/en';

/**
 * Tell the pet what the task that is open does, whenever that changes: its state, its label, and
 * what it says in its bubble in the language of the page. That a task is done is said only where
 * the user is not looking at the window. A turn that has just ended is shown for a while, and the
 * pet rests again then.
 */
export function usePet(workspace: Pick<Workspace, 'status' | 'approval' | 'projection'>, words: Messages) {
  const [, setNow] = useState(0);
  const shown = petStateOf(workspace, Date.now());
  const bubble = shown.state === 'waiting' ? words.hint.pet.waiting(shown.label)
    : shown.state === 'failed' ? words.hint.pet.failed
      : shown.state === 'done' && !document.hasFocus() ? words.hint.pet.done : '';
  useEffect(() => {
    if (shown.state !== 'done' && shown.state !== 'failed') return;
    const timer = setTimeout(() => setNow(Date.now()), AFTER_MS);
    return () => clearTimeout(timer);
  }, [shown.state]);
  useEffect(() => {
    // A shell without a pet, as in a replay or in a test, is told nothing.
    window.columbina?.petState?.({ state: shown.state, label: shown.label, bubble });
  }, [shown.state, shown.label, bubble]);
}
