/// <reference types="vitest/config" />
import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// The hub the development server sends API requests to.
const hub = process.env.STEMMA_HUB ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [
    sveltekit({
      compilerOptions: {
        runes: ({ filename }) =>
          filename.split(/[/\\]/).includes("node_modules") ? undefined : true,
      },
      // A single-page app: the hub serves `200.html` for every page.
      adapter: adapter({ fallback: "200.html" }),
    }),
  ],
  server: {
    proxy: {
      "/api": { target: hub, changeOrigin: false },
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
