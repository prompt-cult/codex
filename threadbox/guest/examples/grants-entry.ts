/// Worked example — grants data entry. See `DSL.md` §"Worked example —
/// grants data entry" for the narrative this program follows verbatim.
///
/// The scenario: a beneficiary submission arrives as JSON. A legacy grants
/// case-management application must be driven to find or create the case,
/// key every field of the submission into it, save, and confirm the save
/// happened.

import { loadJson, screenshot, Multi, Step, FieldRef, Doc } from "../assembly/ir";
import { emitGraph } from "../assembly/emit";
import { Spec, Tier, Providers, Model, Think, Context,
         withReviewMode } from "../assembly/models";

// The submission is declared, not read. The guest has no filesystem;
// whoever eventually runs the graph supplies the document by name.
const submission: Doc = loadJson("submission");

/// Find the search box and click it. The vision model is named
/// explicitly, so the record takes the catalog filter path and carries
/// no role: resolving pixels to coordinates is not one of the four
/// logical roles.
function openSearchBox(): Step {
  return screenshot()
    .scale(1280)
    .locate("the text input labelled 'Search' at the top of the page",
            Spec.model(Providers.Anthropic, Model.OpusPerformance)
                .think(Think.Medium)
                .context(Context.OneMillion))
    .click();
}

/// The case already exists: open it.
function openExistingCase(from: Step): Step {
  return from
    .then(screenshot().scale(1280))
    .locate("the first row of the search results table")
    .click();
}

/// The case does not exist: create it and key the identifying fields.
function createNewCase(from: Step): Step {
  return from
    .then(screenshot().scale(1280))
    .locate("the button labelled 'New Case'")
    .click()
    .then(screenshot().scale(1280))
    .locate("the input labelled 'Case Number'")
    .typeField(submission.field("caseNumber"))
    .then(screenshot().scale(1280))
    .locate("the textarea labelled 'Description'")
    .typeField(submission.field("description"));
}

/// One field of the submission. Each element of `answers` carries the
/// label to find and the value to key.
///
/// The body is self-contained: it starts its own chain and receives no
/// cursor, because it is built before the ForEach node that names it.
///
/// Three attempts against the default locator, then one escalation to a
/// stronger model. `attempts` is a literal, so the cost of this body is
/// bounded before the graph runs.
function enterField(element: FieldRef): Step {
  return screenshot()
    .scale(1280)
    .locate("the input whose visible label matches the element label")
    .click()
    .typeField(element)
    .attempts(3)
    .orElse(
      screenshot()
        .scale(1920)
        .locate("the input whose visible label matches the element label",
                Spec.model(Providers.Anthropic, Model.OpusPerformance)
                    .think(Think.High)
                    .context(Context.OneMillion))
        .click()
        .typeField(element)
    );
}

export function main(): void {
  const searched: Step = openSearchBox()
    .typeField(submission.field("caseNumber"))
    .then(screenshot().scale(1280));

  // The only conditional. It consumes a Verify node; it does not
  // observe a value during construction.
  const opened: Step = searched
    .confirm("the search results table contains at least one row",
             withReviewMode(Spec.tier(Tier.Balanced)))
    .branch(openExistingCase, createNewCase);

  const filled: Step = Multi.over(submission.fields("answers"))
    .forEach(enterField)
    .after(opened);

  filled
    .then(screenshot().scale(1280))
    .locate("the button labelled 'Save'")
    .click()
    .then(screenshot().scale(1280))
    .confirm("a confirmation banner reading 'Case saved' is visible",
             withReviewMode(Spec.tier(Tier.Balanced)))
    .attempts(2)
    .publish();

  emitGraph();
}
