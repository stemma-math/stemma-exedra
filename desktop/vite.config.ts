import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [svelte()],
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  clearScreen: false,
});
