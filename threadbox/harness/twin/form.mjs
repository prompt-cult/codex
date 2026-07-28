/// The Acme Motor Quotes form model.
///
/// Acme Motor Quotes is an invented comparison site. Nothing here is copied
/// from, or modelled on the markup of, any real site; it is written from
/// scratch to be *representative of a class of problem* — a multi-step quote
/// journey built by a legacy form generator.
///
/// It is deliberately difficult in the ways that matter:
///
/// - element handles are generated per session, so nothing can be hardcoded;
/// - two controls have no programmatic accessible name at all, so an
///   accessible-name lookup must fail and something else must resolve them;
/// - a pre-filled control appends rather than replaces unless cleared first,
///   which is a real behaviour that a naive write silently corrupts.
///
/// The last point is the reason `form.propose_write` carries `clearFirst`.

/// A small deterministic PRNG, so a session's "random" ids are stable for a
/// given seed and a run can be replayed exactly.
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const STEPS = [
  { index: 1, title: "About you" },
  { index: 2, title: "Your vehicle" },
  { index: 3, title: "Your cover" },
];

/// Field definitions. `accessibleName` of `null` means the generated markup
/// associates no label with the control — the pathological case.
const FIELDS = [
  { key: "first-name", step: 1, accessibleName: "First name", type: "text", prefill: "" },
  { key: "last-name", step: 1, accessibleName: "Last name", type: "text", prefill: "" },
  { key: "dob", step: 1, accessibleName: "Date of birth", type: "date", prefill: "" },
  { key: "email", step: 1, accessibleName: "Email address", type: "text", prefill: "" },
  { key: "phone", step: 1, accessibleName: null, type: "text", prefill: "" },
  { key: "postcode", step: 1, accessibleName: "Postcode", type: "text", prefill: "" },

  { key: "registration", step: 2, accessibleName: "Registration number", type: "text", prefill: "" },
  { key: "make", step: 2, accessibleName: "Make", type: "select", prefill: "",
    options: ["Audi", "BMW", "Ford", "Skoda", "Toyota", "Volkswagen"] },
  { key: "model", step: 2, accessibleName: "Model", type: "text", prefill: "" },
  // Pre-filled: a naive click-and-type appends to this rather than replacing.
  { key: "year", step: 2, accessibleName: "Year of manufacture", type: "text", prefill: "2015" },
  { key: "value", step: 2, accessibleName: null, type: "text", prefill: "" },
  { key: "parked-at", step: 2, accessibleName: "Where is it kept overnight", type: "select", prefill: "",
    options: ["driveway", "garage", "street", "carpark"] },

  { key: "cover-type", step: 3, accessibleName: "Cover type", type: "select", prefill: "",
    options: ["comprehensive", "third-party-fire-theft", "third-party"] },
  { key: "start-date", step: 3, accessibleName: "Cover start date", type: "date", prefill: "" },
  { key: "excess", step: 3, accessibleName: "Voluntary excess", type: "select", prefill: "",
    options: ["0", "100", "250", "500"] },
  { key: "mileage", step: 3, accessibleName: "Annual mileage", type: "text", prefill: "" },
  { key: "ncd", step: 3, accessibleName: "Years no claims", type: "select", prefill: "",
    options: ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9+"] },
];

/// Build a fresh session. Handles look like the output of a two-decade-old
/// form builder: a control prefix, a generated block id, and an ordinal.
export function newSession(seed = 20260728) {
  const random = mulberry32(seed);
  const block = () => Math.floor(random() * 0xfffff).toString(16).padStart(5, "0");
  const fields = FIELDS.map((field, ordinal) => ({
    ...field,
    handle: `ctl00_${block()}_${field.type === "select" ? "ddl" : "txt"}${String(ordinal).padStart(3, "0")}`,
    value: field.prefill,
  }));
  return { step: 1, navigation: 1, fields, submitted: false };
}

export function stepTitle(step) {
  return STEPS.find((s) => s.index === step)?.title ?? "Unknown";
}

export function visibleFields(session) {
  return session.fields.filter((f) => f.step === session.step);
}

/// Resolve by accessible name, scoped to the current step. A control whose
/// `accessibleName` is null is unreachable this way, which is the point.
export function resolveAccessible(session, accessibleName) {
  if (!accessibleName) {
    return { resolved: false, reason: "an empty accessible name matches nothing" };
  }
  const match = visibleFields(session).find((f) => f.accessibleName === accessibleName);
  if (!match) {
    return {
      resolved: false,
      reason: `no control on step ${session.step} has the accessible name "${accessibleName}"`,
    };
  }
  return { resolved: true, handle: match.handle };
}

/// Resolve by viewport point, which is how a vision binding's answer is used.
/// The layout is a single column of rows, so a y coordinate identifies a row.
export const LAYOUT = { originY: 180, rowHeight: 64, columnX: 420, width: 1280, height: 900 };

export function resolveAtPoint(session, x, y) {
  const rows = visibleFields(session);
  const index = Math.floor((y - LAYOUT.originY) / LAYOUT.rowHeight);
  if (index < 0 || index >= rows.length) {
    return { resolved: false, reason: `point (${x}, ${y}) is not over a control on step ${session.step}` };
  }
  return { resolved: true, handle: rows[index].handle };
}

/// The y coordinate at which a given field renders, used by the twin's own
/// renderer and by the fixture corpus generator.
export function fieldCentre(session, handle) {
  const rows = visibleFields(session);
  const index = rows.findIndex((f) => f.handle === handle);
  if (index < 0) return null;
  return { x: LAYOUT.columnX, y: LAYOUT.originY + index * LAYOUT.rowHeight + LAYOUT.rowHeight / 2 };
}

export function findByHandle(session, handle) {
  return session.fields.find((f) => f.handle === handle) ?? null;
}

/// Apply a write. Without `clearFirst` the value *appends*, which reproduces
/// the real behaviour of typing into a control that already has content.
export function applyWrite(session, handle, value, clearFirst) {
  const field = findByHandle(session, handle);
  if (!field) return { applied: false, reason: `no control has handle "${handle}"` };
  if (field.step !== session.step) {
    return { applied: false, reason: `control "${handle}" is not on the current step` };
  }
  if (field.type === "select" && field.options && !field.options.includes(value)) {
    return { applied: false, reason: `"${value}" is not one of the options for "${handle}"` };
  }
  field.value = clearFirst ? value : `${field.value}${value}`;
  return { applied: true, handle };
}

export function advance(session) {
  if (session.step >= STEPS.length) return { advanced: false, reason: "already on the final step" };
  session.step += 1;
  session.navigation += 1;
  return { advanced: true, step: session.step };
}

export { FIELDS, STEPS };
