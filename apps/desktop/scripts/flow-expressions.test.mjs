import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Script } from "node:vm";
import { gate7FlowExpression } from "./gate7-flow.mjs";
import { keptFlowExpression } from "./kept-flow.mjs";
import { privateDownloadFlowExpression } from "./private-download-flow.mjs";
import { scoreFlowExpression } from "./score-flow.mjs";

// These modules embed browser JavaScript in template literals. Importing catches
// broken outer quoting; parsing the generated expression catches broken inner
// quoting before an expensive Edge run. Actual DOM behavior remains test:flows.
const golden = JSON.parse(readFileSync(new URL(
  "../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url,
), "utf8"));
for (const [name, expression] of Object.entries({
  gate7: gate7FlowExpression(golden),
  kept: keptFlowExpression,
  privateDownload: privateDownloadFlowExpression,
  score: scoreFlowExpression,
})) {
  test(`${name} browser flow is valid JavaScript`, () => {
    assert.equal(typeof expression, "string", "parse the generated browser source");
    assert.doesNotThrow(() => new Script(expression, { filename: `${name}-flow.js` }));
  });
}
