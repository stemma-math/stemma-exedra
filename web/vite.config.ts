/// <reference types="vitest/config" />
import { readFileSync } from "node:fs";
import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";
import { desktopAdapter } from "./desktop-adapter.ts";

// The hub the development server sends API requests to.
const desktop = process.env.STEMMA_DESKTOP === "1";
const hub = process.env.STEMMA_HUB ?? "http://127.0.0.1:8080";

export default defineConfig({
  define: {
    __STEMMA_DESKTOP__: desktop,
    __STEMMA_VERSION__: JSON.stringify(
      readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8").match(
        /^version = "([^"]+)"/m,
      )?.[1] ?? "development",
    ),
  },
  plugins: [
    sveltekit({
      compilerOptions: {
        runes: ({ filename }) =>
          filename.split(/[/\\]/).includes("node_modules") ? undefined : true,
      },
      // A single-page app: the hub serves `200.html` for every page.
      adapter: desktop ? desktopAdapter() : adapter({ fallback: "200.html" }),
    }),
  ],
  server: {
    proxy: {
      "/api": { target: hub, changeOrigin: false },
    },
  },
  test: {
    // Markdown and transport tests use the browser build.
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
