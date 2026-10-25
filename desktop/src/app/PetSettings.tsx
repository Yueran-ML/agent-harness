import { useEffect, useState } from 'react';
import { FolderOpen } from 'lucide-react';
import type { PetChoice, PetChoices } from '../bridge';
import { useWords } from './locale';

/** What went wrong, in the words of what failed. */
const saidOf = (problem: unknown) => (problem instanceof Error ? problem.message : String(problem));

/**
 * The part of the settings for the pet: whether it shows and which one, where the task is told
 * for other programs, and the folder for pets of the user's own. Without the shell, as in a
 * replay, there is no pet and no part for it.
 */
export function PetSettings() {
  const words = useWords();
  const [choices, setChoices] = useState<PetChoices | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  // A shell without a pet, as in a replay, has no part for it.
  const bridge = window.columbina?.pets ? window.columbina : undefined;

  useEffect(() => {
    if (!bridge) return;
    const load = () => { void bridge.pets().then(setChoices).catch(() => {}); };
    load();
    // The pet may be hidden from its own menu.
    return bridge.onPet(load);
  }, [bridge]);
  if (!bridge || choices === null) return null;

  const change = async (asked: { on?: boolean; chosen?: string }) => {
    setFailed(null);
    try {
      await bridge.setPet(asked);
      setChoices(await bridge.pets());
    } catch (problem) {
      setFailed(saidOf(problem));
    }
  };
  const nameOf = (pet: PetChoice) => {
    const builtIn = pet.origin === 'builtin' ? (words.label.pet.builtin as Record<string, string>)[pet.key.slice('builtin/'.length)] : undefined;
    return builtIn ?? (pet.origin === 'user' ? `${pet.name} · ${words.label.pet.mine}` : pet.name);
  };

  return <section className="setting" data-test="pet">
    <h3>{words.label.pet.title}</h3>
    <div className="setting-row">
      <label className="check">
        <input type="checkbox" checked={choices.on} disabled={choices.pets.length === 0} onChange={event => void change({ on: event.target.checked })} data-test="pet-on" />
        {words.label.pet.show}
      </label>
    </div>
    {choices.pets.length === 0 ? <p className="caption" data-tone="warn">{words.hint.pet.none}</p>
      : <div className="setting-row" role="radiogroup" aria-label={words.label.pet.choose} data-test="pet-choices">
        {choices.pets.map(pet => <label key={pet.key} className="check" title={pet.description}>
          <input type="radio" name="pet" checked={choices.chosen === pet.key} onChange={() => void change({ chosen: pet.key })} />
          {nameOf(pet)}
        </label>)}
      </div>}
    <p className="caption">{words.hint.pet.about}</p>
    <p className="caption" data-test="pet-state-file">{words.hint.pet.stateFile(choices.stateFile)}</p>
    <div className="setting-row">
      <button type="button" className="btn" onClick={() => void bridge.openPetsFolder().catch(problem => setFailed(saidOf(problem)))} data-test="pet-folder">
        <FolderOpen size={14} aria-hidden="true" />{words.label.pet.openFolder}
      </button>
    </div>
    <p className="caption">{words.hint.pet.own}</p>
    {failed && <p className="caption" data-tone="warn" role="alert">{failed}</p>}
  </section>;
}
