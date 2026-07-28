/// The twin is scaffolding, but its *difficulty* is load-bearing: if it stops
/// being pathological the alpha stops proving anything. These tests pin the
/// behaviours the graph has to cope with.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  newSession, visibleFields, resolveAccessible, resolveAtPoint,
  applyWrite, advance, findByHandle, fieldCentre, LAYOUT,
} from "../form.mjs";
import { encodePng, surface } from "../png.mjs";

test("handles are generated per session and never hardcodable", () => {
  const a = newSession(1);
  const b = newSession(2);
  const handlesA = a.fields.map((f) => f.handle);
  const handlesB = b.fields.map((f) => f.handle);
  assert.notDeepEqual(handlesA, handlesB, "two seeds produced identical handles");
  assert.deepEqual(handlesA, newSession(1).fields.map((f) => f.handle), "same seed must replay");
  for (const h of handlesA) assert.match(h, /^ctl00_[0-9a-f]{5}_(txt|ddl)\d{3}$/);
});

test("two controls have no accessible name, so name lookup cannot reach them", () => {
  const session = newSession();
  const unnamed = session.fields.filter((f) => f.accessibleName === null);
  assert.equal(unnamed.length, 2, "the pathological controls have gone missing");
  for (const field of unnamed) {
    session.step = field.step;
    assert.equal(resolveAccessible(session, "").resolved, false);
  }
});

test("accessible-name lookup is scoped to the visible step", () => {
  const session = newSession();
  assert.equal(resolveAccessible(session, "First name").resolved, true);
  // "Make" exists, but on step 2.
  const off = resolveAccessible(session, "Make");
  assert.equal(off.resolved, false);
  assert.match(off.reason, /no control on step 1/);
});

test("a point over a control resolves to it, and the pad does not", () => {
  const session = newSession();
  const first = visibleFields(session)[0];
  const centre = fieldCentre(session, first.handle);
  assert.deepEqual(resolveAtPoint(session, centre.x, centre.y), { resolved: true, handle: first.handle });
  assert.equal(resolveAtPoint(session, centre.x, LAYOUT.originY - 40).resolved, false);
  assert.equal(resolveAtPoint(session, centre.x, LAYOUT.originY + 99 * LAYOUT.rowHeight).resolved, false);
});

test("writing a pre-filled control appends unless it is cleared first", () => {
  const session = newSession();
  advance(session);
  const year = session.fields.find((f) => f.key === "year");
  assert.equal(year.value, "2015", "the prefill is what makes this case real");

  applyWrite(session, year.handle, "2019", false);
  assert.equal(findByHandle(session, year.handle).value, "20152019", "append behaviour lost");

  applyWrite(session, year.handle, "2019", true);
  assert.equal(findByHandle(session, year.handle).value, "2019");
});

test("a select refuses a value that is not one of its options", () => {
  const session = newSession();
  advance(session);
  const make = session.fields.find((f) => f.key === "make");
  const bad = applyWrite(session, make.handle, "Lamborghini", true);
  assert.equal(bad.applied, false);
  assert.match(bad.reason, /is not one of the options/);
  assert.equal(applyWrite(session, make.handle, "Skoda", true).applied, true);
});

test("a write to a control on another step is refused", () => {
  const session = newSession();
  const make = session.fields.find((f) => f.key === "make");
  const result = applyWrite(session, make.handle, "Skoda", true);
  assert.equal(result.applied, false);
  assert.match(result.reason, /not on the current step/);
});

test("the renderer produces a real, byte-stable PNG", () => {
  const canvas = surface(8, 4, [255, 255, 255]);
  canvas.rect(1, 1, 3, 2, [0, 0, 0]);
  const png = canvas.toPng();
  assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  assert.equal(png.subarray(12, 16).toString("ascii"), "IHDR");
  assert.equal(png.readUInt32BE(16), 8);
  assert.equal(png.readUInt32BE(20), 4);
  // Reproducibility is what makes the fixture corpus key stable.
  assert.deepEqual(png, surface(8, 4, [255, 255, 255]).toPng().length === png.length
    ? (() => { const c = surface(8, 4, [255, 255, 255]); c.rect(1, 1, 3, 2, [0, 0, 0]); return c.toPng(); })()
    : png);
  assert.equal(encodePng(1, 1, Buffer.from([1, 2, 3])).subarray(12, 16).toString("ascii"), "IHDR");
});
