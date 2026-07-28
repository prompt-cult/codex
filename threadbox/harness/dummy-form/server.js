/// A throwaway CRM-like dummy form server, for the ThreadBox end-to-end
/// proof only. Not production code, not part of `guest/` or `rust/`,
/// and not subject to their zero-dependency rules — this is Node
/// `node:http` test scaffolding. See `harness/dummy-form/README.md`.
///
/// Usage: node server.js [port]
///
/// Reads `HAI_API_KEY` from the environment (the parent .env at
/// /Users/Shared/codex/.env exports it) so the browser proof page never
/// sees the key: the page POSTs same-origin to `/holo/chat` and this
/// server forwards to https://api.hcompany.ai/v1/chat/completions with
/// the bearer token attached. That also dodges the browser CORS
/// preflight that blocks a direct page→api.hcompany.ai call.
///
/// Per-session tracing: every page load allocates a time-prefixed UUID
/// (rendered in the page and logged at the server). The page POSTs each
/// traffic line to `/trace` and each scaled screenshot PNG to
/// `/trace/image`, so a run leaves a durable `logs/<session>/trace.jsonl`
/// plus timestamped PNGs in the same folder — for prompt tuning when a
/// localize call lands wrong.
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync, appendFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

const __dir = dirname(fileURLToPath(import.meta.url));
const PORT = parseInt(process.argv[2] || '3456', 10);
const LOGS_DIR = join(__dir, 'logs');

const initialContacts = () => [
  { id: 1, name: 'Jane Doe', email: 'jane@example.com', address: '1 Old St' },
  { id: 2, name: 'John Smith', email: 'john@example.com', address: '2 Old St' },
  { id: 3, name: 'Alice Brown', email: 'alice@example.com', address: '3 Old St' },
  { id: 4, name: 'Bob Wilson', email: 'bob@example.com', address: '4 Old St' },
  { id: 5, name: 'Carol Davis', email: 'carol@example.com', address: '5 Old St' },
];

let contacts = initialContacts();
let actionLog = [];

function readBody(req) {
  return new Promise((resolve) => {
    let body = '';
    req.on('data', (d) => (body += d));
    req.on('end', () => resolve(body));
  });
}

function readRawBody(req) {
  // For binary payloads (PNG uploads) collect Buffers, not strings.
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', (d) => chunks.push(d));
    req.on('end', () => resolve(Buffer.concat(chunks)));
  });
}

function serveFile(res, path, contentType) {
  res.writeHead(200, { 'Content-Type': contentType });
  res.end(readFileSync(join(__dir, path), 'utf8'));
}

