import { useEffect, useRef, useState } from 'react';
import { useWords } from '../../app/locale';

/** Inline rename: IME confirmation is not submission; a pending write is submitted once. */
export function NameField({ name, onCommit, onCancel }: { name: string; onCommit: (name: string) => Promise<boolean>; onCancel: () => void }) {
  const words = useWords();
  const input = useRef<HTMLInputElement>(null);
  const pending = useRef(false);
  const [value, setValue] = useState(name);
  const [saving, setSaving] = useState(false);
  useEffect(() => { input.current?.focus(); input.current?.select(); }, []);
  const commit = async () => {
    if (pending.current) return;
    if (!value.trim() || value.trim() === name) { onCancel(); return; }
    pending.current = true;
    setSaving(true);
    try { if (await onCommit(value.trim())) onCancel(); }
    finally { pending.current = false; setSaving(false); }
  };
  return <input ref={input} className="pool-name-field" aria-label={words.label.studio.media.name} value={value} disabled={saving}
    onChange={event => setValue(event.target.value)} onClick={event => event.stopPropagation()}
    onDoubleClick={event => event.stopPropagation()} onBlur={() => void commit()}
    onKeyDown={event => {
      event.stopPropagation();
      if (event.nativeEvent.isComposing || event.keyCode === 229) return;
      if (event.key === 'Enter') { event.preventDefault(); void commit(); }
      else if (event.key === 'Escape') { event.preventDefault(); onCancel(); }
    }} />;
}
