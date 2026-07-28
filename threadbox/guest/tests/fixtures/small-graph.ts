/// Golden fixture reproducing `IR.md`'s worked fragment exactly: a
/// document, a screenshot, a scale, a locate, a click, a field-driven
/// type, a retry bound, and the terminal. See
/// `guest/tests/fixtures/small-graph.json` for the expected byte-exact
/// output and `guest/tests/emit_golden.test.js` for the comparison.

import { loadJson, screenshot } from "../../assembly/ir";
import { emitGraph } from "../../assembly/emit";
import { Spec, Providers, Model, Think, Context } from "../../assembly/models";

export function main(): void {
  const doc = loadJson("submission");
  screenshot()
    .scale(1280)
    .locate(
      "the input labelled 'Case Number'",
      Spec.model(Providers.Anthropic, Model.OpusPerformance)
        .think(Think.Medium)
        .context(Context.OneMillion)
    )
    .click()
    .typeField(doc.field("caseNumber"))
    .attempts(3)
    .publish();
  emitGraph();
}
