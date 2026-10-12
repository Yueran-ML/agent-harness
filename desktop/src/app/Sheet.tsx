import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { useWords } from './locale';

/**
 * A window above the window, for something that has to be settled before
 * anything else. Escape and the cross close it, and the keyboard goes back to
 * where it was when it opened.
 */
export function Sheet({ title, children, foot, onClose, wide = false }: {
  title: string;
  children: ReactNode;
  foot?: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const words = useWords();
  const dialog = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    // A sheet that asks for a decision begins at the answer that changes nothing.
    dialog.current?.querySelector<HTMLElement>('[data-first]')?.focus();
    return () => {
      // Whatever had the keyboard gets it back, if it is still there to take it.
      if (before && before.isConnected) before.focus();
    };
  }, []);

  return <dialog
    ref={dialog}
    className="sheet"
    style={wide ? { width: 'min(720px, calc(100vw - 48px))' } : undefined}
    aria-label={title}
    onCancel={event => { event.preventDefault(); onClose(); }}
  >
    <div className="sheet-head">
      <h2>{title}</h2>
      <button className="icon-btn" onClick={onClose} aria-label={words.label.close} title={words.label.close}>
        <X size={16} />
      </button>
    </div>
    <div className="sheet-body">{children}</div>
    {foot && <div className="sheet-foot">{foot}</div>}
  </dialog>;
}
