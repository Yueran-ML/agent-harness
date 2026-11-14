import { useState } from 'react';
import type { Clip } from '../engine';
import { EqPanel } from '../audio/EqPanel';
import type { ProcessorProps } from '../audio/useProcessor';
import { useFairlightText } from '../fairlight/words';

/** Open the same four-band processor used by the mixer, with an explicit clip owner. */
export function ClipEq({ clips, ...props }: Omit<ProcessorProps, 'target'> & { clips: readonly Clip[] }) {
  const t = useFairlightText(), [open, setOpen] = useState(false);
  return <section className="clip-eq">
    <button type="button" className="inspector-section" aria-expanded={open} onClick={() => setOpen(value => !value)}>{t('Equalizer')}</button>
    {open && clips.map(clip => <div key={clip.id}>
      {clips.length > 1 && <h4>{clip.id}</h4>}
      <EqPanel {...props} target={{ clip: clip.id }} />
    </div>)}
  </section>;
}
