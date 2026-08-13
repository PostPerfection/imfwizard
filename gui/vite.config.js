import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    fs: {
      allow: [".."],
    },
  },
  // guikit sits outside gui/, so node resolution from its files never reaches
  // gui/node_modules. dedupe makes vite resolve these from the gui root instead.
  resolve: {
    dedupe: ["@tauri-apps/api", "@tauri-apps/plugin-dialog"],
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
