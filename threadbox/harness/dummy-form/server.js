/// A throwaway CRM-like dummy form server, for the ThreadBox end-to-end
/// proof only. Not production code, not part of `guest/` or `rust/`,
/// and not subject to their zero-dependency rules — this is Node
/// `node:http` test scaffolding. See `harness/dummy-form/README.md`.
///
/// Usage: node server.js [port]
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dir = dirname(fileURLToPath(import.meta.url));
const PORT = parseInt(process.argv[2] || '3456', 10);

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

function serveFile(res, path, contentType) {
  res.writeHead(200, { 'Content-Type': contentType });
  res.end(readFileSync(join(__dir, path), 'utf8'));
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (req.method === 'GET' && url.pathname === '/') {
    return serveFile(res, 'public/index.html', 'text/html');
  }
  if (req.method === 'GET' && url.pathname === '/contact.html') {
    return serveFile(res, 'public/contact.html', 'text/html');
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
