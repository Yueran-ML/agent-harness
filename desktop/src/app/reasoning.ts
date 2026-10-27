/**
 * The language that the model reasons in, as the user chose it in the settings. The program keeps
 * a language of its own, which `columbina reasoning-language` sets. The page tells it the one that
 * the choice leads to, when that is another one than it told it last.
 */

/** What the user chose: a language, the one of the interface, or none at all. */
export type ReasoningChoice = 'interface' | 'zh' | 'en' | 'none';

/** What the program is told: a code that it knows, or none. */
export type ReasoningLanguage = 'zh' | 'en' | 'ja' | 'none';

export const reasoningChoices: readonly ReasoningChoice[] = ['interface', 'zh', 'en', 'none'];

const choiceKey = 'columbina-reasoning-language';
const toldKey = 'columbina-reasoning-language-told';

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    // Without storage the choice holds for this window.
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // See above.
  }
}

/** What was chosen before. Nothing is asked for where nothing was chosen. */
export function storedReasoning(): ReasoningChoice {
  const stored = read(choiceKey);
  return reasoningChoices.find(choice => choice === stored) ?? 'none';
}

export function storeReasoning(choice: ReasoningChoice): void {
  write(choiceKey, choice);
}

/** The language that a choice leads to, where the interface speaks the language that a page names so. */
export function reasoningLanguage(choice: ReasoningChoice, shown: string): ReasoningLanguage {
  if (choice !== 'interface') return choice;
  if (shown.startsWith('zh')) return 'zh';
  return shown.startsWith('ja') ? 'ja' : 'en';
}

/** The language that the program was told last, as far as this page knows. */
export function toldReasoning(): string | null {
  return read(toldKey);
}

export function rememberTold(language: ReasoningLanguage): void {
  write(toldKey, language);
}

/** Whether the shell said that it told no program (it has none): then nothing is remembered as told. */
export function shellSkipped(said: unknown): boolean {
  return typeof said === 'object' && said !== null && (said as { skipped?: unknown }).skipped === true;
}
