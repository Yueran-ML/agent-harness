import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const desktop = fileURLToPath(new URL('..', import.meta.url));
const read = (file: string) => readFileSync(path.join(desktop, file), 'utf8');

/** What a module imports, by the places that it names. */
function importsOf(source: string): string[] {
  return [...source.matchAll(/^import\s[^'"]*['"]([^'"]+)['"];?$/gm)].map(found => found[1]);
}

/** A policy of content security, by its directives. */
function policyOf(page: string): Map<string, string[]> {
  const content = /http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(page)?.[1] ?? '';
  return new Map(content.split(';').map(part => part.trim().split(/\s+/)).filter(words => words[0])
    .map(([name, ...values]) => [name, values]));
}

describe('the window for the key', () => {
  const files = readdirSync(path.join(desktop, 'src/key')).filter(file => /\.tsx?$/.test(file));

  it('is made of its own files', () => {
    expect(files.sort()).toEqual(['KeyApp.tsx', 'bridge.d.ts', 'form.ts', 'main.tsx']);
  });

  it('takes nothing from the part of the application that shows a conversation', () => {
    // The words, the look and the themes are shared. Nothing that reads the log of a task,
    // draws what a model wrote or talks to the service process is.
    const shared = new Set([
      '../app/locale', '../i18n', '../size', '../themes', '../ui/tokens.css', '../ui/base.css',
      './KeyApp', './form', './key.css',
    ]);
    const packages = new Set([
      'react', 'react-dom/client', 'lucide-react', '@fontsource-variable/inter', '@fontsource-variable/jetbrains-mono',
    ]);
    for (const file of files) {
      const named = importsOf(read(`src/key/${file}`));
      expect(named.filter(place => !shared.has(place) && !packages.has(place)), file).toEqual([]);
    }
    // And what it shares takes nothing from that part either.
    expect(importsOf(read('src/app/locale.ts')).sort()).toEqual(['../i18n', '../messages/en', 'react']);
    expect(importsOf(read('src/i18n.ts')).sort()).toEqual(['./messages/en', './messages/ja', './messages/zh-CN']);
    expect(importsOf(read('src/themes.ts'))).toEqual([]);
    expect(importsOf(read('src/size.ts'))).toEqual([]);
  });

  it('never reaches for the bridge of the main window', () => {
    for (const file of files) {
      const source = read(`src/key/${file}`);
      expect(source.replaceAll('window.columbinaKey', ''), file).not.toMatch(/window\.columbina\b/);
    }
  });

  it('has a page that loads its own script and nothing from elsewhere', () => {
    const page = read('key.html');
    const policy = policyOf(page);
    expect(policy.get('default-src')).toEqual(["'self'"]);
    expect(policy.get('script-src')).toEqual(["'self'"]);
    // It talks to nobody but the shell, which is not a connection of the page.
    expect(policy.get('connect-src')).toEqual(["'none'"]);
    expect(policy.get('object-src')).toEqual(["'none'"]);
    expect(policy.get('form-action')).toEqual(["'none'"]);
    expect([...page.matchAll(/<script[^>]*src="([^"]+)"/g)].map(found => found[1])).toEqual(['/src/key/main.tsx']);
    expect(page).not.toMatch(/<script(?![^>]*\bsrc=)/);
  });

  it('has a bridge that offers six things and nothing of the main window', () => {
    const preload = read('electron/key-preload.cjs');
    expect([...preload.matchAll(/ipcRenderer\.(?:invoke|send|on)\('([^']+)'/g)].map(found => found[1]).sort())
      .toEqual([
        'columbina-key:check', 'columbina-key:close', 'columbina-key:context', 'columbina-key:forget',
        'columbina-key:open-page', 'columbina-key:store',
      ]);
    expect(preload).toContain("exposeInMainWorld('columbinaKey'");
  });

  it('keeps what is typed for a key out of sight and out of what a browser remembers', () => {
    const source = read('src/key/KeyApp.tsx');
    const field = /<input ref=\{keyField\}[^>]*>/.exec(source.replace(/\s+/g, ' '))?.[0] ?? '';
    expect(field).toContain('type="password"');
    expect(field).toContain('autoComplete="off"');
    expect(field).toContain('spellCheck={false}');
    // It is kept nowhere: not in the storage of the page, and not in its address.
    expect(source).not.toMatch(/localStorage|sessionStorage|indexedDB|location\.|history\./);
  });
});

describe('the window for the app of Feishu', () => {
  const files = readdirSync(path.join(desktop, 'src/feishu')).filter(file => /\.(tsx?|css)$/.test(file));

  it('is made of its own files', () => {
    expect(files.sort()).toEqual(['FeishuApp.tsx', 'bridge.d.ts', 'feishu.css', 'form.ts', 'main.tsx']);
  });

  it('takes nothing from the part of the application that shows a conversation', () => {
    // It shares what the window for the key shares, and the look and the plain helpers of that window.
    const shared = new Set([
      '../app/locale', '../i18n', '../size', '../themes', '../ui/tokens.css', '../ui/base.css',
      '../key/key.css', '../key/form', './FeishuApp', './form', './feishu.css',
    ]);
    const packages = new Set([
      'react', 'react-dom/client', 'lucide-react', '@fontsource-variable/inter', '@fontsource-variable/jetbrains-mono',
    ]);
    for (const file of files.filter(name => !name.endsWith('.css'))) {
      const named = importsOf(read(`src/feishu/${file}`));
      expect(named.filter(place => !shared.has(place) && !packages.has(place)), file).toEqual([]);
    }
  });

  it('never reaches for the bridge of the main window or of the window for the key', () => {
    for (const file of files) {
      const source = read(`src/feishu/${file}`);
      expect(source.replaceAll('window.columbinaFeishu', ''), file).not.toMatch(/window\.columbina(Key)?\b/);
    }
  });

  it('has a page that loads its own script and nothing from elsewhere', () => {
    const page = read('feishu.html');
    const policy = policyOf(page);
    expect(policy).toEqual(policyOf(read('key.html')));
    expect(policy.get('connect-src')).toEqual(["'none'"]);
    expect([...page.matchAll(/<script[^>]*src="([^"]+)"/g)].map(found => found[1])).toEqual(['/src/feishu/main.tsx']);
    expect(page).not.toMatch(/<script(?![^>]*\bsrc=)/);
  });

  it('has a bridge that offers four things and nothing of the other windows', () => {
    const preload = read('electron/feishu-preload.cjs');
    expect([...preload.matchAll(/ipcRenderer\.(?:invoke|send|on)\('([^']+)'/g)].map(found => found[1]).sort())
      .toEqual(['columbina-feishu:close', 'columbina-feishu:context', 'columbina-feishu:open-page', 'columbina-feishu:store']);
    expect(preload).toContain("exposeInMainWorld('columbinaFeishu'");
  });

  it('keeps what is typed for the App Secret out of sight and out of what a browser remembers', () => {
    const source = read('src/feishu/FeishuApp.tsx');
    const field = /<input ref=\{secretField\}[^>]*>/.exec(source.replace(/\s+/g, ' '))?.[0] ?? '';
    expect(field).toContain('type="password"');
    expect(field).toContain('autoComplete="off"');
    expect(field).toContain('spellCheck={false}');
    expect(field).not.toContain('value=');
    expect(source).not.toMatch(/localStorage|sessionStorage|indexedDB|location\.|history\./);
  });
});

describe('the main window and the key', () => {
  it('has no way to the calls of the window for the key, nor of the window for the app of Feishu', () => {
    expect(read('electron/preload.cjs')).not.toContain('columbina-key:');
    expect(read('electron/preload.cjs')).not.toContain('columbina-feishu:');
  });

  it('is handed the services without a key, and only whether something was stored or deleted', () => {
    const bridge = read('src/bridge.d.ts');
    expect(bridge).toContain('setUpService(asked: { service: string | null; language: string }): Promise<{ changed: boolean }>;');
    for (const file of ['src/useWorkspace.ts', 'src/app/Settings.tsx', 'src/app/App.tsx']) {
      expect(read(file), file).not.toMatch(/api_key|apiKey/);
    }
    // And whether an app of Feishu was stored, and nothing of its App Secret.
    expect(bridge).toContain('setUpFeishu(asked: { language: string }): Promise<{ changed: boolean }>;');
    for (const file of ['src/useFeishu.ts', 'src/app/feishu.ts', 'src/app/FeishuAssistant.tsx']) {
      // What the code names, and not what its comments say of the App Secret.
      expect(read(file), file).not.toMatch(/app_secret|appSecret|\bsecret\s*[:=(]|\.secret\b/);
    }
  });
});

describe('what the packed application holds of the shell', () => {
  it('is every file that the shell names beside itself', () => {
    const shell = read('electron/main.cjs');
    const named = [...shell.matchAll(/path\.join\(__dirname, '([^']+)'\)/g)].map(found => found[1]);
    expect(new Set(named)).toEqual(new Set(['icon.ico', 'preload.cjs', 'key-preload.cjs', 'feishu-preload.cjs', 'mock-service.mjs']));
    const packer = read('electron/package.mjs');
    for (const file of ['main.cjs', ...named]) expect(packer, file).toContain(`'/electron/${file}'`);
  });

  it('is the three pages of the build', () => {
    const config = read('vite.config.ts');
    expect(config).toContain("input: { index: 'index.html', key: 'key.html', feishu: 'feishu.html' }");
    const shell = read('electron/main.cjs');
    expect(shell).toContain("path.resolve(__dirname, '../dist/index.html')");
    expect(shell).toContain("path.resolve(__dirname, '../dist/key.html')");
    expect(shell).toContain("path.resolve(__dirname, '../dist/feishu.html')");
  });
});
