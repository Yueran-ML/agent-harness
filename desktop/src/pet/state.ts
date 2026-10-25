/**
 * What a task is doing, as the plugin columbina.pets writes it to the file of its state, and the
 * shell hands it on to the window of the pet. See docs/pets.md in the repository.
 */
export type PetState = 'idle' | 'thinking' | 'review' | 'waiting' | 'failed' | 'done';
