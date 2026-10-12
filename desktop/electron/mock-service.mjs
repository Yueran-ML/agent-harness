import { createInterface } from 'node:readline';
import { createReplayService } from '../src/replay.shared.js';

let ending = false;
const diagnostics = process.env.COLUMBINA_REPLAY_DIAGNOSTICS === '1';
const methods = new Map();
const finish = () => {
  if (ending) return;
  ending = true;
  service.close();
  process.stdout.write('', () => process.exit(0));
};
const service = createReplayService((message) => {
  process.stdout.write(`${JSON.stringify(message)}\n`);
  if (diagnostics && !message.method && methods.has(message.id)) {
    process.stderr.write(`[replay] ${methods.get(message.id)} ${message.error ? 'error' : 'ok'}\n`);
    methods.delete(message.id);
  }
}, { cwd: process.cwd(), onShutdown: finish });
const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', (line) => {
  try {
    const message = JSON.parse(line);
    if (diagnostics && message.method) methods.set(message.id, message.method);
    void service.receive(message).catch((error) => {
      process.stderr.write(`${error.stack || error}\n`);
      process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: message.id ?? null, error: { code: -32603, message: 'An internal error occurred in the replay service.' } })}\n`);
    });
  } catch {
    process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'This line is not valid JSON.' } })}\n`);
  }
});
input.on('close', finish);
process.on('SIGTERM', finish);
process.on('SIGINT', finish);
