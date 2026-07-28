import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const directory = dirname(fileURLToPath(import.meta.url));
const port = Number.parseInt(process.argv[2] || "3456", 10);
const host = "127.0.0.1";
const maxBodyBytes = 1024 * 1024;

const initialContacts = () => [
  { id: 1, name: "Jane Doe", email: "jane@example.com", address: "1 Old St" },
  { id: 2, name: "John Smith", email: "john@example.com", address: "2 Old St" },
  { id: 3, name: "Alice Brown", email: "alice@example.com", address: "3 Old St" },
  { id: 4, name: "Bob Wilson", email: "bob@example.com", address: "4 Old St" },
  { id: 5, name: "Carol Davis", email: "carol@example.com", address: "5 Old St" },
];

let contacts = initialContacts();
let actionLog = [];

function send(res, status, contentType, body) {
  res.writeHead(status, {
    "Content-Type": contentType,
    "Cache-Control": "no-store",
  });
  res.end(body);
}

async function serveFile(res, path, contentType) {
  try {
    send(res, 200, contentType, await readFile(path));
  } catch (error) {
    send(res, 404, "text/plain; charset=utf-8", `not found: ${error.message}`);
  }
}

function readBody(req) {
  return new Promise((resolveBody, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (chunk) => {
      size += chunk.length;
      if (size > maxBodyBytes) {
        reject(new Error(`request body exceeds ${maxBodyBytes} bytes`));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => resolveBody(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

async function readJson(req) {
  return JSON.parse(await readBody(req));
}

async function handle(req, res) {
  const url = new URL(req.url, `http://${host}:${port}`);

  if (req.method === "GET" && url.pathname === "/") {
    return serveFile(res, resolve(directory, "public/index.html"), "text/html; charset=utf-8");
  }
  if (req.method === "GET" && url.pathname === "/contact.html") {
    return serveFile(res, resolve(directory, "public/contact.html"), "text/html; charset=utf-8");
  }
  if (req.method === "GET" && url.pathname === "/runtime-lab.html") {
    return serveFile(
      res,
      resolve(directory, "public/runtime-lab.html"),
      "text/html; charset=utf-8",
    );
  }
  if (req.method === "GET" && url.pathname === "/extension/runtime.mjs") {
    return serveFile(
      res,
      resolve(directory, "../../extension/runtime.mjs"),
      "text/javascript; charset=utf-8",
    );
  }
  if (req.method === "GET" && url.pathname === "/extension/wasm/coordinates.wasm") {
    return serveFile(
      res,
      resolve(directory, "../../extension/wasm/coordinates.wasm"),
      "application/wasm",
    );
  }
  if (req.method === "GET" && url.pathname === "/contacts.json") {
    const query = (url.searchParams.get("q") || "").toLowerCase();
    const filtered = contacts.filter((contact) =>
      contact.name.toLowerCase().includes(query),
    );
    return send(res, 200, "application/json", JSON.stringify(filtered));
  }
  if (req.method === "GET" && url.pathname.startsWith("/contact/")) {
    const id = Number.parseInt(url.pathname.split("/")[2], 10);
    const contact = contacts.find((candidate) => candidate.id === id);
    return send(
      res,
      contact ? 200 : 404,
      "application/json",
      JSON.stringify(contact || { error: "contact not found" }),
    );
  }
  if (req.method === "POST" && url.pathname === "/contact/save") {
    const body = await readJson(req);
    const contact = contacts.find((candidate) => candidate.id === body.id);
    if (!contact || typeof body.address !== "string") {
      return send(res, 400, "application/json", JSON.stringify({ error: "invalid contact" }));
    }
    contact.address = body.address;
    return send(res, 200, "application/json", JSON.stringify({ ok: true }));
  }
  if (req.method === "POST" && url.pathname === "/action") {
    actionLog.push({ ...(await readJson(req)), ts: Date.now() });
    return send(res, 200, "text/plain; charset=utf-8", "ok");
  }
  if (req.method === "GET" && url.pathname === "/log") {
    return send(res, 200, "application/json", JSON.stringify(actionLog));
  }
  if (req.method === "POST" && url.pathname === "/reset") {
    actionLog = [];
    contacts = initialContacts();
    return send(res, 200, "text/plain; charset=utf-8", "ok");
  }
  if (req.method === "GET" && url.pathname === "/health") {
    return send(res, 200, "text/plain; charset=utf-8", "ok");
  }
  return send(res, 404, "text/plain; charset=utf-8", "not found");
}

const server = createServer((req, res) => {
  handle(req, res).catch((error) => {
    if (!res.headersSent) {
      send(res, 400, "application/json", JSON.stringify({ error: error.message }));
    } else {
      res.destroy();
    }
  });
});

server.listen(port, host, () => {
  process.stdout.write(`READY http://${host}:${port}\n`);
});
