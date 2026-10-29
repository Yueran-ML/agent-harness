import type { ReactNode } from 'react';

/** Media keeps the pool beside its source monitor; Cut and Edit share the same project pool. */
export function MediaPage({ pool, monitor }: { pool: ReactNode; monitor: ReactNode }) {
  return <div className="studio-body media-page">{pool}{monitor}</div>;
}
