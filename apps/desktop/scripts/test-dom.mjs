/**
 * Give the test runner a DOM.
 *
 * Loaded with `node --import`, which is deliberate rather than convenient: DOMPurify decides at
 * *module-evaluation* time whether it has a window, and either binds to it or degrades. ES module
 * imports hoist, so a shim written inside a test file would run after `render.ts` (and therefore
 * after DOMPurify) had already been evaluated. A preload is the only placement that reliably
 * happens first.
 *
 * Without this, the app's sanitizer boundary was untestable, and so untested: `render.ts` is where
 * text written by other group members becomes markup, and the two tests that come closest to it
 * both stop at its edge with a comment deferring to "the DOMPurify allow-list".
 *
 * **jsdom, not happy-dom**, despite happy-dom being the faster and more obvious pick. DOMPurify
 * 3.4.13 added a realm-safe tag-name probe that reads names through the cached
 * `Node.prototype.nodeName` getter, so that a clobbered element cannot lie about what it is.
 * happy-dom has that getter but it returns `""` for every node, so the sanitizer sees nameless
 * elements and strips markup it should keep. The failure is silent and looks like a config
 * mistake. jsdom returns real tag names, and is the environment DOMPurify is tested against.
 */
import { JSDOM } from "jsdom";

const { window } = new JSDOM("", { url: "http://localhost/" });

// Only what a sanitizer needs: a document to parse into, the node types it walks, and the
// constructors it identity-checks against. Deliberately not a wholesale copy of every window
// property onto the global object, which would let a test pass by accident on some browser API
// the app would not actually have.
const globals = {
  window,
  document: window.document,
  DOMParser: window.DOMParser,
  Node: window.Node,
  Element: window.Element,
  HTMLElement: window.HTMLElement,
  HTMLTemplateElement: window.HTMLTemplateElement,
  HTMLFormElement: window.HTMLFormElement,
  DocumentFragment: window.DocumentFragment,
  NodeFilter: window.NodeFilter,
  NamedNodeMap: window.NamedNodeMap,
  Text: window.Text,
  Comment: window.Comment,
};

for (const [name, value] of Object.entries(globals)) {
  if (value === undefined) throw new Error(`the test DOM is missing ${name}`);
  Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
}
