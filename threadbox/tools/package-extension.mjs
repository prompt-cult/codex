import { copyFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, "guest/assembly/coordinates.wasm");
const destination = resolve(root, "extension/wasm/coordinates.wasm");

await mkdir(dirname(destination), { recursive: true });
await copyFile(source, destination);
process.stdout.write(`${destination}\n`);
