/// The Acme Motor Quotes digital twin.
///
/// Throwaway test scaffolding, not production code. It serves two audiences
/// through one model of the same page:
///
/// - an **emulated DOM API** over HTTP, so the CLI host can drive the form
///   headlessly and CI needs no browser; and
/// - **real HTML**, so the same journey can later be driven by a content
///   script in the extension.
///
/// Every mutation is appended to a server-side action log. **That log is the
/// oracle**: it is how a run proves what actually happened, rather than what a
/// transcript claims happened.
///
/// Usage: node server.mjs [port]
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import {
  newSession,
  visibleFields,
  resolveAccessible,
  resolveAtPoint,
  findByHandle,
  applyWrite,
  advance,
  stepTitle,
  fieldCentre,
  LAYOUT,
} from "./form.mjs";
import { surface } from "./png.mjs";

const PORT = Number.parseInt(process.argv[2] ?? "3456", 10);

let session = newSession();
let actionLog = [];
let sequence = 0;

function record(action, detail) {
  sequence += 1;
  actionLog.push({ seq: sequence, action, ...detail });
}

function json(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

/// Read a JSON body defensively. A malformed body is a 400, never an
/// unhandled rejection that takes the process down mid-run.
function readJson(req) {
  return new Promise((resolve) => {
    let body = "";
    let tooBig = false;
    req.on("data", (chunk) => {
      body += chunk;
      if (body.length > 1_000_000) tooBig = true;
    });
    req.on("end", () => {
      if (tooBig) return resolve({ ok: false, error: "request body exceeds 1MB" });
      if (body.length === 0) return resolve({ ok: true, value: {} });
      try {
        resolve({ ok: true, value: JSON.parse(body) });
      } catch (cause) {
        resolve({ ok: false, error: `body is not valid JSON: ${cause.message}` });
      }
    });
    req.on("error", (cause) => resolve({ ok: false, error: cause.message }));
  });
}

function navigationId() {
  return `nav_${session.navigation}`;
}

function stateView() {
  return {
    step: session.step,
    title: stepTitle(session.step),
    navigationId: navigationId(),
    url: `http://localhost:${PORT}/quote/step-${session.step}`,
    fields: visibleFields(session).map((f) => ({
      handle: f.handle,
      accessibleName: f.accessibleName ?? "",
      type: f.type,
      value: f.value,
      options: f.options ?? [],
    })),
  };
}

/// Render the current step as a deterministic screenshot. Controls are drawn
/// as filled rows at the coordinates `form.mjs` declares, so a coordinate
/// returned against this image maps back to a real control.
function render() {
  const canvas = surface(LAYOUT.width, LAYOUT.height, [247, 248, 250]);
  // Header band and step indicator.
  canvas.rect(0, 0, LAYOUT.width, 96, [21, 58, 96]);
  canvas.rect(48, 120, 300 + session.step * 40, 24, [90, 110, 130]);

  visibleFields(session).forEach((field, index) => {
    const y = LAYOUT.originY + index * LAYOUT.rowHeight;
    // Label column, then the control itself; a filled control reads darker.
    canvas.rect(48, y + 18, 300, 20, [140, 148, 158]);
    canvas.rect(360, y + 8, 520, 40, [255, 255, 255]);
    canvas.rect(360, y + 8, 520, 2, [176, 184, 194]);
    canvas.rect(360, y + 46, 520, 2, [176, 184, 194]);
    if (field.value) canvas.rect(372, y + 20, Math.min(480, field.value.length * 11), 16, [60, 70, 84]);
  });

  const footerY = LAYOUT.originY + visibleFields(session).length * LAYOUT.rowHeight + 24;
  canvas.rect(360, footerY, 220, 48, [16, 122, 84]);
  return canvas.toPng();
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  const { pathname } = url;

  try {
    if (req.method === "GET" && pathname === "/health") {
      return json(res, 200, { ok: true });
    }

    if (req.method === "GET" && (pathname === "/" || pathname.startsWith("/quote/"))) {
      const html = renderHtml();
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      return res.end(html);
    }

    if (req.method === "GET" && pathname === "/twin/state") {
      return json(res, 200, stateView());
    }

    if (req.method === "GET" && pathname === "/twin/read") {
      const handle = url.searchParams.get("handle") ?? "";
      const field = findByHandle(session, handle);
      if (!field) return json(res, 404, { error: `no control has handle "${handle}"` });
      return json(res, 200, { handle, value: field.value });
    }

    if (req.method === "GET" && pathname === "/twin/render") {
      const png = render();
      res.writeHead(200, { "Content-Type": "image/png", "Content-Length": png.length });
      return res.end(png);
    }

    if (req.method === "GET" && pathname === "/log") {
      return json(res, 200, actionLog);
    }

    if (req.method === "POST") {
      const body = await readJson(req);
      if (!body.ok) return json(res, 400, { error: body.error });
      const input = body.value;

      if (pathname === "/reset") {
        session = newSession(input.seed ?? 20260728);
        actionLog = [];
        sequence = 0;
        return json(res, 200, { ok: true });
      }

      if (pathname === "/twin/resolve") {
        const result = resolveAccessible(session, input.accessibleName ?? "");
        record("resolve", { by: "accessible-name", accessibleName: input.accessibleName ?? "", resolved: result.resolved });
        return json(res, 200, result);
      }

      if (pathname === "/twin/resolve-point") {
        const result = resolveAtPoint(session, Number(input.x), Number(input.y));
        record("resolve", { by: "point", x: Number(input.x), y: Number(input.y), resolved: result.resolved });
        return json(res, 200, result);
      }

      if (pathname === "/twin/write") {
        const result = applyWrite(session, input.handle, String(input.value ?? ""), Boolean(input.clearFirst));
        record("write", {
          handle: input.handle,
          value: String(input.value ?? ""),
          clearFirst: Boolean(input.clearFirst),
          applied: result.applied,
        });
        return json(res, result.applied ? 200 : 409, result);
      }

      if (pathname === "/twin/advance") {
        const result = advance(session);
        record("advance", { step: session.step, advanced: result.advanced });
        return json(res, result.advanced ? 200 : 409, { ...result, navigationId: navigationId() });
      }
    }

    return json(res, 404, { error: `no route for ${req.method} ${pathname}` });
  } catch (cause) {
    // A defect in the twin must surface as a 500 with a message, never as a
    // dead process half way through a run.
    return json(res, 500, { error: `twin failed handling ${req.method} ${pathname}: ${cause.message}` });
  }
});

