import { defineConfig } from "vite";

// https://vitejs.dev/config/
export default defineConfig({
  // Tauri expects a fixed dev server port for devUrl in tauri.conf.json
  server: {
    port: 1420,
    strictPort: true,
  },
  // Prevent Vite from obscuring Rust errors
  clearScreen: false,
  // Tauri works with environment variables
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Tauri uses Chromium on Windows, WebKit on macOS/Linux, and Android WebView
    target: process.env.TAURI_PLATFORM === "android" ? "es2021" : "esnext",
    // Don't minify for debug builds
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    // Produce sourcemaps for debugging
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
