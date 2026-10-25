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
    expect(files.sort()).toEqual(['FeishuApp.tsx', 'bridge.d.ts', 'form.ts', 'main.tsx']);
  });

  it('takes nothing from the part of the application that shows a conversation', () => {
    // It shares what the window for the key shares, and the look and the plain helpers of that window.
    const shared = new Set([
      '../app/locale', '../i18n', '../size', '../themes', '../ui/tokens.css', '../ui/base.css',
      '../key/key.css', '../key/steps.css', '../key/form', './FeishuApp', './form',
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

describe('the window for the bot of WeCom', () => {
  const files = readdirSync(path.join(desktop, 'src/wecom')).filter(file => /\.(tsx?|css)$/.test(file));

  it('is made of its own files', () => {
    expect(files.sort()).toEqual(['WecomApp.tsx', 'bridge.d.ts', 'form.ts', 'main.tsx']);
  });

  it('takes nothing from the part of the application that shows a conversation, nor from the window for Feishu', () => {
    const shared = new Set([
      '../app/locale', '../i18n', '../size', '../themes', '../ui/tokens.css', '../ui/base.css',
      '../key/key.css', '../key/steps.css', '../key/form', './WecomApp', './form',
    ]);
    const packages = new Set([
      'react', 'react-dom/client', 'lucide-react', '@fontsource-variable/inter', '@fontsource-variable/jetbrains-mono',
    ]);
    for (const file of files) {
      const named = importsOf(read(`src/wecom/${file}`));
      expect(named.filter(place => !shared.has(place) && !packages.has(place)), file).toEqual([]);
    }
  });

  it('never reaches for the bridge of another window', () => {
    for (const file of files) {
      const source = read(`src/wecom/${file}`);
      expect(source.replaceAll('window.columbinaWecom', ''), file).not.toMatch(/window\.columbina(Key|Feishu)?\b/);
    }
  });

  it('has a page that loads its own script and nothing from elsewhere', () => {
    const page = read('wecom.html');
    expect(policyOf(page)).toEqual(policyOf(read('key.html')));
    expect([...page.matchAll(/<script[^>]*src="([^"]+)"/g)].map(found => found[1])).toEqual(['/src/wecom/main.tsx']);
    expect(page).not.toMatch(/<script(?![^>]*\bsrc=)/);
  });

  it('has a bridge that offers four things and nothing of the other windows', () => {
    const preload = read('electron/wecom-preload.cjs');
    expect([...preload.matchAll(/ipcRenderer\.(?:invoke|send|on)\('([^']+)'/g)].map(found => found[1]).sort())
      .toEqual(['columbina-wecom:close', 'columbina-wecom:context', 'columbina-wecom:open-page', 'columbina-wecom:store']);
    expect(preload).toContain("exposeInMainWorld('columbinaWecom'");
  });

  it('keeps what is typed for the Secret out of sight and out of what a browser remembers', () => {
    const source = read('src/wecom/WecomApp.tsx');
    const field = /<input ref=\{secretField\}[^>]*>/.exec(source.replace(/\s+/g, ' '))?.[0] ?? '';
    expect(field).toContain('type="password"');
    expect(field).toContain('autoComplete="off"');
    expect(field).toContain('spellCheck={false}');
    expect(field).not.toContain('value=');
    expect(source).not.toMatch(/localStorage|sessionStorage|indexedDB|location\.|history\./);
  });
});

describe('the window for the bot of QQ', () => {
  const files = readdirSync(path.join(desktop, 'src/qq')).filter(file => /\.(tsx?|css)$/.test(file));

  it('is made of its own files', () => {
    expect(files.sort()).toEqual(['QqApp.tsx', 'bridge.d.ts', 'form.ts', 'main.tsx']);
  });

  it('takes nothing from the part of the application that shows a conversation, nor from the windows for Feishu and WeCom', () => {
    const shared = new Set([
      '../app/locale', '../i18n', '../size', '../themes', '../ui/tokens.css', '../ui/base.css',
      '../key/key.css', '../key/steps.css', '../key/form', './QqApp', './form',
    ]);
    const packages = new Set([
      'react', 'react-dom/client', 'lucide-react', '@fontsource-variable/inter', '@fontsource-variable/jetbrains-mono',
    ]);
    for (const file of files) {
      const named = importsOf(read(`src/qq/${file}`));
      expect(named.filter(place => !shared.has(place) && !packages.has(place)), file).toEqual([]);
    }
  });

  it('never reaches for the bridge of another window', () => {
    for (const file of files) {
      const source = read(`src/qq/${file}`);
      expect(source.replaceAll('window.columbinaQq', ''), file).not.toMatch(/window\.columbina(Key|Feishu|Wecom)?\b/);
    }
  });

  it('has a page that loads its own script and nothing from elsewhere', () => {
    const page = read('qq.html');
    expect(policyOf(page)).toEqual(policyOf(read('key.html')));
    expect([...page.matchAll(/<script[^>]*src="([^"]+)"/g)].map(found => found[1])).toEqual(['/src/qq/main.tsx']);
    expect(page).not.toMatch(/<script(?![^>]*\bsrc=)/);
  });

  it('has a bridge that offers four things and nothing of the other windows', () => {
    const preload = read('electron/qq-preload.cjs');
    expect([...preload.matchAll(/ipcRenderer\.(?:invoke|send|on)\('([^']+)'/g)].map(found => found[1]).sort())
      .toEqual(['columbina-qq:close', 'columbina-qq:context', 'columbina-qq:open-page', 'columbina-qq:store']);
    expect(preload).toContain("exposeInMainWorld('columbinaQq'");
  });

  it('keeps what is typed for the AppSecret out of sight and out of what a browser remembers', () => {
    const source = read('src/qq/QqApp.tsx');
    const field = /<input ref=\{secretField\}[^>]*>/.exec(source.replace(/\s+/g, ' '))?.[0] ?? '';
    expect(field).toContain('type="password"');
    expect(field).toContain('autoComplete="off"');
    expect(field).toContain('spellCheck={false}');
    expect(field).not.toContain('value=');
    expect(source).not.toMatch(/localStorage|sessionStorage|indexedDB|location\.|history\./);
  });
});

describe('the main window and the key', () => {
  it('has no way to the calls of the window for the key, nor of the windows for the assistants', () => {
    expect(read('electron/preload.cjs')).not.toContain('columbina-key:');
    expect(read('electron/preload.cjs')).not.toContain('columbina-feishu:');
    expect(read('electron/preload.cjs')).not.toContain('columbina-wecom:');
    expect(read('electron/preload.cjs')).not.toContain('columbina-qq:');
  });

  it('is handed the services without a key, and only whether something was stored or deleted', () => {
    const bridge = read('src/bridge.d.ts');
    expect(bridge).toContain('setUpService(asked: { service: string | null; language: string }): Promise<{ changed: boolean }>;');
    for (const file of ['src/useWorkspace.ts', 'src/app/Settings.tsx', 'src/app/App.tsx']) {
      expect(read(file), file).not.toMatch(/api_key|apiKey/);
    }
    // And whether an app of Feishu or a bot of WeCom was stored, and nothing of its secret.
    expect(bridge).toContain('setUpFeishu(asked: { language: string }): Promise<{ changed: boolean }>;');
    expect(bridge).toContain('setUpWecom(asked: { language: string }): Promise<{ changed: boolean }>;');
    expect(bridge).toContain('setUpQq(asked: { language: string }): Promise<{ changed: boolean }>;');
    for (const file of ['src/useAssistant.ts', 'src/app/assistant.ts', 'src/app/AssistantPanel.tsx']) {
      // What the code names, and not what its comments say of the App Secret.
      expect(read(file), file).not.toMatch(/app_secret|appSecret|\bsecret\s*[:=(]|\.secret\b/);
    }
  });
});

describe('what the packed application holds of the shell', () => {
  it('is every file that the shell names beside itself', () => {
    const shell = read('electron/main.cjs');
    // A file of each assistant is named by the command of the assistant.
    const kinds = [...shell.matchAll(/^ {2}(\w+): assistantOf\('\1'/gm)].map(found => found[1]);
    expect(kinds).toEqual(['feishu', 'wecom', 'qq']);
    const named = [...shell.matchAll(/path\.join\(__dirname, (['`])([^'`]+)\1\)/g)]
      .flatMap(found => (found[2].includes('${kind}') ? kinds.map(kind => found[2].replace('${kind}', kind)) : [found[2]]));
    expect(new Set(named)).toEqual(new Set([
      'icon.ico', 'preload.cjs', 'key-preload.cjs', 'feishu-preload.cjs', 'wecom-preload.cjs', 'qq-preload.cjs',
      'pet-preload.cjs', 'mock-service.mjs',
    ]));
    const packer = read('electron/package.mjs');
    // The pets are a part of the shell of their own, which it requires beside itself.
    expect(shell).toContain("require('./pets.cjs')");
    for (const file of ['main.cjs', 'pets.cjs', ...named]) expect(packer, file).toContain(`'/electron/${file}'`);
  });

  it('is the six pages of the build', () => {
    const config = read('vite.config.ts');
    expect(config).toContain("input: { index: 'index.html', key: 'key.html', feishu: 'feishu.html', wecom: 'wecom.html', qq: 'qq.html', pet: 'pet.html' }");
    const shell = read('electron/main.cjs');
    expect(shell).toContain("path.resolve(__dirname, '../dist/index.html')");
    expect(shell).toContain("path.resolve(__dirname, '../dist/key.html')");
    expect(shell).toContain("path.resolve(__dirname, '../dist/pet.html')");
    // The pets that come with the application are in the build, from its public folder.
    expect(shell).toContain("path.resolve(__dirname, '../dist/pets')");
    // The page of each assistant has the name of its command.
    expect(shell).toContain('path.resolve(__dirname, `../dist/${kind}.html`)');
  });
});
