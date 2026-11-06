// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useSyncExternalStore } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { MigrationBanner } from '../src/studio/MigrationBanner';
import { Migrations } from '../src/studio/migration';
import { Words } from '../src/app/locale';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import { likeABrowser } from './mounted';

beforeEach(likeABrowser);
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const settle = () => act(async () => { for (let n=0;n<10;n++) await Promise.resolve(); });
const report = {source:'F:/work/Old Film',timeline:'t3',dropped:['duck: omitted'],existing:true,duck_pending:true};

it.each([en,zhCN,ja])('offers the translated question only for unanswered duck receipts', words => {
  const view=render(<Words.Provider value={words}><MigrationBanner migrated={{session:1,done:[report],failed:[],error:null}} timelines={[]} onDismiss={()=>{}} onAnswer={async()=>{}}/></Words.Provider>);
  expect(view.container.textContent).toContain(words.hint.studio.migration.duck);
  expect(screen.getByRole('button',{name:words.label.studio.migration.duckYes})).toBeTruthy();
  expect(view.container.querySelector('[data-test="migration-done"]')).toBeNull();
  view.rerender(<MigrationBanner migrated={{session:1,done:[{...report,duck_prompted:true},{source:'old',timeline:'t4',dropped:[]}],failed:[],error:null}} timelines={[]} onDismiss={()=>{}} onAnswer={async()=>{}}/>);
  expect(view.container.querySelector('[data-duck-timeline]')).toBeNull();
});

it('blocks duplicate answers, keeps failure retryable, and removes a successful prompt from shared state',async()=>{
  let hear:(event:any)=>void=()=>{};
  const store=new Migrations({onEvent:listener=>{hear=listener;return()=>{};}});
  hear({kind:'migrated',session:1,migrations:[report],error:null});
  let finish:()=>void=()=>{};let fail=true;
  const answer=vi.fn(async (_id:string,_accept:boolean)=>{await new Promise<void>(resolve=>{finish=resolve;});if(fail)throw new Error('stale project');store.answered(1,'t3');});
  function Host(){const migrated=useSyncExternalStore(store.subscribe,store.getSnapshot);return migrated?<MigrationBanner migrated={migrated} timelines={[]} onDismiss={()=>store.dismiss()} onAnswer={answer}/>:null;}
  const view=render(<Host/>);
  fireEvent.click(screen.getByRole('button',{name:'Add ducking'}));expect(screen.getByRole('button',{name:'No thanks'})).toHaveProperty('disabled',true);
  fireEvent.click(screen.getByRole('button',{name:'Applying…'}));expect(answer).toHaveBeenCalledTimes(1);
  finish();await settle();expect(screen.getByRole('alert').textContent).toContain('Stale project');
  fail=false;fireEvent.click(screen.getByRole('button',{name:'No thanks'}));expect(answer).toHaveBeenLastCalledWith('t3',false);finish();await settle();
  expect(view.container.querySelector('[data-duck-timeline]')).toBeNull();
  act(()=>hear({kind:'migrated',session:2,migrations:[report],error:null}));
  act(()=>store.answered(1,'t3'));expect(view.container.querySelector('[data-duck-timeline]')).not.toBeNull();
});
