export interface Selection { ids: string[]; focus: string | null; anchor: string | null }
export const EMPTY_SELECTION: Selection = { ids: [], focus: null, anchor: null };

/** Extend from the last plain/toggle click; keep the list's order for multi-item operations. */
export function choose(selection: Selection, order: readonly string[], id: string, toggle: boolean, extend: boolean): Selection {
  if (!order.includes(id)) return selection;
  let ids: string[];
  if (extend && selection.anchor !== null && order.includes(selection.anchor)) {
    const [from, to] = [order.indexOf(selection.anchor), order.indexOf(id)].sort((a, b) => a - b);
    ids = [...(toggle ? selection.ids : []), ...order.slice(from, to + 1)];
  } else ids = toggle ? (selection.ids.includes(id) ? selection.ids.filter(each => each !== id) : [...selection.ids, id]) : [id];
  return { ids: order.filter(each => ids.includes(each)), focus: id, anchor: extend ? selection.anchor ?? id : id };
}
