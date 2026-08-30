// Derive a navigable folder tree from the flat book list. The tree mirrors the
// real on-disk structure, but only branches that contain discovered comics are
// surfaced, and each library folder's `mode` shapes how it's presented:
//   tree    — the folder is one root, subfolders navigable
//   flat    — all nested comics collapse into one flat list under the folder
//   promote — the folder wrapper is dropped; its subfolders become top roots,
//             and any comics directly in it float up to the Library root.

import type { BookRow, FolderInfo } from "../types";

export interface FolderEntry {
  name: string;
  path: string;
  bookCount: number;
}

export interface Crumb {
  name: string;
  path: string | null;
}

export interface FolderView {
  crumbs: Crumb[];
  subfolders: FolderEntry[];
  booksHere: BookRow[];
}

export interface TreeNode {
  name: string;
  path: string;
  bookCount: number;
  children: TreeNode[];
}

/** A concrete top-level root actually shown to the user (post-mode-expansion). */
interface DisplayRoot {
  path: string;
  /** true → collapse everything under it into one flat list. */
  flatten: boolean;
}

export function normalize(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/+$/, "");
}

function dirOf(pathN: string): string {
  const i = pathN.lastIndexOf("/");
  return i === -1 ? pathN : pathN.slice(0, i);
}

function basename(pathN: string): string {
  const i = pathN.lastIndexOf("/");
  return i === -1 ? pathN : pathN.slice(i + 1);
}

function isUnder(pathN: string, dirN: string): boolean {
  return pathN === dirN || pathN.startsWith(dirN + "/");
}

function byName(a: { name: string }, b: { name: string }): number {
  return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
}

/** Expand library folders into the concrete roots shown, honouring each mode. */
function displayRoots(items: BookRow[], folders: FolderInfo[]): DisplayRoot[] {
  const out: DisplayRoot[] = [];
  for (const f of folders) {
    const fp = normalize(f.path);
    if (f.mode === "flat") {
      out.push({ path: fp, flatten: true });
    } else if (f.mode === "promote") {
      // Each immediate subfolder that contains comics becomes its own root.
      const childDirs = new Set<string>();
      for (const b of items) {
        const bp = normalize(b.path);
        if (!isUnder(bp, fp)) continue;
        const d = dirOf(bp);
        if (d === fp) continue; // direct-in-wrapper → floats to Library root
        const seg = d.slice(fp.length + 1).split("/")[0];
        childDirs.add(fp + "/" + seg);
      }
      for (const c of childDirs) out.push({ path: c, flatten: false });
    } else {
      out.push({ path: fp, flatten: false }); // "tree"
    }
  }
  return out;
}

/** Longest display root that contains `bp`, or null if it's loose. */
function effectiveRoot(bp: string, roots: DisplayRoot[]): DisplayRoot | null {
  let best: DisplayRoot | null = null;
  for (const r of roots) {
    if (isUnder(bp, r.path) && (!best || r.path.length > best.path.length)) {
      best = r;
    }
  }
  return best;
}

/** The directory a book is filed under for display (root itself if flattened). */
function effectiveDir(bp: string, roots: DisplayRoot[]): string {
  const r = effectiveRoot(bp, roots);
  if (r && r.flatten) return r.path;
  return dirOf(bp);
}

/**
 * Compute what to show at directory `cwd`.
 * `cwd === null` is the virtual root: display roots + any loose books.
 */