/// A time-prefixed UUID so session folders sort chronologically and the
/// id is also grep-able in console output. Format: YYYYMMDDHHmmss-<uuid>.
function newSessionId() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, '0');
  const ts = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
  return `${ts}-${randomUUID()}`;
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (req.method === 'GET' && url.pathname === '/') {
    return serveFile(res, 'public/index.html', 'text/html');
  }
  if (req.method === 'GET' && url.pathname === '/contact.html') {
    return serveFile(res, 'public/contact.html', 'text/html');
  }
  if (req.method === 'GET' && url.pathname === '/holo.html') {
    return serveFile(res, 'public/holo.html', 'text/html');
  }
  // Serve the guest's compiled WASM so the holo.html host page can
  // fetch it cross-origin-free. The browser proof reads coordinate
  // algebra from this artifact.
  if (req.method === 'GET' && url.pathname === '/guest/assembly/holo.wasm') {
    const wasmPath = join(__dir, '..', '..', 'guest', 'assembly', 'holo.wasm');
    try {
      const bytes = readFileSync(wasmPath);
      res.writeHead(200, { 'Content-Type': 'application/wasm' });
      return res.end(bytes);
    } catch (e) {
      res.writeHead(404);
      return res.end('wasm not built; run `npx asc assembly/holo.ts -o assembly/holo.wasm --exportRuntime` in guest/');
    }
  }

  // ---- session allocation ----
  // Each page load asks for a fresh time-prefixed UUID; the server logs
  // the allocation and returns the id. The page renders it and includes
  // it in every subsequent /trace and /trace/image POST.
  if (req.method === 'POST' && url.pathname === '/trace/session') {
    const sessionId = newSessionId();
    const sessionDir = join(LOGS_DIR, sessionId);
    mkdirSync(sessionDir, { recursive: true });
    const entry = { ts: Date.now(), session: sessionId, event: 'session-allocated' };
    appendFileSync(join(sessionDir, 'trace.jsonl'), JSON.stringify(entry) + '\n');
    console.log(`[trace] session ${sessionId} allocated`);
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ session: sessionId }));
  }

  // ---- trace line append ----
  // Body: { session, kind, text }. Appended to logs/<session>/trace.jsonl.
  if (req.method === 'POST' && url.pathname === '/trace') {
    const body = JSON.parse(await readBody(req));
    const { session, kind, text } = body;
    if (!session) {
      res.writeHead(400);
      return res.end('missing session');
    }
    const sessionDir = join(LOGS_DIR, session);
    const entry = { ts: Date.now(), kind, text };
    appendFileSync(join(sessionDir, 'trace.jsonl'), JSON.stringify(entry) + '\n');
    res.writeHead(200);
    return res.end('ok');
  }

  // ---- image logging ----
  // The page POSTs a PNG (raw body) with query params ?session=...&name=...
  // The server writes logs/<session>/<timestamp>-<name>.png so a run
  // leaves a chronological set of exactly what Holo saw, for prompt
  // tuning when a localize lands wrong.
  if (req.method === 'POST' && url.pathname === '/trace/image') {
    const session = url.searchParams.get('session');
    const name = url.searchParams.get('name') || 'screenshot';
    if (!session) {
      res.writeHead(400);
      return res.end('missing session');
    }
    const sessionDir = join(LOGS_DIR, session);
    mkdirSync(sessionDir, { recursive: true });
    const d = new Date();
    const pad = (n) => String(n).padStart(2, '0');
    const fileTs = `${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}${String(d.getMilliseconds()).padStart(3, '0')}`;
    const filename = `${fileTs}-${name}.png`;
    const buf = await readRawBody(req);
    writeFileSync(join(sessionDir, filename), buf);
    const entry = { ts: Date.now(), kind: 'image', text: `saved ${filename} (${buf.length} bytes)` };
    appendFileSync(join(sessionDir, 'trace.jsonl'), JSON.stringify(entry) + '\n');
    console.log(`[trace] ${session} image ${filename} (${buf.length} bytes)`);
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ ok: true, filename }));
  }

  // ---- Holo proxy: same-origin POST /holo/chat -> HAI ----
  // The browser page cannot call api.hcompany.ai directly (CORS), so it
  // POSTs its JSON body here and this server attaches the bearer token
  // from the environment and forwards. The key never reaches the page.
  if (req.method === 'POST' && url.pathname === '/holo/chat') {
    const body = await readBody(req);
    const haiKey = process.env.HAI_API_KEY;
    if (!haiKey) {
      res.writeHead(500, { 'Content-Type': 'application/json' });
      return res.end(JSON.stringify({ error: 'HAI_API_KEY not set on server' }));
    }
    try {
      const upstream = await fetch('https://api.hcompany.ai/v1/chat/completions', {
        method: 'POST',
        headers: { Authorization: `Bearer ${haiKey}`, 'Content-Type': 'application/json' },
        body,
      });
      const text = await upstream.text();
      res.writeHead(upstream.status, { 'Content-Type': upstream.headers.get('content-type') || 'application/json' });
      return res.end(text);
    } catch (e) {
      res.writeHead(502, { 'Content-Type': 'application/json' });
      return res.end(JSON.stringify({ error: `upstream fetch failed: ${e.message}` }));
    }
  }
  if (req.method === 'GET' && url.pathname === '/contacts.json') {
    const q = (url.searchParams.get('q') || '').toLowerCase();
    const filtered = contacts.filter((c) => c.name.toLowerCase().includes(q));
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify(filtered));
  }
  if (req.method === 'GET' && url.pathname.startsWith('/contact/')) {
    const id = parseInt(url.pathname.split('/')[2], 10);
    const contact = contacts.find((c) => c.id === id) || null;
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify(contact));
  }
  if (req.method === 'POST' && url.pathname === '/contact/save') {
    const body = JSON.parse(await readBody(req));
    const contact = contacts.find((c) => c.id === body.id);
    if (contact) contact.address = body.address;
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ ok: true }));
  }
  if (req.method === 'POST' && url.pathname === '/action') {
    const entry = { ...JSON.parse(await readBody(req)), ts: Date.now() };
    actionLog.push(entry);
    res.writeHead(200);
    return res.end('ok');
  }
  if (req.method === 'GET' && url.pathname === '/log') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify(actionLog));
  }
  if (req.method === 'POST' && url.pathname === '/reset') {
    actionLog = [];
    contacts = initialContacts();
    res.writeHead(200);
    return res.end('ok');
  }
  if (req.method === 'GET' && url.pathname === '/health') {
    res.writeHead(200);
    return res.end('ok');
  }
  res.writeHead(404);
  res.end('not found');
});

server.listen(PORT, () => {
  console.log(`READY port=${PORT}`);
});
