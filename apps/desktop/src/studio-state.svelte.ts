// Shared studio state for the two places that show it: the contextual sidebar (StudioNav) and
// the content surface (Studio). One in-memory store per app session until Studio save/load is
// connected; `rev` is bumped after every mutation so derived views re-read the plain-data store.

import { StudioStore, demoStudio } from "./studio-store.ts";

export type StudioPeople = { me: string; rook: string; mika: string; wren: string; owner: string };

export const studio = $state({
  store: null as StudioStore | null,
  selected: "",
  rev: 0,
  people: null as StudioPeople | null,
});

/// Build the demo studio once, keyed on the local identity. Fixtures stand in for what the
/// StudioIndex and StudioObject documents will materialize.
export function ensureStudio(me: string): StudioStore {
  if (studio.store && studio.people?.me === me) return studio.store;
  const people: StudioPeople = {
    me,
    rook: "rook".padEnd(64, "0"),
    mika: "mika".padEnd(64, "0"),
    wren: "wren".padEnd(64, "0"),
    owner: "owner".padEnd(64, "0"),
  };
  const { store, moonCat } = demoStudio(people);
  studio.store = store;
  studio.people = people;
  studio.selected = moonCat;
  studio.rev++;
  return store;
}

export function bump(): void {
  studio.rev++;
}

/// Fixture identities render with fixed names and colours; anything else is a real member the
/// host resolves. The colours are the peer colours the mockups use (identity, not theme).
export function fixtureName(id: string): string | null {
  if (!studio.people) return null;
  if (id === studio.people.rook) return "rook";
  if (id === studio.people.mika) return "mika";
  if (id === studio.people.wren) return "wren";
  if (id === studio.people.owner) return "thalpy";
  return null;
}

export function fixtureColor(id: string): string | null {
  if (!studio.people) return null;
  if (id === studio.people.rook) return "#d8a657";
  if (id === studio.people.mika) return "#6ca0d8";
  if (id === studio.people.wren) return "#e07ab8";
  if (id === studio.people.owner) return "#5ec96e";
  return null;
}