export function folderView(
  books: BookRow[],
  folders: FolderInfo[],
  cwd: string | null,
): FolderView {
  const items = books.filter((b) => b.status !== "invalid");
  const roots = displayRoots(items, folders);

  if (cwd === null) {
    const subfolders: FolderEntry[] = roots
      .map((r) => ({
        name: basename(r.path) || r.path,
        path: r.path,
        bookCount: items.filter((b) => isUnder(normalize(b.path), r.path)).length,
      }))
      .sort(byName);
    // Books not under any display root (e.g. loose in a promoted wrapper).
    const booksHere = items.filter(
      (b) => effectiveRoot(normalize(b.path), roots) === null,
    );
    return { crumbs: [{ name: "Library", path: null }], subfolders, booksHere };
  }

  const cwdN = normalize(cwd);
  const booksHere = items.filter(
    (b) => effectiveDir(normalize(b.path), roots) === cwdN,
  );

  const counts = new Map<string, number>();
  for (const b of items) {
    const edN = effectiveDir(normalize(b.path), roots);
    if (edN === cwdN || !edN.startsWith(cwdN + "/")) continue;
    const seg = edN.slice(cwdN.length + 1).split("/")[0];
    const childPath = cwdN + "/" + seg;
    counts.set(childPath, (counts.get(childPath) ?? 0) + 1);
  }
  const subfolders: FolderEntry[] = [...counts.entries()]
    .map(([path, bookCount]) => ({ name: basename(path), path, bookCount }))
    .sort(byName);

  return { crumbs: buildCrumbs(cwdN, roots), subfolders, booksHere };
}

function buildCrumbs(cwdN: string, roots: DisplayRoot[]): Crumb[] {
  const crumbs: Crumb[] = [{ name: "Library", path: null }];
  const r = effectiveRoot(cwdN + "/x", roots); // pretend a child to match the root
  const root = r?.path ?? roots.find((x) => x.path === cwdN)?.path;
  if (!root) return crumbs;

  crumbs.push({ name: basename(root) || root, path: root });
  if (cwdN === root) return crumbs;

  const rest = cwdN.slice(root.length + 1).split("/");
  let acc = root;
  for (const seg of rest) {
    acc = acc + "/" + seg;
    crumbs.push({ name: seg, path: acc });
  }
  return crumbs;
}

export function buildTree(books: BookRow[], folders: FolderInfo[]): TreeNode[] {
  const items = books.filter((b) => b.status !== "invalid");
  const roots = displayRoots(items, folders);
  const rootPaths = new Set(roots.map((r) => r.path));

  const dirs = new Set<string>();
  for (const b of items) {
    const bp = normalize(b.path);
    const r = effectiveRoot(bp, roots);
    if (!r) continue; // loose books create no tree nodes
    if (r.flatten) {
      dirs.add(r.path);
      continue;
    }
    let d = dirOf(bp);
    while (true) {
      dirs.add(d);
      if (d === r.path) break;
      d = dirOf(d);
    }
  }

  const nodes = new Map<string, TreeNode>();
  for (const path of dirs) {
    nodes.set(path, {
      name: basename(path) || path,
      path,
      bookCount: items.filter((b) => normalize(b.path).startsWith(path + "/")).length,
      children: [],
    });
  }

  const tops: TreeNode[] = [];
  for (const path of dirs) {
    const node = nodes.get(path)!;
    if (rootPaths.has(path)) {
      tops.push(node);
      continue;
    }
    const parent = nodes.get(dirOf(path));
    if (parent) parent.children.push(node);
    else tops.push(node);
  }

  const sortRec = (ns: TreeNode[]) => {
    ns.sort(byName);
    ns.forEach((n) => sortRec(n.children));
  };
  sortRec(tops);
  return tops;
}

/** Ancestor directory paths of `cwd` up to its display root (for auto-expand). */
export function ancestors(
  cwd: string | null,
  books: BookRow[],
  folders: FolderInfo[],
): string[] {
  if (cwd === null) return [];
  const cwdN = normalize(cwd);
  const roots = displayRoots(
    books.filter((b) => b.status !== "invalid"),
    folders,
  );
  const root =
    effectiveRoot(cwdN + "/x", roots)?.path ??
    roots.find((x) => x.path === cwdN)?.path;
  if (!root) return [];

  const out: string[] = [];
  let d = cwdN;
  while (d !== root) {
    out.push(d);
    d = dirOf(d);
  }
  out.push(root);
  return out;
}

/** Does `cwd` still exist? Used to reset stale navigation. */
export function locationExists(
  cwd: string | null,
  books: BookRow[],
  folders: FolderInfo[],
): boolean {
  if (cwd === null) return true;
  const cwdN = normalize(cwd);
  const roots = displayRoots(
    books.filter((b) => b.status !== "invalid"),
    folders,
  );
  if (roots.some((r) => cwdN === r.path)) return true;
  return books.some((b) => isUnder(normalize(b.path), cwdN));
}
