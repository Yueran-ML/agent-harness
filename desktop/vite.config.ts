import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  base: './',
  server: { watch: { ignored: ['**/release/**', '**/dist/**'] } },
  build: {
    // Every asset is written as a file. Vite would put a small one into the
    // stylesheet as a `data:` address, and the page refuses a font that comes
    // that way: its policy is `font-src 'self'`.
    assetsInlineLimit: 0,
    rollupOptions: {
      // Four pages: the application, the window that takes a key, the one that takes the app of
      // Feishu, and the one that takes the bot of WeCom.
      input: { index: 'index.html', key: 'key.html', feishu: 'feishu.html', wecom: 'wecom.html', qq: 'qq.html' },
      output: {
        entryFileNames: 'assets/[name].js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name][extname]',
      },
    },
  },
});
