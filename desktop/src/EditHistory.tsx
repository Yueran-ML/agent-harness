import { Check, ChevronRight, Clock3, FileCode2, Square, X } from 'lucide-react';
import type { ConversationItem, ToolItem } from './state';

function readArguments(value: string): Record<string, unknown> | null {
  try {
    const parsed: unknown = JSON.parse(value);
    return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

function EditEntry({ item }: { item: ToolItem }) {
  const args = readArguments(item.arguments);
  const path = typeof args?.path === 'string' ? args.path : 'Path unavailable';
  const status = item.interrupted ? 'Interrupted' : item.error ? 'Failed'
    : item.result === null ? 'Pending' : 'Applied';
  const StatusIcon = status === 'Applied' ? Check : status === 'Failed' ? X
    : status === 'Interrupted' ? Square : Clock3;
  const isEdit = item.name === 'edit';
  const hasEditText = isEdit && typeof args?.old === 'string' && typeof args?.new === 'string';
  const hasWriteText = !isEdit && typeof args?.content === 'string';

  return <details className="edit-entry">
    <summary className="edit-entry-header">
      <FileCode2 size={15} aria-hidden="true" />
      <span><code title={path}>{path}</code><small>{isEdit ? 'Edit' : 'Write'}</small></span>
      <span className="edit-status" data-status={status.toLowerCase()}>
        <StatusIcon size={12} aria-hidden="true" />{status}
      </span>
      <ChevronRight size={13} aria-hidden="true" />
    </summary>
    {hasEditText ? <>
      <div className="edit-fragment" data-kind="removed"><span>Before</span><pre>{args.old as string || '(Empty text)'}</pre></div>
      <div className="edit-fragment" data-kind="added"><span>After</span><pre>{args.new as string || '(Empty text)'}</pre></div>
      {args.replace_all === true && <p className="edit-history-note">The call requested replacement of every matching occurrence.</p>}
    </> : hasWriteText ? <div className="edit-fragment" data-kind="written"><span>Content</span><pre>{args.content as string || '(Empty content)'}</pre></div>
      : <div className="edit-fragment"><span>Arguments</span><pre>{item.arguments || '(No arguments)'}</pre></div>}
    {item.result !== null && <div className="edit-fragment" data-kind="result"><span>Tool result</span><pre>{item.result || '(No output)'}</pre></div>}
    {item.error && <p className="edit-history-note">{item.error.code}: {item.error.reason}</p>}
    {item.interrupted && <p className="edit-history-note">This tool did not produce a result before the turn ended.</p>}
    {status === 'Pending' && <p className="edit-history-note">Waiting for the tool result.</p>}
  </details>;
}

export function EditHistory({ items }: { items: ConversationItem[] }) {
  const edits = items.filter((item): item is ToolItem => item.type === 'tool' && (item.name === 'edit' || item.name === 'write'));
  return <div className="edit-history">
    {edits.length === 0 ? <div className="edit-empty"><FileCode2 size={22} aria-hidden="true" /><p>No edits in this task</p></div>
      : <><p className="edit-history-note">Tool operations recorded in this task.</p>{edits.map(item => <EditEntry key={item.id} item={item} />)}</>}
  </div>;
}
