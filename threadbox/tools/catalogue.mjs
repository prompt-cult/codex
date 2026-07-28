/// The allowlisted MJS tool catalogue.
///
/// Every tool declares its logical name, its input and result schema URIs, and
/// the capabilities it needs. A tool that is not in this map cannot be
/// dispatched, so a graph naming an unregistered tool fails before anything
/// happens.
///
/// **There is no submit tool, and there is no way to add one from a graph, a
/// prompt, or a model.** Absence is the control.
///
/// Each tool is a leaf: it performs one effect and returns typed data. None of
/// them decides what runs next — that is the graph's job, and keeping it that
/// way is what lets a binding be swapped without a rewrite.

const S = (path) => `agent-dsl://schemas/${path}`;

/// Tools reach the page through the twin's emulated DOM in this host. In the
/// extension host the same logical tools are bound to content-script calls;
/// the contracts below are what stay identical across both.
function pageClient(baseUrl) {
  async function call(method, path, body) {
    const response = await fetch(`${baseUrl}${path}`, {
      method,
      headers: body ? { "Content-Type": "application/json" } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await response.text();
    let parsed;
    try {
      parsed = text ? JSON.parse(text) : {};
    } catch {
      throw new Error(`page returned a non-JSON response from ${path}: ${text.slice(0, 200)}`);
    }
    return { status: response.status, body: parsed };
  }
  return {
    state: () => call("GET", "/twin/state"),
    read: (handle) => call("GET", `/twin/read?handle=${encodeURIComponent(handle)}`),
    render: async () => {
      const response = await fetch(`${baseUrl}/twin/render`);
      if (!response.ok) throw new Error(`capture failed: HTTP ${response.status}`);
      return Buffer.from(await response.arrayBuffer());
    },
    resolveName: (accessibleName) => call("POST", "/twin/resolve", { accessibleName }),
    resolvePoint: (x, y) => call("POST", "/twin/resolve-point", { x, y }),
    write: (handle, value, clearFirst) => call("POST", "/twin/write", { handle, value, clearFirst }),
    advance: () => call("POST", "/twin/advance", {}),
  };
}

/// Build the catalogue. `ctx` supplies the artifact store and the page client,
/// so a tool never reaches for ambient state of its own.
export function buildCatalogue({ baseUrl, artifacts }) {
  const page = pageClient(baseUrl);

  return new Map(Object.entries({
    "page.observe": {
      capabilities: ["page:read"],
      inputSchemaUri: S("tools/page-observe/input/0.1.0"),
      resultSchemaUri: S("tools/page-observe/result/0.1.0"),
      async run() {
        const { body } = await page.state();
        return {
          result: {
            navigationId: body.navigationId,
            url: body.url,
            title: body.title,
            step: body.step,
            confidence: 1,
          },
        };
      },
    },

    "dom.resolve_accessible": {
      capabilities: ["page:read"],
      inputSchemaUri: S("tools/dom-resolve/input/0.1.0"),
      resultSchemaUri: S("tools/dom-resolve/result/0.1.0"),
      async run(input) {
        const { body } = await page.resolveName(input.accessibleName ?? "");
        return {
          result: body.resolved
            ? { resolved: true, handle: body.handle }
            : { resolved: false, reason: body.reason ?? "not resolved" },
        };
      },
    },

    /// Resolve a viewport coordinate to a control. This is how a vision
    /// model's answer is *used*: the model proposes a point, and the page
    /// decides what is actually there. A model verdict never overrides an
    /// observation.
    "dom.resolve_point": {
      capabilities: ["page:read"],
      inputSchemaUri: S("tools/dom-resolve-point/input/0.1.0"),
      resultSchemaUri: S("tools/dom-resolve-point/result/0.1.0"),
      async run(input) {
        const { body } = await page.resolvePoint(input.x, input.y);
        return {
          result: body.resolved
            ? { resolved: true, handle: body.handle }
            : { resolved: false, reason: body.reason ?? "not resolved" },
        };
      },
    },

    /// Capture emits an immutable raw image artifact and does nothing else.
    /// It has no idea which model, comparison, or archive will consume it.
    "capture.viewport": {
      capabilities: ["page:capture"],
      inputSchemaUri: S("tools/capture-viewport/input/0.1.0"),
      resultSchemaUri: S("tools/capture-viewport/result/0.1.0"),
      async run(_input, call) {
        const bytes = await page.render();
        const artifact = await artifacts.create(call, {
          name: "viewport.png",
          mediaType: "image/png",
          semanticKind: "image",
          bytes,
          labels: { profile: "raw" },
        });
        return {
          result: { width: 1280, height: 900, devicePixelRatio: 1 },
          files: { items: [artifact] },
        };
      },
    },

    /// The token-preserving transform. It is a *separate* tool from capture on
    /// purpose: fusing them would bake one model's accepted dimensions into
    /// the capture step. A model-bound edge requires this to have run.
    "image.standardize": {
      capabilities: ["artifact:read", "artifact:write"],
      inputSchemaUri: S("tools/image-standardize/input/0.1.0"),
      resultSchemaUri: S("tools/image-standardize/result/0.1.0"),
      async run(input, call) {
        const source = artifacts.get(input.sourceArtifactId);
        if (!source) throw new Error(`no artifact "${input.sourceArtifactId}" to standardize`);
        if (source.semanticKind !== "image") {
          throw new Error(`artifact "${input.sourceArtifactId}" is ${source.semanticKind}, not an image`);
        }
        const standardWidth = input.standardWidth ?? 1280;
        const scale = standardWidth / 1280;
        // The twin renders at exactly the standard width, so normalization is
        // an identity scale here. The step still runs, still records its
        // profile, and still produces a *distinct* artifact — which is what
        // the guard downstream checks for.
        const artifact = await artifacts.create(call, {
          name: "normalized.png",
          mediaType: "image/png",
          semanticKind: "image",
          bytes: artifacts.bytes(input.sourceArtifactId),
          labels: {
            profile: "normalized",
            imageProfileId: `std-${standardWidth}`,
            sourceArtifactId: input.sourceArtifactId,
            scale: String(scale),
          },
        });
        return {
          result: {
            imageProfileId: `std-${standardWidth}`,
            standardWidth,
            scale,
            tileCount: 1,
            sourceArtifactId: input.sourceArtifactId,
          },
          files: { items: [artifact] },
        };
      },
    },

    "form.propose_write": {
      capabilities: ["page:read"],
      inputSchemaUri: S("tools/form-propose-write/input/0.1.0"),
      resultSchemaUri: S("tools/form-propose-write/result/0.1.0"),
      async run(input) {
        const { body } = await page.read(input.handle);
        const current = body.value ?? "";
        return {
          result: {
            proposalId: `prop_${input.handle}`,
            handle: input.handle,
            expected: String(input.value ?? ""),
            classification: "routine",
            // A control that already holds text must be cleared, or the write
            // appends. The proposal carries that decision so the applying tool
            // does not have to guess.
            clearFirst: current.length > 0,
          },
        };
      },
    },

    "form.apply": {
      capabilities: ["page:write"],
      inputSchemaUri: S("tools/form-apply/input/0.1.0"),
      resultSchemaUri: S("tools/form-apply/result/0.1.0"),
      async run(input) {
        const { status, body } = await page.write(input.handle, input.value, input.clearFirst);
        if (status !== 200 || !body.applied) {
          throw new Error(body.reason ?? `write to "${input.handle}" was refused`);
        }
        return { result: { applied: true, handle: input.handle } };
      },
    },

    "form.verify": {
      capabilities: ["page:read"],
      inputSchemaUri: S("tools/form-verify/input/0.1.0"),
      resultSchemaUri: S("tools/form-verify/result/0.1.0"),
      async run(input) {
        const { body } = await page.read(input.handle);
        const observed = body.value ?? "";
        return {
          result: { matches: observed === input.expected, observed, handle: input.handle },
        };
      },
    },

    "page.advance": {
      capabilities: ["page:write"],
      inputSchemaUri: S("tools/page-observe/input/0.1.0"),
      resultSchemaUri: S("tools/page-observe/result/0.1.0"),
      async run() {
        await page.advance();
        const { body } = await page.state();
        return {
          result: {
            navigationId: body.navigationId,
            url: body.url,
            title: body.title,
            step: body.step,
            confidence: 1,
          },
        };
      },
    },

    /// The human gate. In this host it answers from policy so a run is
    /// unattended; in the extension it raises real UI. Either way a
    /// `prohibited` classification is never approved.
    "human.gate": {
      capabilities: [],
      inputSchemaUri: S("tools/human-gate/input/0.1.0"),
      resultSchemaUri: S("tools/human-gate/result/0.1.0"),
      async run(input, _call, { policy }) {
        const classification = input.classification ?? "routine";
        const decision =
          classification === "prohibited"
            ? "reject"
            : policy.autoApprove.includes(classification)
              ? "approve"
              : "reject";
        return {
          result: { decision, proposalId: input.proposal?.proposalId ?? "", automatic: true },
        };
      },
    },

    "checkpoint.commit": {
      capabilities: ["log:append"],
      async run(input, call, { log }) {
        log.checkpoint(call, input.label, input.data);
        return { result: { committed: true, label: input.label } };
      },
    },
  }));
}

export { pageClient };
