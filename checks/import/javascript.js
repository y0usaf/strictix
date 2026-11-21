import { readFile } from "node:fs/promises";

const names = ["a", "b"];

async function load(path) {
  const text = await readFile(path, "utf8");
  const name = names.find((n) => text.includes(n));
  if (name === undefined) {
    return null;
  }
  return { name, size: text.length };
}

console.log(await load(process.argv.at(2)));
