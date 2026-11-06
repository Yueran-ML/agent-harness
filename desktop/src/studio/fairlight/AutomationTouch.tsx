import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type AutomationTouch as TouchMessage, type AutomationParameter, type Timeline } from '../engine';
import { categoryOfPath, ownerKey, type AutomationTarget } from './automation';
/** One in-flight RPC, 10 ms event sampling, newest movement wins; releases never disappear. */
export class TouchQueue {
  private target: AutomationTarget | null = null;
  private value = 0; private timer: ReturnType<typeof setTimeout> | null = null;
  private chain = Promise.resolve(); private pending: TouchMessage | null = null;
  private queued: { message:TouchMessage; replaceable:boolean }[]=[]; private sending=false;
  constructor(private timeline: string, private send: (message: TouchMessage) => Promise<unknown>, private fail: (error: unknown) => void) {}
  private enqueue(message: TouchMessage, replaceable=false) {
    const last=this.queued.at(-1);
    if(replaceable&&last?.replaceable&&last.message.path===message.path&&ownerKey(last.message)===ownerKey(message))last.message=message;
    else this.queued.push({message,replaceable});
    if(this.sending)return;this.sending=true;
    this.chain=(async()=>{
      while(this.queued.length){const next=this.queued.shift()!;try{await this.send(next.message);}catch(error){this.fail(error);this.pending=null;this.queued=this.queued.filter(x=>!x.message.touching);this.end();}}
      this.sending=false;
    })();
  }
  start(target: AutomationTarget, value: number) { this.end(); this.target = target; this.value = value; this.enqueue({ timeline: this.timeline, ...target.of, path: target.path, value, touching: true }); }
  change(value: number) {
    if (!this.target) return;
    this.value = value; this.pending = { timeline: this.timeline, ...this.target.of, path: this.target.path, value, touching: true };
    if (!this.timer) this.timer = setTimeout(() => { this.timer = null; const message = this.pending; this.pending = null; if (message) this.enqueue(message,true); }, 10);
  }
  end() {
    if (this.timer) clearTimeout(this.timer); this.timer = null;
    if (this.pending) this.enqueue(this.pending,true); this.pending = null;
    if (this.target) this.enqueue({ timeline: this.timeline, ...this.target.of, path: this.target.path, value: this.value, touching: false });
    this.target = null;
  }
  active() { return this.target !== null; }
  settled() { return this.chain; }
}
export interface AutomationTouchControl {
  enabled: boolean; recording: boolean; trim: boolean; preview: boolean;
  active: (target?: AutomationTarget) => boolean; start: (target: AutomationTarget, value: number) => boolean;
  change: (value: number) => void; end: () => void; instant: (target: AutomationTarget, value: number) => boolean;
  startBatch: (values: {target:AutomationTarget;value:number}[])=>boolean;
  changeBatch: (values: {target:AutomationTarget;value:number}[])=>void;
  instantBatch: (values: {target:AutomationTarget;value:number}[])=>boolean;
  show: (target: AutomationTarget) => void;
  live: { target: AutomationTarget; value: number } | null;
  lives: { target: AutomationTarget; value: number }[];
  reading: (target: AutomationTarget, baseline: number, parameter?:Pick<AutomationParameter,'kind'|'scale'>) => number;
  status: (target: AutomationTarget) => 'write'|'preview'|null;
}
const Context = createContext<AutomationTouchControl | null>(null);
export const useAutomationTouch = () => useContext(Context);
export function AutomationTouchProvider({ bridge, session, timeline, revision=0, enabled, recording, playing=true, read=false, show, onFailure, children }: {
  bridge: Pick<EngineBridge,'call'>; session: number; timeline: Timeline; revision?:number; enabled: boolean; recording: boolean; playing?:boolean; read?:boolean;
  show: (target: AutomationTarget) => void; onFailure: (error: unknown) => void; children: ReactNode;
}) {
  const now = useRef({ session, timeline, enabled, recording, playing, onFailure }); now.current = { session, timeline, enabled, recording, playing, onFailure };
  const [live, setLive] = useState<AutomationTouchControl['live']>(null);
  const [lives, setLives] = useState<AutomationTouchControl['lives']>([]);
  const held=useRef(new Map<string,{value:number;released:number|null}>());
  const [,redraw]=useState(0);
  const queues=useMemo(()=>new Map<string,TouchQueue>(),[bridge,session,timeline.id]);
  const primary=useRef<string|null>(null);
  const targetId=(target:AutomationTarget)=>`${ownerKey(target.of)}/${target.path}`;
  const end = () => { queues.forEach(q=>q.end()); for(const value of held.current.values())if(value.released===null)value.released=performance.now();setLive(null); setLives([]); };
  useEffect(() => { window.addEventListener('blur', end); return () => { window.removeEventListener('blur', end); queues.forEach(q=>q.end()); }; }, [queues]);
  const preview=timeline.audio?.record?.preview??false;
  useEffect(() => { if (!enabled || !recording || (!playing&&!preview)) end(); }, [enabled,recording,playing,preview]);
  useEffect(()=>{end();},[revision,queues]);
  useEffect(()=>{if(!enabled||!recording||read||(!playing&&!preview)){held.current.clear();redraw(n=>n+1);}},[enabled,recording,playing,preview,read,revision,queues]);
  useEffect(()=>{if(!enabled||!recording)return;const timer=setInterval(()=>{if(held.current.size)redraw(n=>n+1);},16);return()=>clearInterval(timer);},[enabled,recording]);
  const eligible=(target:AutomationTarget,value:number)=>{
    const {timeline:t,enabled:on,recording:rec,playing:running}=now.current;
    if(!on||!rec||(!running&&!t.audio?.record?.preview)||!Number.isFinite(value)||t.audio?.record?.categories?.[categoryOfPath(target.path)]===false)return false;
    const of=target.of;
    const owner='track' in of?t.tracks.find(x=>x.id===of.track&&x.kind==='audio'):'bus' in of?t.buses?.find(x=>x.id===of.bus):t.vcas?.find(x=>x.id===of.vca);
    if(!owner||('locked' in owner&&owner.locked)||('automation_arm' in owner&&(owner.automation_arm==='safe'||owner.automation_arm==='off')))return false;
    return (t.audio?.record?.touch??'off')!=='off'||('automation_arm' in owner&&owner.automation_arm===true);
  };
  const startBatch=(values:{target:AutomationTarget;value:number}[])=>{
    values=values.filter(v=>eligible(v.target,v.value));
    if(!values.length)return false;
    end();
    for(const {target,value} of values){
      const id=targetId(target);let queue=queues.get(id);
      if(!queue){queue=new TouchQueue(timeline.id,message=>callEngine(bridge,'automation.touch',message,{expectedSession:session}),error=>{if(now.current.session===session&&now.current.timeline.id===timeline.id){held.current.delete(id);setLive(null);setLives(old=>old.filter(v=>targetId(v.target)!==id));now.current.onFailure(error);}});queues.set(id,queue);}
      queue.start(target,value);
      held.current.set(id,{value,released:null});
    }
    primary.current=targetId(values[0].target);setLive(values[0]);setLives(values);return true;
  };
  const changeBatch=(values:{target:AutomationTarget;value:number}[])=>{values=values.filter(v=>Number.isFinite(v.value)&&queues.get(targetId(v.target))?.active());for(const v of values){queues.get(targetId(v.target))!.change(v.value);held.current.set(targetId(v.target),{value:v.value,released:null});}if(values.length){setLive(values[0]);setLives(values);}};
  const start=(target:AutomationTarget,value:number)=>startBatch([{target,value}]);
  const control: AutomationTouchControl = { enabled, recording, trim: timeline.audio?.record?.mode === 'trim', preview: timeline.audio?.record?.preview ?? false, active: target => target?Boolean(queues.get(targetId(target))?.active()):[...queues.values()].some(q=>q.active()), start, startBatch, changeBatch,
    instantBatch: values=>{if(!startBatch(values))return false;end();return true;},
    change: value => { if(!Number.isFinite(value)||!primary.current||!queues.get(primary.current)?.active())return;queues.get(primary.current)!.change(value);held.current.set(primary.current,{value,released:null});setLive(old => old ? {...old,value} : null);setLives(old=>old.map(v=>targetId(v.target)===primary.current?{...v,value}:v)); }, end,
    instant: (target,value) => { if (!start(target,value)) return false; end(); return true; }, show, live, lives,
    status:target=>enabled&&recording&&held.current.has(targetId(target))?(timeline.audio?.record?.preview?'preview':'write'):null,
    reading:(target,baseline,parameter)=>{
      const id=targetId(target),value=held.current.get(id);if(!value||!enabled||!recording)return baseline;
      const record=timeline.audio?.record, snap=record?.touch==='snap'||record?.touch==='snap_latch'&&target.path==='fader_db';
      if(!record?.preview&&snap&&value.released!==null){const configured=Number(localStorage.getItem('fairlight.glide_ms')??250),duration=Math.max(10,Number.isFinite(configured)?configured:250),progress=parameter?.kind==='switch'?1:Math.min(1,(performance.now()-value.released)/duration);if(progress===1){held.current.delete(id);return baseline;}return parameter?.scale==='log'&&value.value>0&&baseline>0?Math.exp(Math.log(value.value)*(1-progress)+Math.log(baseline)*progress):value.value+(baseline-value.value)*progress;}
      return value.value;
    } };
  return <Context.Provider value={control}>{children}</Context.Provider>;
}
export const sameTarget = (a: AutomationTarget, b: AutomationTarget) => ownerKey(a.of) === ownerKey(b.of) && a.path === b.path;
