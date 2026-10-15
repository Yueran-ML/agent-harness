/**
 * The models of a service as the page offers them: what a filter keeps of them and in which
 * order, and when what is typed is offered as a model of its own. Nothing here draws.
 */
import { looksLikeAKey } from '../key/form';

/** How long a list of models is taken as it is, before the service is asked again, in milliseconds. */
export const MODELS_FRESH_MS = 5 * 60_000;

/**
 * The models that a filter keeps, the closest first: the one that is written, then those that
 * begin with it, then the others that hold it, each in the order that the service gave. A filter
 * goes by letters and not by their case.
 */
export function filtered(models: readonly string[], query: string): string[] {
  const wanted = query.trim().toLowerCase();
  if (wanted === '') return [...models];
  const same: string[] = [];
  const starting: string[] = [];
  const holding: string[] = [];
  for (const model of models) {
    const name = model.toLowerCase();
    if (name === wanted) same.push(model);
    else if (name.startsWith(wanted)) starting.push(model);
    else if (name.includes(wanted)) holding.push(model);
  }
  return [...same, ...starting, ...holding];
}

/**
 * What is typed into the filter, where it is offered as a model of its own: a model that the
 * service does not list, or lists under another case, or a service that could not be asked. It
 * is written as the name of a model is, in one piece and not long, and it looks like no key: a
 * model is kept as it is written, and not where a key is kept.
 */
export function typedModel(models: readonly string[], query: string): string | null {
  const name = query.trim();
  if (!/^\S{1,200}$/.test(name) || /[\x00-\x1f\x7f]/.test(name) || looksLikeAKey(name)) return null;
  return models.some(model => model.toLowerCase() === name.toLowerCase()) ? null : name;
}
