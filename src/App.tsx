import { useCallback, useEffect, useState } from "react";
import "./App.css";
import type {
  BookRow,
  FolderInfo,
  FolderMode,
  LibraryKind,
  ProbeResult,
  ScanStatus,
} from "./types";
import {
  addFolder,
  getSetting,
  libraryCounts,
  listBooks,
  listFolders,
  onBookUpdated,
  onLibraryRecovered,
  onScanStatus,
  pauseIndexing,
  pickFolder,
  probeFolder,
  reindex,
  removeBook,
  removeFolder,
  removePath,
  rescan,
  resumeIndexing,
  setProgress,
  setSetting,
} from "./lib/api";
import { Library } from "./components/Library";
import { Reader } from "./components/Reader";
import { EbookReader } from "./components/EbookReader";
import { AddFolderDialog } from "./components/AddFolderDialog";
import { Settings } from "./components/Settings";
import { isComic } from "./lib/formats";

const IDLE: ScanStatus = { phase: "idle", current: 0, total: 0 };

interface PendingAdd {
  path: string;
  probe: ProbeResult;
}

function sortBooks(books: BookRow[]): BookRow[] {
  return [...books].sort((a, b) =>
    a.title.toLowerCase().localeCompare(b.title.toLowerCase()),
  );
}

/** Update a book in place by path. New rows only arrive via list refreshes, so
 *  a sweep event for a book not in the current library's list is ignored. */
function mergeBook(books: BookRow[], row: BookRow): BookRow[] {
  const idx = books.findIndex((b) => b.path === row.path);
  if (idx === -1) return books;
  const next = [...books];
  next[idx] = row;
  return next;
}

