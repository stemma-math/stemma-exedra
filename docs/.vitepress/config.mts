import { defineConfig } from "vitepress";

export default defineConfig({
  // Published at https://stemma-math.github.io/stemma-exedra/ with each release.
  base: process.env.DOCS_BASE ?? "/",
  // Release notes are read by the release workflow, not published here.
  srcExclude: ["releases/**"],
  title: "Stemma",
  description: "Where a research group does mathematics by talking.",
  cleanUrls: true,
  lastUpdated: false,
  markdown: { math: false },
  themeConfig: {
    nav: [
      { text: "Get started", link: "/start/" },
      { text: "Guide", link: "/guide/forum" },
      { text: "How it works", link: "/inside/" },
      { text: "Releases", link: "https://github.com/stemma-math/stemma-exedra/releases" },
    ],
    sidebar: [
      {
        text: "Get started",
        items: [
          { text: "Quick start", link: "/start/" },
          { text: "Install", link: "/start/install" },
          { text: "Set up your computer", link: "/start/setup" },
          { text: "First steps in a group", link: "/start/first-steps" },
        ],
      },
      {
        text: "Guide",
        items: [
          { text: "Groups and roles", link: "/guide/groups" },
          { text: "The forum", link: "/guide/forum" },
          { text: "Your agents", link: "/guide/agents" },
          { text: "The library and papers", link: "/guide/library-papers" },
          { text: "The workshop", link: "/guide/workshop" },
          { text: "Writing recipes", link: "/guide/recipes" },
          { text: "This machine's settings", link: "/guide/settings" },
        ],
      },
      {
        text: "Running a hub",
        items: [{ text: "The hub and its forge", link: "/hosting/hub" }],
      },
      {
        text: "How it works",
        items: [
          { text: "Architecture", link: "/inside/" },
          { text: "Tokens and trust", link: "/inside/trust" },
          { text: "The images", link: "/inside/images" },
          { text: "How an agent runs", link: "/inside/agents" },
          { text: "Builds", link: "/inside/builds" },
          { text: "Workshop processes", link: "/inside/processes" },
          { text: "The orchestrators", link: "/inside/orchestrators" },
          { text: "Working on Stemma", link: "/inside/development" },
        ],
      },
    ],
    socialLinks: [{ icon: "github", link: "https://github.com/stemma-math/stemma-exedra" }],
    search: { provider: "local" },
    outline: { level: [2, 3] },
    footer: { message: "Stemma is early: it changes often, and breaks." },
  },
});
