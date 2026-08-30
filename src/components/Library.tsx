import { useEffect, useMemo, useState } from "react";
import type {
  BookRow,
  FolderInfo,
  LibraryKind,
  MoveAction,
  MovePlan,
  ScanStatus,
} from "../types";
import { Cover } from "./Cover";
import { StatusBar } from "./StatusBar";
import { Sidebar } from "./Sidebar";
import { MoveDialog } from "./MoveDialog";
import { moveItems, planMove } from "../lib/api";
import {
  ancestors,
  buildTree,
  folderView,
  locationExists,
  normalize,
  type Crumb,
  type FolderEntry,
} from "../lib/tree";

interface Props {
  books: BookRow[];
  folders: FolderInfo[];
  status: ScanStatus;
  library: LibraryKind;
  comicsCount: number;
  ebooksCount: number;
  onSwitchLibrary: (lib: LibraryKind) => void;
  onOpenSettings: () => void;
  onReindex: () => void;
  onAddFolder: () => void;
  onRemoveFolder: (folder: string) => void;
  onRemoveBook: (path: string) => void;
  onRemovePath: (path: string) => void;
  onPause: () => void;
  onResume: () => void;
  onBooksChanged: (books: BookRow[]) => void;
  onOpenBook: (book: BookRow) => void;
}

