import adapter from "@sveltejs/adapter-static";
import { readFile, writeFile } from "node:fs/promises";
import type { Adapter } from "@sveltejs/kit";

/** External bootstrap keeps the bundled page at script-src 'self'. */
export function desktopAdapter(): Adapter {
  const staticAdapter = adapter({
    fallback: "index.html",
    pages: "../desktop/dist",
    assets: "../desktop/dist",
  });
  return {
    name: "stemma-desktop",
    async adapt(builder) {
      await staticAdapter.adapt(builder);
      const path = "../desktop/dist/index.html";
      const html = await readFile(path, "utf8");
      const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
      if (scripts.length !== 1) throw new Error("Expected one SvelteKit bootstrap");
      await writeFile("../desktop/dist/_app/bootstrap.js", scripts[0][1]);
      await writeFile(
        path,
        html.replace(scripts[0][0], '<script src="/_app/bootstrap.js" defer></script>'),
      );
    },
  };
}