function renderHtml() {
  const rows = visibleFields(session)
    .map((field) => {
      const label = field.accessibleName
        ? `<label for="${field.handle}">${field.accessibleName}</label>`
        : `<span class="orphan-label">${field.key.replace(/-/g, " ")}</span>`;
      const control =
        field.type === "select"
          ? `<select id="${field.handle}" name="${field.handle}">${(field.options ?? [])
              .map((o) => `<option${o === field.value ? " selected" : ""}>${o}</option>`)
              .join("")}</select>`
          : `<input id="${field.handle}" name="${field.handle}" type="text" value="${field.value}">`;
      return `<div class="row"><div class="cell">${label}</div><div class="cell">${control}</div></div>`;
    })
    .join("\n");

  // The nesting and the generated ids are the point: this is what a form
  // built by a two-decade-old generator looks like to anything trying to
  // find a control.
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Acme Motor Quotes — ${stepTitle(session.step)}</title>
<style>
 body{font-family:system-ui,sans-serif;margin:0;background:#f7f8fa}
 header{background:#153a60;color:#fff;padding:1.5rem 3rem}
 main{padding:2rem 3rem;max-width:60rem}
 .row{display:flex;gap:1rem;align-items:center;height:64px}
 .cell{flex:1}
 .orphan-label{color:#666}
 button{background:#107a54;color:#fff;border:0;padding:.9rem 2rem;font-size:1rem}
</style></head>
<body>
<header><h1>Acme Motor Quotes</h1><p>Step ${session.step} of 3 — ${stepTitle(session.step)}</p></header>
<main><form id="quote-form"><div class="panel"><div class="inner">
${rows}
</div></div>
<button type="button" id="continue">${session.step < 3 ? "Continue" : "Get my quotes"}</button>
</form></main>
</body></html>`;
}

server.listen(PORT, "127.0.0.1", () => {
  // Bound to loopback deliberately: the twin has no authentication and logs
  // everything, so it must not be reachable from the network.
  process.stdout.write(`READY port=${PORT}\n`);
});

export { render, stateView };
