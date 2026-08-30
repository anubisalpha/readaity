import { useState } from "react";
import type { LibraryKind } from "../types";
import type { TreeNode } from "../lib/tree";

interface Props {
  library: LibraryKind;
  onSwitchLibrary: (lib: LibraryKind) => void;
  tree: TreeNode[];
  cwd: string | null;
  expanded: Set<string>;
  onNavigate: (path: string | null) => void;
  onToggle: (path: string) => void;
  onDropFolder: (dest: string) => void;
}

/** Left column: library switcher on top, Windows-Explorer-style folder tree below. */
export function Sidebar({
  library,
  onSwitchLibrary,
  tree,
  cwd,
  expanded,
  onNavigate,
  onToggle,
  onDropFolder,
}: Props) {
  return (
    <aside className="sidebar">
      <div className="lib-switcher">
        <button
          className={`lib-tab${library === "comics" ? " active" : ""}`}
          onClick={() => onSwitchLibrary("comics")}
        >
          Comics
        </button>
        <button
          className={`lib-tab${library === "ebooks" ? " active" : ""}`}
          onClick={() => onSwitchLibrary("ebooks")}
        >
          Ebooks
        </button>
      </div>

      <button
        className={`tree-row root-row${cwd === null ? " active" : ""}`}
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
          cwd={cwd}
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
