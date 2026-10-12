/** The sizes of type that can be chosen. How much larger or smaller each is stands in `ui/tokens.css`. */
export const sizes = ['small', 'standard', 'large'] as const;
export type SizeId = typeof sizes[number];

export const sizeStorageKey = 'columbina-size';

/** The size that was chosen last, or the standard one. */
export function initialSize(): SizeId {
  try {
    const saved = localStorage.getItem(sizeStorageKey);
    if (sizes.some(size => size === saved)) return saved as SizeId;
  } catch {
    // Without storage the choice holds for this window.
  }
  return 'standard';
}

export function storeSize(size: SizeId): void {
  try {
    localStorage.setItem(sizeStorageKey, size);
  } catch {
    // See above.
  }
}