export function Library({
  books,
  folders,
  status,
  library,
  comicsCount,
  ebooksCount,
  onSwitchLibrary,
  onOpenSettings,
  onReindex,
  onAddFolder,
  onRemoveFolder,
  onRemoveBook,
  onRemovePath,
  onPause,
  onResume,
  onBooksChanged,
  onOpenBook,
}: Props) {
  // Current directory being browsed. null = the virtual "Library" root.
  const [cwd, setCwd] = useState<string | null>(null);
  // Which sidebar tree nodes are expanded.
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  // Auto-descend into a lone tree/flat folder; reset if the location goes away.
  useEffect(() => {
    if (!locationExists(cwd, books, folders)) {
      setCwd(null);
    } else if (
      cwd === null &&
      folders.length === 1 &&
      folders[0].mode !== "promote"
    ) {
      setCwd(normalize(folders[0].path));
    }
  }, [folders, books, cwd]);

  // Keep the current location revealed in the sidebar tree.
  useEffect(() => {
    const anc = ancestors(cwd, books, folders);
    if (anc.length) {
      setExpanded((prev) => {
        const next = new Set(prev);
        anc.forEach((a) => next.add(a));
        return next;
      });
    }
  }, [cwd, books, folders]);

  const toggle = (path: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });

  const tree = useMemo(() => buildTree(books, folders), [books, folders]);
  const view = useMemo(
    () => folderView(books, folders, cwd),
    [books, folders, cwd],
  );

  // ----- Windows-style multi-select -----
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [anchor, setAnchor] = useState<string | null>(null);

  // Ordered paths in the current view (folders first, then books) for Shift-range.
  const orderedPaths = useMemo(
    () => [
      ...view.subfolders.map((f) => f.path),
      ...view.booksHere.map((b) => b.path),
    ],
    [view],
  );

  const clearSel = () => {
    setSelected(new Set());
    setAnchor(null);
  };

  // Reset selection whenever the location or library changes.
  useEffect(() => {
    setSelected(new Set());
    setAnchor(null);
  }, [cwd, library]);

  const toggleSel = (p: string) => {
    setSelected((prev) => {
      const n = new Set(prev);
      n.has(p) ? n.delete(p) : n.add(p);
      return n;
    });
    setAnchor(p);
  };

  const selectRange = (to: string) => {
    const a = anchor ? orderedPaths.indexOf(anchor) : -1;
    const b = orderedPaths.indexOf(to);
    if (a === -1 || b === -1) {
      setSelected(new Set([to]));
      setAnchor(to);
      return;
    }
    const [lo, hi] = a < b ? [a, b] : [b, a];
    setSelected(new Set(orderedPaths.slice(lo, hi + 1)));
  };

  const activateFolder = (e: React.MouseEvent, path: string) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      toggleSel(path);
    } else if (e.shiftKey) {
      e.preventDefault();
      selectRange(path);
    } else {
      clearSel();
      setCwd(path);
    }
  };

  const activateBook = (e: React.MouseEvent, book: BookRow) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      toggleSel(book.path);
    } else if (e.shiftKey) {
      e.preventDefault();
      selectRange(book.path);
    } else {
      clearSel();
      onOpenBook(book);
    }
  };

  // ----- Drag to move -----
  const [dragging, setDragging] = useState<string[] | null>(null);
  const [pendingMove, setPendingMove] = useState<{
    dest: string;
    destName: string;
    plans: MovePlan[];
    collisions: { src: string; name: string }[];
  } | null>(null);

  const startDrag = (e: React.DragEvent, path: string) => {
    // Drag the whole selection if the grabbed tile is part of it; else just it.
    const sources = selected.has(path) ? [...selected] : [path];
    if (!selected.has(path)) setSelected(new Set(sources));
    setDragging(sources);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", "readaity-move");
  };

  const runMove = async (dest: string, plans: MovePlan[], action: MoveAction) => {
    const ops = plans.map((p) => ({
      src: p.src,
      action: p.collides ? action : ("move" as MoveAction),
    }));
    onBooksChanged(await moveItems(dest, ops, library));
    clearSel();
  };

  const dropOnFolder = async (dest: string) => {
    const sources = dragging;
    setDragging(null);
    if (!sources || sources.length === 0) return;
    const plans = (await planMove(sources, dest)).filter((p) => !p.error);
    if (plans.length === 0) return;
    const collisions = plans.filter((p) => p.collides);
    if (collisions.length > 0) {
      setPendingMove({
        dest,
        destName: dest.split(/[\\/]/).filter(Boolean).pop() ?? dest,
        plans,
        collisions: collisions.map((c) => ({ src: c.src, name: c.name })),
      });
    } else {
      await runMove(dest, plans, "move");
    }
  };

  // Ctrl+A selects all in view; Esc clears.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        setSelected(new Set(orderedPaths));
      } else if (e.key === "Escape") {
        clearSel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [orderedPaths]);

  const invalid = useMemo(
    () => books.filter((b) => b.status === "invalid"),
    [books],
  );

  const idle = status.phase === "idle";
  const nothingHere = view.subfolders.length === 0 && view.booksHere.length === 0;

  return (
    <div className="library">
      <StatusBar
        status={status}
        comicsCount={comicsCount}
        booksCount={ebooksCount}
        onPause={onPause}
        onResume={onResume}
      />

      <div className="library-body">
        <Sidebar
          library={library}
          onSwitchLibrary={onSwitchLibrary}
          tree={tree}
          cwd={cwd}
          expanded={expanded}
          onNavigate={setCwd}
          onToggle={toggle}
          onDropFolder={dropOnFolder}
        />

        <main className="content">
          <header className="library-header">
            <h1>Readaity</h1>
            <div className="header-actions">
              <button
                className="btn ghost icon"
                onClick={onReindex}
                title="Re-index this library (rebuild covers & page counts)"
                aria-label="Re-index"
              >
                ↻
              </button>
              <button
                className="btn ghost icon"
                onClick={onOpenSettings}
                title="Settings"
                aria-label="Settings"
              >
                ⚙
              </button>
              <button className="btn primary" onClick={onAddFolder}>
                ＋ Add folder
              </button>
            </div>
          </header>

          <Breadcrumb crumbs={view.crumbs} onNavigate={setCwd} />

          {folders.length === 0 && idle ? (
            <div className="empty">
              <p className="empty-title">
                No {library === "ebooks" ? "ebooks" : "comics"} yet
              </p>
              <p className="empty-sub">
                {library === "ebooks" ? (
                  <>
                    Add a folder of <code>.epub</code>, <code>.pdf</code> or{" "}
                    <code>.mobi</code> files to build your library.
                  </>
                ) : (
                  <>
                    Add a folder of <code>.cbz</code> or <code>.cbr</code> files
                    to build your library.
                  </>
                )}
              </p>
              <button className="btn primary" onClick={onAddFolder}>
                Choose a folder
              </button>
            </div>
          ) : nothingHere ? (
            <div className="empty-inline">
              {idle ? "This folder is empty." : "Scanning…"}
            </div>
          ) : (
            <>
              {selected.size > 0 && (
                <div className="sel-bar">
                  <span className="sel-count">{selected.size} selected</span>
                  <button className="btn small" onClick={clearSel}>
                    Clear
                  </button>
                </div>
              )}
              <div
                className="shelf"
                onClick={(e) => {
                  if (e.target === e.currentTarget) clearSel();
                }}
              >
                {view.subfolders.map((f) => (
                  <FolderItem
                    key={f.path}
                    entry={f}
                    selected={selected.has(f.path)}
                    // Root tiles are library folders → remove_folder; subfolders → remove_path.
                    onRemove={
                      cwd === null
                        ? () => onRemoveFolder(f.path)
                        : () => onRemovePath(f.path)
                    }
                    onActivate={(e) => activateFolder(e, f.path)}
                    onDragStart={(e) => startDrag(e, f.path)}
                    onDropFolder={dropOnFolder}
                  />
                ))}
                {view.booksHere.map((book) =>
                  book.status === "ready" ? (
                    <ReadyItem
                      key={book.path}
                      book={book}
                      selected={selected.has(book.path)}
                      onActivate={(e) => activateBook(e, book)}
                      onRemove={() => onRemoveBook(book.path)}
                      onDragStart={(e) => startDrag(e, book.path)}
                    />
                  ) : (
                    <PendingItem key={book.path} book={book} />
                  ),
                )}
              </div>
            </>
          )}

          {invalid.length > 0 && (
            <details className="invalid-note">
              <summary>
                {invalid.length} file{invalid.length > 1 ? "s" : ""} couldn't be
                read
              </summary>
              <ul>
                {invalid.map((b) => (
                  <li key={b.path}>
                    <span className="inv-title">{b.title}</span>
                    <span className="inv-reason">{b.error ?? "unknown error"}</span>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </main>
      </div>

      {pendingMove && (
        <MoveDialog
          destName={pendingMove.destName}
          collisions={pendingMove.collisions}
          onResolve={(action) => {
            const pm = pendingMove;
            setPendingMove(null);
            runMove(pm.dest, pm.plans, action);
          }}
          onCancel={() => setPendingMove(null)}
        />
      )}
    </div>
  );
}

function Breadcrumb({
  crumbs,
  onNavigate,
}: {
  crumbs: Crumb[];
  onNavigate: (path: string | null) => void;
}) {
  return (
    <nav className="breadcrumb">
      {crumbs.map((c, i) => {
        const last = i === crumbs.length - 1;
        return (
          <span key={c.path ?? "__root"} className="crumb">
            {last ? (
              <span className="crumb-current">{c.name}</span>
            ) : (
              <button className="crumb-link" onClick={() => onNavigate(c.path)}>
                {c.name}
              </button>
            )}
            {!last && <span className="crumb-sep">›</span>}
          </span>
        );
      })}
    </nav>
  );
}

function FolderItem({
  entry,
  selected,
  onActivate,
  onRemove,
  onDragStart,
  onDropFolder,
}: {
  entry: FolderEntry;
  selected: boolean;
  onActivate: (e: React.MouseEvent) => void;
  onRemove?: () => void;
  onDragStart: (e: React.DragEvent) => void;
  onDropFolder: (dest: string) => void;
}) {
  const [over, setOver] = useState(false);
  return (
    <div
      className={`shelf-item folder-item${selected ? " selected" : ""}${
        over ? " drop-over" : ""
      }`}
      draggable
      onDragStart={onDragStart}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        if (!over) setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        onDropFolder(entry.path);
      }}
    >
      <button
        className="folder-open"
        onClick={onActivate}
        title={entry.name}
      >
        <div className="folder-glyph">
          <svg viewBox="0 0 24 24" width="46" height="46" aria-hidden="true">
            <path
              fill="currentColor"
              d="M10 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-8l-2-2Z"
            />
          </svg>
        </div>
        <div className="shelf-meta">
          <span className="shelf-title">{entry.name}</span>
          <span className="shelf-sub">
            {entry.bookCount} book{entry.bookCount === 1 ? "" : "s"}
          </span>
        </div>
      </button>
      {onRemove && (
        <button
          className="tile-remove"
          onClick={onRemove}
          title="Remove from library (keeps files on disk)"
          aria-label={`Remove ${entry.name} from library`}
        >
          ×
        </button>
      )}
    </div>
  );
}

function ReadyItem({
  book,
  selected,
  onActivate,
  onRemove,
  onDragStart,
}: {
  book: BookRow;
  selected: boolean;
  onActivate: (e: React.MouseEvent) => void;
  onRemove: () => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  const isEpub = book.format === "epub";
  const started = book.last_page > 0;
  // EPUB progress is per-mille (0–1000); comics/PDF are page indices.
  const pct = isEpub
    ? Math.min(100, Math.round(book.last_page / 10))
    : book.page_count
      ? Math.round(((book.last_page + 1) / book.page_count) * 100)
      : 0;
  const sub = isEpub
    ? started
      ? `${pct}% read`
      : "EPUB"
    : book.page_count > 0
      ? `${started ? `${pct}% · ` : ""}${book.page_count} pages`
      : book.format.toUpperCase();
  return (
    <div
      className={`shelf-item${selected ? " selected" : ""}`}
      draggable
      onDragStart={onDragStart}
    >
      <button className="book-open" onClick={onActivate} title={book.title}>
        <div className="cover-wrap">
          <Cover path={book.path} title={book.title} format={book.format} />
          <span className={`format-badge ${book.format}`}>
            {book.format.toUpperCase()}
          </span>
        </div>
        <div className="shelf-meta">
          <span className="shelf-title">{book.title}</span>
          <span className="shelf-sub">{sub}</span>
          {started && (
            <span className="progress-bar">
              <span className="progress-fill" style={{ width: `${pct}%` }} />
            </span>
          )}
        </div>
      </button>
      <button
        className="tile-remove"
        onClick={onRemove}
        title="Remove from library (keeps file on disk)"
        aria-label={`Remove ${book.title} from library`}
      >
        ×
      </button>
    </div>
  );
}

function PendingItem({ book }: { book: BookRow }) {
  return (
    <div className="shelf-item pending" title={`${book.title} (scanning…)`}>
      <div className="cover">
        <div className="cover-placeholder scanning">
          <span className="spinner" />
        </div>
      </div>
      <div className="shelf-meta">
        <span className="shelf-title">{book.title}</span>
        <span className="shelf-sub">Scanning…</span>
      </div>
    </div>
  );
}
