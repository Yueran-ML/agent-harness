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
      // The pages: the application, the window that takes a key, the ones that take the app of
      // Feishu, the bot of WeCom and the bot of QQ, and the window of the pet.
      input: { index: 'index.html', key: 'key.html', feishu: 'feishu.html', wecom: 'wecom.html', qq: 'qq.html', pet: 'pet.html' },
      output: {
        entryFileNames: 'assets/[name].js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name][extname]',
      },
    },
  },
});
