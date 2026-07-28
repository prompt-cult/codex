function defaultImports() {
  return {
    env: {
      abort(_message, _file, line, column) {
        throw new Error(`AssemblyScript aborted at ${line}:${column}`);
      },
    },
  };
}

export function requireExports(instance, names) {
  for (const name of names) {
    if (typeof instance.exports[name] !== "function") {
      throw new Error(`WASM module is missing required function export "${name}"`);
    }
  }
  return instance;
}

export async function instantiateWasmBytes(bytes, imports = defaultImports()) {
  const source =
    bytes instanceof ArrayBuffer
      ? bytes
      : bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  const result = await WebAssembly.instantiate(source, imports);
  return result.instance;
}

export async function instantiateWasmUrl(url, imports = defaultImports(), fetchImpl = fetch) {
  const response = await fetchImpl(url);
  if (!response.ok) {
    throw new Error(`cannot load WASM from ${url}: HTTP ${response.status}`);
  }
  return instantiateWasmBytes(await response.arrayBuffer(), imports);
}
