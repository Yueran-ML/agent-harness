/** Internal drags name media IDs in the current engine session, never filesystem paths. */
export const MEDIA_DRAG = 'application/x-columbina-media';
export function draggedMedia(data: Pick<DataTransfer, 'getData'>, session: number): string[] {
  try {
    const value = JSON.parse(data.getData(MEDIA_DRAG));
    return value.session === session && Array.isArray(value.media) && value.media.every((id: unknown) => typeof id === 'string')
      ? [...new Set<string>(value.media)] : [];
  } catch { return []; }
}
