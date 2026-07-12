import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri開発時のモバイル向けホスト設定(通常はundefined)
const tauriDevHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  // `tauri dev` のログを消さない
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: tauriDevHost || false,
    hmr: tauriDevHost
      ? { protocol: "ws", host: tauriDevHost, port: 1421 }
      : undefined,
    watch: {
      // Rust側の変更でViteが再起動しないようにする
      ignored: ["**/src-tauri/**"],
    },
  },
});
