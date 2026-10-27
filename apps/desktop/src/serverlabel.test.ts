import assert from "node:assert/strict";
import test from "node:test";
import {
  type LabelSources,
  type LabelledEntry,
  resolveServerLabel,
  settleServerLabels,
} from "./serverlabel.ts";

const sources = (over: Partial<LabelSources> = {}): LabelSources => ({
  local: "",
  published: "",
  created: "",
  ...over,
});

test("a member's own override outranks the group's published name", () => {
  assert.equal(
    resolveServerLabel(sources({ local: "work chat", published: "The Cat Cafe", created: "New server" })),
    "work chat",
  );
});

test("the group's published name outranks the label the entry was created with", () => {
  // The whole point of publishing: a joiner who typed nothing sees what the group is called
  // rather than the placeholder it was added under.
  assert.equal(
    resolveServerLabel(sources({ published: "The Cat Cafe", created: "New server" })),
    "The Cat Cafe",
  );
});

test("an unpublished, unrenamed group keeps the label it was created with", () => {
  assert.equal(resolveServerLabel(sources({ created: "New server" })), "New server");
});

test("whitespace is not a name: a blank override or publication does not win", () => {
  // Both arrive from storage and from another member's client respectively, so neither is
  // trustworthy enough to blank a rail entry by being present-but-empty.
  assert.equal(
    resolveServerLabel(sources({ local: "   ", published: "\t", created: "New server" })),
    "New server",
  );
});

test("settling applies a published name that became known before its entry existed", () => {
  // The regression. `join_server` runs the livery catch-up before it returns, so the group's
  // published name can land while the rail entry is still being constructed: a settle pass at
  // that moment has nothing to apply it to, and the entry then has to be settled as it is added.
  // Without that, the joined group kept its placeholder until the next restart.
  const published: Record<number, string> = { 7: "The Cat Cafe" };
  const entries: LabelledEntry[] = [];
  assert.equal(settleServerLabels(entries, () => sources()), false, "nothing to settle yet");

  entries.push({ id: 7, name: "New server", isDm: false });
  const changed = settleServerLabels(entries, (e) =>
    sources({ published: published[e.id] ?? "", created: e.name }),
  );
  assert.equal(changed, true);
  assert.equal(entries[0].name, "The Cat Cafe");
});

test("settling leaves a DM alone: its label is the friend, and is never published", () => {
  const entries: LabelledEntry[] = [{ id: 3, name: "Juniper", isDm: true }];
  const changed = settleServerLabels(entries, () => sources({ published: "The Cat Cafe" }));
  assert.equal(changed, false);
  assert.equal(entries[0].name, "Juniper");
});

test("settling reports no change when every entry already shows its label", () => {
  const entries: LabelledEntry[] = [{ id: 7, name: "The Cat Cafe", isDm: false }];
  const changed = settleServerLabels(entries, (e) =>
    sources({ published: "The Cat Cafe", created: e.name }),
  );
  assert.equal(changed, false, "an idle settle pass must not force a re-render");
});

test("settling never blanks an entry when no source has a name", () => {
  const entries: LabelledEntry[] = [{ id: 7, name: "New server", isDm: false }];
  settleServerLabels(entries, () => sources());
  assert.equal(entries[0].name, "New server");
});
