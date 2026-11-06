import { useSyncExternalStore } from 'react';
import type { Effect, InsertsOf, SourceRef } from '../engine';
function clipboard<T>() {
  let value: T | null = null; const listeners = new Set<() => void>();
  return { set(next: T) { value = next; listeners.forEach(fn => fn()); }, use() { return useSyncExternalStore(fn => {listeners.add(fn);return()=>{listeners.delete(fn);};},()=>value); } };
}
export const effectClipboard = clipboard<Effect>();
export const chainClipboard = clipboard<{bridge:object;session:number;timeline:string;of:InsertsOf}>();
export const stripClipboard = clipboard<{bridge:object;session:number;timeline:string;target:SourceRef}>();
