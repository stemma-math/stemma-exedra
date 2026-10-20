import { readdir, readFile } from "node:fs/promises";

async function check(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = `${dir}/${entry.name}`;
    if (entry.isDirectory()) await check(path);
    else if (path.endsWith(".js")) {
      const source = await readFile(path, "utf8");
      if (/__TAURI|@tauri-apps|open-settings|Sign in to your hub|Prepare the image/.test(source)) {
        throw new Error(`Desktop code in browser bundle: ${path}`);
      }
    }
  }
}
await check("build");
console.log("Browser bundle contains no desktop UI or Tauri modules.");