function App() {
  const [books, setBooks] = useState<BookRow[]>([]);
  const [folders, setFolders] = useState<FolderInfo[]>([]);
  const [status, setStatus] = useState<ScanStatus>(IDLE);
  const [library, setLibrary] = useState<LibraryKind>("comics");
  // Which library opens on launch and leads the sidebar switcher.
  const [firstLibrary, setFirstLibrary] = useState<LibraryKind>("comics");
  const [booted, setBooted] = useState(false);
  const [ready, setReady] = useState(false);
  const [openBook, setOpenBook] = useState<BookRow | null>(null);
  const [pendingAdd, setPendingAdd] = useState<PendingAdd | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [counts, setCounts] = useState({ comics: 0, ebooks: 0 });
  const [recovered, setRecovered] = useState(false);

  const refreshCounts = useCallback(() => {
    libraryCounts()
      .then(setCounts)
      .catch((e) => console.error("counts failed", e));
  }, []);

  const loadLibrary = useCallback(
    async (lib: LibraryKind) => {
      const [fs, bs] = await Promise.all([listFolders(lib), listBooks(lib)]);
      setFolders(fs);
      setBooks(sortBooks(bs));
      refreshCounts();
    },
    [refreshCounts],
  );

  // Subscribe to live events once.
  useEffect(() => {
    let unlisteners: Array<() => void> = [];
    (async () => {
      unlisteners = await Promise.all([
        onBookUpdated((row) => setBooks((prev) => mergeBook(prev, row))),
        onScanStatus((s) => {
          setStatus(s);
          if (s.phase === "idle") refreshCounts(); // sweep finished
        }),
        onLibraryRecovered(() => setRecovered(true)),
      ]);
    })();
    return () => unlisteners.forEach((u) => u());
  }, [refreshCounts]);

  // Read the saved "show first" preference once, before the first load.
  useEffect(() => {
    getSetting("default_library")
      .then((v) => {
        const first: LibraryKind = v === "ebooks" ? "ebooks" : "comics";
        setFirstLibrary(first);
        setLibrary(first);
      })
      .catch((e) => console.error("read default library failed", e))
      .finally(() => setBooted(true));
  }, []);

  // Load (and reload) the active library's data.
  useEffect(() => {
    if (!booted) return;
    (async () => {
      await loadLibrary(library);
      setReady(true);
    })();
  }, [booted, library, loadLibrary]);

  const handleSetFirstLibrary = useCallback((lib: LibraryKind) => {
    setFirstLibrary(lib);
    setLibrary(lib);
    setSetting("default_library", lib).catch((e) =>
      console.error("save default library failed", e),
    );
  }, []);

  const doAdd = useCallback(
    async (path: string, mode: FolderMode) => {
      setPendingAdd(null);
      setStatus({ phase: "scanning", current: 0, total: 0 });
      const bs = await addFolder(path, mode, library);
      setBooks(sortBooks(bs));
      setFolders(await listFolders(library));
    },
    [library],
  );

  const handleAddFolder = useCallback(async () => {
    const folder = await pickFolder();
    if (!folder) return;
    const probe = await probeFolder(folder, library);
    if (probe.nested > 0 && probe.subfolders > 0) {
      setPendingAdd({ path: folder, probe });
    } else {
      await doAdd(folder, "tree");
    }
  }, [doAdd, library]);

  const handleRemoveFolder = useCallback(
    async (folder: string) => {
      const bs = await removeFolder(folder, library);
      setBooks(sortBooks(bs));
      setFolders(await listFolders(library));
    },
    [library],
  );

  const handleRemoveBook = useCallback(
    async (path: string) => setBooks(sortBooks(await removeBook(path, library))),
    [library],
  );

  const handleRemovePath = useCallback(
    async (path: string) => setBooks(sortBooks(await removePath(path, library))),
    [library],
  );

  // Re-walk the folders for added / removed / changed files; only new or
  // changed books are swept. Covers and metadata of unchanged books are kept.
  const handleRescan = useCallback(async () => {
    setStatus({ phase: "scanning", current: 0, total: 0 });
    setBooks(sortBooks(await rescan(library)));
  }, [library]);

  // Re-walk the folders, then reset every book so covers, page counts and
  // hashes are rebuilt from scratch with the current code.
  const handleReindex = useCallback(async () => {
    setStatus({ phase: "scanning", current: 0, total: 0 });
    setBooks(sortBooks(await reindex(library)));
  }, [library]);

  const handlePause = useCallback(() => {
    pauseIndexing().catch((e) => console.error("pause failed", e));
  }, []);

  const handleResume = useCallback(() => {
    resumeIndexing().catch((e) => console.error("resume failed", e));
  }, []);

  const handlePageChange = useCallback(
    (page: number) => {
      if (!openBook) return;
      setProgress(openBook.path, page).catch((e) =>
        console.error("save progress failed", e),
      );
      setBooks((prev) =>
        prev.map((b) =>
          b.path === openBook.path ? { ...b, last_page: page } : b,
        ),
      );
    },
    [openBook],
  );

  if (!ready) {
    return <div className="boot">Loading library…</div>;
  }

  const back = () => setOpenBook(null);

  return (
    <>
      {recovered && (
        <div className="recovery-banner" role="status">
          <span>
            Your library file was damaged and has been rebuilt. Your folders are
            back and the books are being re-scanned now — covers and reading
            positions will repopulate. The old file was kept alongside it.
          </span>
          <button className="btn small" onClick={() => setRecovered(false)}>
            Dismiss
          </button>
        </div>
      )}

      {/* Library stays mounted so scroll, folder location and selection persist
          when you open a book or Settings and come back. */}
      <Library
        books={books}
        folders={folders}
        status={status}
        library={library}
        firstLibrary={firstLibrary}
        comicsCount={counts.comics}
        ebooksCount={counts.ebooks}
        onSwitchLibrary={setLibrary}
        onOpenSettings={() => setSettingsOpen(true)}
        onRescan={handleRescan}
        onReindex={handleReindex}
        onAddFolder={handleAddFolder}
        onRemoveFolder={handleRemoveFolder}
        onRemoveBook={handleRemoveBook}
        onRemovePath={handleRemovePath}
        onPause={handlePause}
        onResume={handleResume}
        onBooksChanged={(bs) => setBooks(sortBooks(bs))}
        onOpenBook={setOpenBook}
      />

      {pendingAdd && (
        <AddFolderDialog
          path={pendingAdd.path}
          probe={pendingAdd.probe}
          onChoose={(mode) => doAdd(pendingAdd.path, mode)}
          onCancel={() => setPendingAdd(null)}
        />
      )}

      {settingsOpen && (
        <div className="overlay-full">
          <Settings
            library={library}
            firstLibrary={firstLibrary}
            onSetFirstLibrary={handleSetFirstLibrary}
            onClose={() => setSettingsOpen(false)}
            onBooksChanged={(bs) => setBooks(sortBooks(bs))}
          />
        </div>
      )}

      {openBook && (
        <div className="overlay-full">
          {isComic(openBook.format) ? (
            <Reader
              book={openBook}
              initialPage={openBook.last_page}
              onBack={back}
              onPageChange={handlePageChange}
            />
          ) : (
            <EbookReader
              book={openBook}
              initialPage={openBook.last_page}
              onBack={back}
              onPageChange={handlePageChange}
            />
          )}
        </div>
      )}
    </>
  );
}

export default App;
