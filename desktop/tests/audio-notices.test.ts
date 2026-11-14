import { mkdtemp, readFile, writeFile, rm, mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { writeCrateNotices } from '../scripts/engine-notices.mjs';

it('ships built-in voice-model provenance beside the unchanged original crate license', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'audio-notices-'));
  try {
    const source = path.join(root, 'source'), out = path.join(root, 'out'); await mkdir(source);
    await writeFile(path.join(source, 'COPYING'), 'Original upstream license\n');
    const table = await writeCrateNotices({out,crates:[{name:'nnnoiseless',version:'0.5.2',license:'BSD-3-Clause',folder:source,files:['COPYING'],repository:'https://github.com/jneem/nnnoiseless'}]});
    const folder=path.join(out,'licenses','crates','nnnoiseless-0.5.2');
    expect(await readFile(path.join(folder,'COPYING'),'utf8')).toBe('Original upstream license\n');
    expect(await readFile(path.join(folder,'MODEL-NOTICE.txt'),'utf8')).toContain('weights.rnn');
    expect(table).toContain('MODEL-NOTICE.txt');expect(table).toContain('COPYING');
  } finally { await rm(root,{recursive:true,force:true}); }
});
