import { useCallback, useEffect, useState } from "react";
import "./App.css";
import type {
  BookRow,
  FolderInfo,
  FolderMode,
  ImportPlan,
  LibraryKind,
  ProbeResult,
  ReaderPrefs,
  ScanStatus,
} from "./types";
import {
  DEFAULT_READER_PREFS,
  parseReaderPrefs,
} from "./lib/readerTheme";
import {
  addFolder,
  getSetting,
  importBooks,
  libraryCounts,
  listBooks,
  listFolders,
  pickBookFiles,
  suggestImport,
  onBookUpdated,
  onLibraryRecovered,
  onScanStatus,
  pauseIndexing,
  pickFolder,
  probeFolder,
  clearBeingRead,
  markOpened,
  reindex,
  removeBook,
  removeFolder,
  removePath,
  rescan,
  resumeIndexing,
  setFavorite,
  setBookLibrary,
  folderLayoutSplit,
  splitFolderLibraries,
  setProgress,
  setSetting,
} from "./lib/api";
import { AppHeader } from "./components/AppHeader";
import { Library } from "./components/Library";
import { Reader } from "./components/Reader";
import { EbookReader } from "./components/EbookReader";
import { AddFolderDialog } from "./components/AddFolderDialog";
import { AddBooksDialog } from "./components/AddBooksDialog";
import { NetworkView } from "./components/NetworkView";
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
  // After adding an Ebooks folder that turned out to hold comic-format azw3.
  const [splitPrompt, setSplitPrompt] = useState<{
    path: string;
    fixed: number;
    other: number;
  } | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [networkOpen, setNetworkOpen] = useState(false);
  const [importPlans, setImportPlans] = useState<ImportPlan[] | null>(null);
  const [importBusy, setImportBusy] = useState(false);
  const [counts, setCounts] = useState({ comics: 0, ebooks: 0 });
  const [recovered, setRecovered] = useState(false);
  const [readerPrefs, setReaderPrefs] = useState<ReaderPrefs>(DEFAULT_READER_PREFS);

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

  // Load the persisted reading-appearance preferences once.
  useEffect(() => {
    Promise.all([getSetting("reader_theme"), getSetting("reader_font_scale")])
      .then(([theme, scale]) => setReaderPrefs(parseReaderPrefs(theme, scale)))
      .catch((e) => console.error("read reader prefs failed", e));
  }, []);

  const handleReaderPrefsChange = useCallback((next: ReaderPrefs) => {
    setReaderPrefs(next);
    setSetting("reader_theme", next.theme).catch(() => {});
    setSetting("reader_font_scale", String(next.fontScale)).catch(() => {});
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
      // Comic-format azw3 scanned into Ebooks — offer to route them to Comics.
      if (library === "ebooks") {
        try {
          const [fixed, other] = await folderLayoutSplit(path, library);
          if (fixed > 0 && other > 0) setSplitPrompt({ path, fixed, other });
        } catch (e) {
          console.error("layout split probe failed", e);
        }
      }
    },
    [library],
  );

  const applySplit = useCallback(async () => {
    if (!splitPrompt) return;
    const { path } = splitPrompt;
    setSplitPrompt(null);
    try {
      setBooks(sortBooks(await splitFolderLibraries(path, library)));
      setFolders(await listFolders(library));
    } catch (e) {
      console.error("split failed", e);
    }
  }, [splitPrompt, library]);

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

  const handleAddBooks = useCallback(async () => {
    try {
      const files = await pickBookFiles(library);
      if (!files.length) return;
      setImportPlans(await suggestImport(files, library));
    } catch (e) {
      console.error("pick books failed", e);
    }
  }, [library]);

  const doImport = useCallback(
    async (items: { path: string; dest: string }[]) => {
      setImportBusy(true);
      try {
        setBooks(sortBooks(await importBooks(items, library)));
        setFolders(await listFolders(library));
        setImportPlans(null);
      } catch (e) {
        console.error("import failed", e);
      } finally {
        setImportBusy(false);
      }
    },
    [library],
  );

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

  const handleToggleFavorite = useCallback(
    async (path: string) => {
      const book = books.find((b) => b.path === path);
      const next = !(book?.favorite ?? false);
      setBooks((prev) =>
        prev.map((b) => (b.path === path ? { ...b, favorite: next } : b)),
      );
      try {
        setBooks(sortBooks(await setFavorite(path, next, library)));
      } catch (e) {
        console.error("favourite failed", e);
      }
    },
    [books, library],
  );

  const handleMoveLibrary = useCallback(
    async (path: string, to: LibraryKind) => {
      // The book leaves this library's list.
      setBooks((prev) => prev.filter((b) => b.path !== path));
      try {
        setBooks(sortBooks(await setBookLibrary(path, to, library)));
      } catch (e) {
        console.error("move library failed", e);
      }
    },
    [library],
  );

  const handleClearBeingRead = useCallback(
    async (path: string) => {
      setBooks((prev) =>
        prev.map((b) => (b.path === path ? { ...b, last_opened: null } : b)),
      );
      try {
        setBooks(sortBooks(await clearBeingRead(path, library)));
      } catch (e) {
        console.error("remove from being-read failed", e);
      }
    },
    [library],
  );

  const handleOpenBook = useCallback((book: BookRow) => {
    setOpenBook(book);
    const stamp = Math.floor(Date.now() / 1000);
    setBooks((prev) =>
      prev.map((b) => (b.path === book.path ? { ...b, last_opened: stamp } : b)),
    );
    markOpened(book.path).catch((e) => console.error("mark opened failed", e));
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
      <div className="app-shell">
        {recovered && (
          <div className="recovery-banner" role="status">
            <span>
              Your library file was damaged and has been rebuilt. Your folders
              are back and the books are being re-scanned now — covers and
              reading positions will repopulate. The old file was kept alongside
              it.
            </span>
            <button className="btn small" onClick={() => setRecovered(false)}>
              Dismiss
            </button>
          </div>
        )}

        <AppHeader
          status={status}
          comicsCount={counts.comics}
          ebooksCount={counts.ebooks}
          onPause={handlePause}
          onResume={handleResume}
          onAddFolder={handleAddFolder}
          onAddBooks={handleAddBooks}
          onRescan={handleRescan}
          onReindex={handleReindex}
          onOpenSettings={() => setSettingsOpen(true)}
        />

        {/* Library stays mounted so scroll, folder location and selection
            persist when you open a book or Settings and come back. */}
        <Library
          books={books}
          folders={folders}
          status={status}
          library={library}
          firstLibrary={firstLibrary}
          onSwitchLibrary={setLibrary}
          onAddFolder={handleAddFolder}
          onRemoveFolder={handleRemoveFolder}
          onRemoveBook={handleRemoveBook}
          onRemovePath={handleRemovePath}
          onBooksChanged={(bs) => setBooks(sortBooks(bs))}
          onOpenBook={handleOpenBook}
          onToggleFavorite={handleToggleFavorite}
          onMoveLibrary={handleMoveLibrary}
          onClearBeingRead={handleClearBeingRead}
          onOpenNetwork={() => setNetworkOpen(true)}
        />
      </div>

      {pendingAdd && (
        <AddFolderDialog
          path={pendingAdd.path}
          probe={pendingAdd.probe}
          onChoose={(mode) => doAdd(pendingAdd.path, mode)}
          onCancel={() => setPendingAdd(null)}
        />
      )}

      {importPlans && (
        <AddBooksDialog
          library={library}
          plans={importPlans}
          busy={importBusy}
          onImport={doImport}
          onAddFolderFirst={() => {
            setImportPlans(null);
            handleAddFolder();
          }}
          onCancel={() => setImportPlans(null)}
        />
      )}

      {splitPrompt && (
        <div className="modal-overlay" onClick={() => setSplitPrompt(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2 className="modal-title">This folder has comics in it</h2>
            <p className="modal-sub">
              {splitPrompt.fixed} of these books are fixed-layout (comics, manga,
              picture books) and {splitPrompt.other} are regular ebooks.
              Fixed-layout books read best in your Comics library with
              page-turning — the ebooks stay here.
            </p>
            <div className="modal-options">
              <button
                className="modal-option"
                onClick={applySplit}
                style={{ borderColor: "var(--accent)" }}
              >
                <span className="opt-title">
                  Move {splitPrompt.fixed} to Comics (recommended)
                </span>
                <span className="opt-desc">
                  Comics go to your Comics library; ebooks stay in Ebooks.
                </span>
              </button>
              <button
                className="modal-option"
                onClick={() => setSplitPrompt(null)}
              >
                <span className="opt-title">Keep all in Ebooks</span>
                <span className="opt-desc">
                  You can still move them individually later.
                </span>
              </button>
            </div>
          </div>
        </div>
      )}

      {networkOpen && (
        <div className="overlay-full">
          <NetworkView
            library={library}
            onImported={(bs) => setBooks(sortBooks(bs))}
            onClose={() => setNetworkOpen(false)}
          />
        </div>
      )}

      {settingsOpen && (
        <div className="overlay-full">
          <Settings
            library={library}
            firstLibrary={firstLibrary}
            onSetFirstLibrary={handleSetFirstLibrary}
            readerPrefs={readerPrefs}
            onReaderPrefsChange={handleReaderPrefsChange}
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
              prefs={readerPrefs}
              onBack={back}
              onPageChange={handlePageChange}
              suggestComics={openBook.fixed_layout && library === "ebooks"}
              onMoveToComics={() => {
                handleMoveLibrary(openBook.path, "comics");
                back();
              }}
            />
          )}
        </div>
      )}
    </>
  );
}

export default App;
