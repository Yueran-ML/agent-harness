import { useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useOverlay } from '../../app/overlay';
import { useFairlightText } from './words';
let front = 90;
export function EffectWindow({ title, close, children }: { title: string; close: () => void; children: ReactNode }) {
  useOverlay(); const t = useFairlightText();
  const [position, setPosition] = useState({left:Math.min(180,innerWidth/4),top:100}), [z,setZ]=useState(()=>++front);
  const drag=useRef<{x:number;y:number;left:number;top:number}|null>(null);
  return createPortal(<aside className="insert-effect-window" role="dialog" aria-label={title} style={{...position,zIndex:z}} onPointerDown={()=>setZ(++front)} onKeyDown={event=>{if(event.key==='Escape'){event.stopPropagation();close();}}}>
    <header onPointerDown={event=>{if(event.button!==0 || (event.target as HTMLElement).closest('button'))return;drag.current={x:event.clientX,y:event.clientY,...position};event.currentTarget.setPointerCapture?.(event.pointerId);}}
      onPointerMove={event=>{const d=drag.current;if(d)setPosition({left:Math.max(0,Math.min(innerWidth-80,d.left+event.clientX-d.x)),top:Math.max(0,Math.min(innerHeight-40,d.top+event.clientY-d.y))});}}
      onPointerUp={()=>{drag.current=null;}} onPointerCancel={()=>{drag.current=null;}}><strong>{title}</strong><button type="button" aria-label={t('Close processor')} onClick={close}>×</button></header>{children}
  </aside>,document.body);
}
