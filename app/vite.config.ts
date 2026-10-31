import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  root: 'ui',
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    // The Drop, the card, the main window and the welcome window ship; the dev server still serves ui/preview.html for
    // side-by-side checks.
    rolldownOptions: {
      input: {
        drop: here('./ui/drop.html'),
        card: here('./ui/card.html'),
        window: here('./ui/window.html'),
        welcome: here('./ui/welcome.html'),
      },
    },
  },
  test: { environment: 'jsdom', include: ['test/**/*.test.ts'], setupFiles: ['test/setup.ts'] },
});
