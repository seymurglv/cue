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
    // The Drop, the card and the main window ship; the dev server still serves ui/preview.html for side-by-side checks.
    rolldownOptions: {
      input: { drop: here('./ui/drop.html'), card: here('./ui/card.html'), window: here('./ui/window.html') },
    },
  },
  test: { environment: 'jsdom', include: ['test/**/*.test.ts'], setupFiles: ['test/setup.ts'] },
});
