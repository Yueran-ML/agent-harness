import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

function peer() {
  const child = spawn(process.execPath, [path.resolve('electron/mock-service.mjs')], { cwd: process.cwd(), stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  const pending = new Map<number, (message: any) => void>();
  let next = 0;
  let stderr = '';
  child.stderr.on('data', (data) => { stderr += data.toString(); });
  createInterface({ input: child.stdout, crlfDelay: Infinity }).on('line', (line) => {
    const message = JSON.parse(line);
    if (!message.method) pending.get(message.id)?.(message);
  });
  const exited = new Promise<number | null>((resolve) => child.once('exit', resolve));
  return {
    child, exited, stderr: () => stderr,
    request(method: string, params?: Record<string, unknown>) {
      const id = ++next;
      return new Promise<any>((resolve) => {
        pending.set(id, resolve);
        child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\r\n`);
      });
    },
  };
}

describe('standalone mock stdio process', () => {
  it('accepts CRLF, serves complete recorded history, and exits after shutdown', async () => {
    const process = peer();
    try {
      expect((await process.request('initialize', { protocol: 1 })).result.protocol).toBe(1);
      expect((await process.request('session/open', { session_id: 'recorded-deepseek-fix' })).result.events).toBe(31);
      const result = await process.request('session/events', { from: 0 });
      expect(result.result.events).toHaveLength(31);
      expect(result.result.events.at(-1).data.reason).toBe('completed');
      expect((await process.request('shutdown')).result).toEqual({});
      expect(await process.exited).toBe(0);
      expect(process.stderr()).toBe('');
    } finally { process.child.kill(); }
  });
  it('exits gracefully when its owner closes stdin', async () => {
    const process = peer();
    try {
      await process.request('initialize', { protocol: 1 });
      await process.request('session/open');
      process.child.stdin.end();
      expect(await process.exited).toBe(0);
      expect(process.stderr()).toBe('');
    } finally { process.child.kill(); }
  });
});
