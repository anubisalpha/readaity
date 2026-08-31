import { useState } from "react";
import type { LibraryKind } from "../types";
import type { TreeNode } from "../lib/tree";

type Shelf = "library" | "favorites" | "beingRead";

interface Props {
  library: LibraryKind;
  firstLibrary: LibraryKind;
  onSwitchLibrary: (lib: LibraryKind) => void;
  shelf: Shelf;
  onShelf: (s: Shelf) => void;
  favoritesCount: number;
  beingReadCount: number;
  tree: TreeNode[];
  cwd: string | null;
  expanded: Set<string>;
  onNavigate: (path: string | null) => void;
  onToggle: (path: string) => void;
  onDropFolder: (dest: string) => void;
  onOpenNetwork: () => void;
}

/** Left column: library switcher on top, Windows-Explorer-style folder tree below. */
export function Sidebar({
  library,
  firstLibrary,
  onSwitchLibrary,
  shelf,
  onShelf,
  favoritesCount,
  beingReadCount,
  tree,
  cwd,
  expanded,
  onNavigate,
  onToggle,
  onDropFolder,
  onOpenNetwork,
}: Props) {
  return (
    <aside className="sidebar">
      <div className="lib-switcher">
        {(firstLibrary === "ebooks"
          ? (["ebooks", "comics"] as LibraryKind[])
          : (["comics", "ebooks"] as LibraryKind[])
        ).map((kind) => (
          <button
            key={kind}
            className={`lib-tab${library === kind ? " active" : ""}`}
            onClick={() => onSwitchLibrary(kind)}
          >
            {kind === "comics" ? "Comics" : "Ebooks"}
          </button>
        ))}
      </div>

      <button
        className={`tree-row shelf-row${
          shelf === "favorites" ? " active" : ""
        }`}
        onClick={() => onShelf("favorites")}
      >
        <span className="tree-chevron placeholder" />
        <span className="tree-label">★ Favourites</span>
        <span className="tree-count">{favoritesCount}</span>
      </button>
      <button
        className={`tree-row shelf-row${
          shelf === "beingRead" ? " active" : ""
        }`}
        onClick={() => onShelf("beingRead")}
      >
        <span className="tree-chevron placeholder" />
        <span className="tree-label">Being Read</span>
        <span className="tree-count">{beingReadCount}</span>
      </button>

      <button className="tree-row shelf-row" onClick={onOpenNetwork}>
        <span className="tree-chevron placeholder" />
        <span className="tree-label">🖧 Network</span>
      </button>

      <button
        className={`tree-row root-row${
          shelf === "library" && cwd === null ? " active" : ""
        }`}
        onClick={() => onNavigate(null)}
      >
        <span className="tree-chevron placeholder" />
        <span className="tree-label">Library</span>
      </button>
      {tree.map((node) => (
        <TreeItem
          key={node.path}
          node={node}
          depth={0}
          cwd={shelf === "library" ? cwd : null}
          expanded={expanded}
          onNavigate={onNavigate}
          onToggle={onToggle}
          onDropFolder={onDropFolder}
        />
      ))}
    </aside>
  );
}

function TreeItem({
  node,
  depth,
  cwd,
  expanded,
  onNavigate,
  onToggle,
  onDropFolder,
}: {
  node: TreeNode;
  depth: number;
  cwd: string | null;
  expanded: Set<string>;
  onNavigate: (path: string | null) => void;
  onToggle: (path: string) => void;
  onDropFolder: (dest: string) => void;
}) {
  const isOpen = expanded.has(node.path);
  const hasChildren = node.children.length > 0;
  const active = cwd === node.path;
  const [over, setOver] = useState(false);

  return (
    <div className="tree-node">
      <div
        className={`tree-row${active ? " active" : ""}${over ? " drop-over" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => onNavigate(node.path)}
        role="button"
        tabIndex={0}
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
          if (!over) setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          onDropFolder(node.path);
        }}
      >
        {hasChildren ? (
          <button
            className="tree-chevron"
            onClick={(e) => {
              e.stopPropagation();
              onToggle(node.path);
            }}
            aria-label={isOpen ? "Collapse" : "Expand"}
          >
            {isOpen ? "−" : "+"}
          </button>
        ) : (
          <span className="tree-chevron placeholder" />
        )}
        <span className="tree-label" title={node.name}>
          {node.name}
        </span>
        <span className="tree-count">{node.bookCount}</span>
      </div>
      {isOpen &&
        node.children.map((child) => (
          <TreeItem
            key={child.path}
            node={child}
            depth={depth + 1}
            cwd={cwd}
            expanded={expanded}
            onNavigate={onNavigate}
            onToggle={onToggle}
            onDropFolder={onDropFolder}
          />
        ))}
    </div>
  );
}
