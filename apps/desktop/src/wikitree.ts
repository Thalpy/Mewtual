// The wiki sidebar's page tree: `/`-separated page names ("Guides/Setup/Linux") organise
// into nested, collapsible folders, the standard subpage scheme (MediaWiki subpages, wiki
// namespaces). Pure functions over the flat page list; the CRDT schema is untouched: a
// "folder" is nothing but a shared name prefix, so it needs no storage, can never conflict,
// and vanishes when its last page does.

/// One node in the page tree. A node is a folder (has children), a page (`page` set), or
/// both at once: "Guides" the page and "Guides/Setup" the subpage coexist, and the node for
/// "Guides" then both opens a page and expands.
export type WikiTreeNode = {
  /// The display segment ("Setup" for "Guides/Setup").
  label: string;
  /// The full path prefix up to and including this node ("Guides/Setup").
  path: string;
  /// The full page name if a page exists exactly at this path, else null.
  page: string | null;
  /// Child nodes, folders first, then by label (case-insensitive, stable).
  children: WikiTreeNode[];
};

/// Split a page name into its tree segments. Empty segments collapse ("a//b" nests like
/// "a/b") so a stray slash can't mint an unnamed folder level.
function segmentsOf(name: string): string[] {
  return name.split("/").filter((s) => s.trim() !== "");
}

/// Build the sidebar tree from the flat page list. Every page lands at its path; pages
/// whose name has no `/` are roots. Sorting: within each level, nodes order by label
/// case-insensitively, folders and pages intermixed (a reader scans one alphabet, not two).
export function buildWikiTree(pages: string[]): WikiTreeNode[] {
  const roots: WikiTreeNode[] = [];
  const nodeAt = new Map<string, WikiTreeNode>();
  for (const name of [...pages].sort((a, b) => a.localeCompare(b))) {
    const segs = segmentsOf(name);
    if (segs.length === 0) continue;
    let path = "";
    let siblings = roots;
    let node: WikiTreeNode | undefined;
    for (const seg of segs) {
      path = path ? `${path}/${seg}` : seg;
      node = nodeAt.get(path);
      if (!node) {
        node = { label: seg, path, page: null, children: [] };
        nodeAt.set(path, node);
        siblings.push(node);
      }
      siblings = node.children;
    }
    if (node) node.page = name;
  }
  const sortLevel = (nodes: WikiTreeNode[]) => {
    nodes.sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) || a.label.localeCompare(b.label));
    for (const n of nodes) sortLevel(n.children);
  };
  sortLevel(roots);
  return roots;
}

/// The paths of every folder node (a node with children) in the tree, for expand/collapse
/// bookkeeping.
export function folderPaths(nodes: WikiTreeNode[]): string[] {
  const out: string[] = [];
  const walk = (ns: WikiTreeNode[]) => {
    for (const n of ns) {
      if (n.children.length > 0) {
        out.push(n.path);
        walk(n.children);
      }
    }
  };
  walk(nodes);
  return out;
}

/// The ancestor folder paths of a page name ("a/b/c" -> ["a", "a/b"]): what must be
/// expanded for the page's row to be visible.
export function ancestorsOf(name: string): string[] {
  const segs = segmentsOf(name);
  const out: string[] = [];
  let path = "";
  for (const seg of segs.slice(0, -1)) {
    path = path ? `${path}/${seg}` : seg;
    out.push(path);
  }
  return out;
}

/// A row the sidebar renders: a tree node at a depth, visible under the current collapse
/// state. Collapsed folders keep their row but hide their descendants.
export type WikiTreeRow = { node: WikiTreeNode; depth: number };

/// Flatten the tree into visible rows given the set of collapsed folder paths.
export function visibleRows(nodes: WikiTreeNode[], collapsed: ReadonlySet<string>): WikiTreeRow[] {
  const out: WikiTreeRow[] = [];
  const walk = (ns: WikiTreeNode[], depth: number) => {
    for (const n of ns) {
      out.push({ node: n, depth });
      if (n.children.length > 0 && !collapsed.has(n.path)) walk(n.children, depth + 1);
    }
  };
  walk(nodes, 0);
  return out;
}
