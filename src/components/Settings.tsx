import { useCallback, useEffect, useState } from "react";
import type { BookRow, DupGroup, LibraryKind } from "../types";
import {
  clearExclusions,
  ignoreDupe,
  listDuplicates,
  listExclusions,
  listIgnoredDupes,
  listNameDuplicates,
  removeBook,
  restoreExclusion,
  unignoreDupe,
} from "../lib/api";

interface Props {
  library: LibraryKind;
  onClose: () => void;
  onBooksChanged: (books: BookRow[]) => void;
}

type Tab = "removed" | "exact" | "similar";

function splitPath(p: string): { name: string; dir: string } {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return { name: parts[parts.length - 1] ?? p, dir: parts.slice(0, -1).join("/") };
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function Settings({ library, onClose, onBooksChanged }: Props) {
  const [tab, setTab] = useState<Tab>("removed");
  const [exclusions, setExclusions] = useState<string[]>([]);
  const [exact, setExact] = useState<DupGroup[]>([]);
  const [similar, setSimilar] = useState<DupGroup[]>([]);
  const [ignored, setIgnored] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const [ex, dup, name, ign] = await Promise.all([
      listExclusions(),
      listDuplicates(),
      listNameDuplicates(),
      listIgnoredDupes(),
    ]);
    setExclusions(ex);
    setExact(dup);
    setSimilar(name);
    setIgnored(ign);
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const withBusy = useCallback(
    async (fn: () => Promise<void>) => {
      setBusy(true);
      try {
        await fn();
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const restore = (path: string) =>
    withBusy(async () => {
      onBooksChanged(await restoreExclusion(path, library));
      await refresh();
    });

  const restoreAll = () =>
    withBusy(async () => {
      onBooksChanged(await clearExclusions(library));
      await refresh();
    });

  const removeOne = (path: string) =>
    withBusy(async () => {
      onBooksChanged(await removeBook(path, library));
      await refresh();
    });

  // Remove every copy in a group except the first (largest / suggested keep).
  const removeOthers = (group: DupGroup) =>
    withBusy(async () => {
      let latest: BookRow[] | undefined;
      for (const b of group.books.slice(1))
        latest = await removeBook(b.path, library);
      if (latest) onBooksChanged(latest);
      await refresh();
    });

  const ignore = (key: string) =>
    withBusy(async () => {
      await ignoreDupe(key);
      await refresh();
    });

  const restoreIgnore = (key: string) =>
    withBusy(async () => {
      await unignoreDupe(key);
      await refresh();
    });

  const NAV: { id: Tab; label: string; count: number }[] = [
    { id: "removed", label: "Removed from library", count: exclusions.length },
    { id: "exact", label: "Exact duplicates", count: exact.length },
    { id: "similar", label: "Possible duplicates", count: similar.length },
  ];

  return (
    <div className="settings">
      <header className="settings-header">
        <button className="btn ghost" onClick={onClose}>
          ‹ Library
        </button>
        <h1>Settings</h1>
        <div style={{ width: 80 }} />
      </header>

      <div className="settings-layout">
        <nav className="settings-nav">
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`settings-navitem${tab === n.id ? " active" : ""}`}
              onClick={() => setTab(n.id)}
            >
              <span>{n.label}</span>
              {n.count > 0 && <span className="tab-badge">{n.count}</span>}
            </button>
          ))}
        </nav>

        <div className="settings-body">
          {tab === "removed" && (
            <RemovedTab
              exclusions={exclusions}
              busy={busy}
              onRestore={restore}
              onRestoreAll={restoreAll}
            />
          )}
          {tab === "exact" && (
            <ExactTab groups={exact} busy={busy} onRemove={removeOne} />
          )}
          {tab === "similar" && (
            <SimilarTab
              groups={similar}
              ignored={ignored}
              busy={busy}
              onRemove={removeOne}
              onRemoveOthers={removeOthers}
              onIgnore={ignore}
              onRestoreIgnore={restoreIgnore}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function RemovedTab({
  exclusions,
  busy,
  onRestore,
  onRestoreAll,
}: {
  exclusions: string[];
  busy: boolean;
  onRestore: (p: string) => void;
  onRestoreAll: () => void;
}) {
  if (exclusions.length === 0)
    return <p className="settings-empty">Nothing has been removed from the library.</p>;
  return (
    <>
      <div className="settings-row-actions">
        <span className="settings-hint">
          These paths are excluded from rescans. Files remain on disk.
        </span>
        <button className="btn" onClick={onRestoreAll} disabled={busy}>
          Restore all
        </button>
      </div>
      <ul className="excl-list">
        {exclusions.map((p) => {
          const { name, dir } = splitPath(p);
          return (
            <li key={p} className="excl-item">
              <div className="excl-path">
                <span className="excl-name">{name}</span>
                <span className="excl-dir">{dir}</span>
              </div>
              <button className="btn" onClick={() => onRestore(p)} disabled={busy}>
                Restore
              </button>
            </li>
          );
        })}
      </ul>
    </>
  );
}

function ExactTab({
  groups,
  busy,
  onRemove,
}: {
  groups: DupGroup[];
  busy: boolean;
  onRemove: (p: string) => void;
}) {
  if (groups.length === 0)
    return <p className="settings-empty">No byte-identical files found.</p>;
  return (
    <>
      <p className="settings-hint">
        Each group is byte-identical (same content hash). Remove the extra copies
        from the library — files stay on disk.
      </p>
      <div className="dup-groups">
        {groups.map((g) => (
          <div key={g.key} className="dup-group">
            <div className="dup-head">
              {g.books.length} identical copies · {g.books[0]?.title}
            </div>
            <ul className="dup-copies">
              {g.books.map((b) => {
                const { dir } = splitPath(b.path);
                return (
                  <li key={b.path} className="dup-copy">
                    <span className={`format-tag ${b.format}`}>{b.format}</span>
                    <span className="dup-dir" title={b.path}>
                      {dir}
                    </span>
                    <span className="dup-size">{formatSize(b.size)}</span>
                    <button
                      className="btn small"
                      onClick={() => onRemove(b.path)}
                      disabled={busy}
                    >
                      Remove
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    </>
  );
}

function SimilarTab({
  groups,
  ignored,
  busy,
  onRemove,
  onRemoveOthers,
  onIgnore,
  onRestoreIgnore,
}: {
  groups: DupGroup[];
  ignored: string[];
  busy: boolean;
  onRemove: (p: string) => void;
  onRemoveOthers: (g: DupGroup) => void;
  onIgnore: (key: string) => void;
  onRestoreIgnore: (key: string) => void;
}) {
  return (
    <>
      <p className="settings-hint">
        These look like the same issue by filename (different scans / editions).
        This is a fuzzy match — check before removing. The largest file (usually
        the best scan) is suggested to keep. Use <strong>Ignore</strong> to
        permanently hide a group that isn't really a duplicate.
      </p>

      {groups.length === 0 ? (
        <p className="settings-empty">No possible duplicates found by filename.</p>
      ) : (
        <div className="dup-groups">
          {groups.map((g) => (
            <div key={g.key} className="dup-group">
              <div className="dup-head dup-head-row">
                <span>
                  {g.books.length} possible copies · {prettyKey(g.key)}
                </span>
                <span className="dup-head-actions">
                  <button
                    className="btn small ghost"
                    onClick={() => onIgnore(g.key)}
                    disabled={busy}
                    title="Never show this group again"
                  >
                    Ignore
                  </button>
                  {g.books.length > 1 && (
                    <button
                      className="btn small"
                      onClick={() => onRemoveOthers(g)}
                      disabled={busy}
                      title="Remove all but the largest from the library"
                    >
                      Remove all but largest
                    </button>
                  )}
                </span>
              </div>
            <ul className="dup-copies">
              {g.books.map((b, i) => {
                const { name, dir } = splitPath(b.path);
                return (
                  <li key={b.path} className="dup-copy">
                    <span className={`format-tag ${b.format}`}>{b.format}</span>
                    {i === 0 ? (
                      <span className="keep-badge">Keep · largest</span>
                    ) : (
                      <span className="keep-spacer" />
                    )}
                    <span className="dup-name" title={b.path}>
                      {name}
                      <span className="dup-dir-sub">{dir}</span>
                    </span>
                    <span className="dup-size">{formatSize(b.size)}</span>
                    <button
                      className="btn small"
                      onClick={() => onRemove(b.path)}
                      disabled={busy}
                    >
                      Remove
                    </button>
                  </li>
                );
              })}
            </ul>
            </div>
          ))}
        </div>
      )}

      {ignored.length > 0 && (
        <details className="ignored-note">
          <summary>
            {ignored.length} ignored group{ignored.length > 1 ? "s" : ""}
          </summary>
          <ul className="excl-list">
            {ignored.map((k) => (
              <li key={k} className="excl-item">
                <span className="excl-name">{prettyKey(k)}</span>
                <button
                  className="btn small"
                  onClick={() => onRestoreIgnore(k)}
                  disabled={busy}
                >
                  Restore
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}
    </>
  );
}

/** "farscape#0001" → "Farscape #1" */
function prettyKey(key: string): string {
  const [series, issue] = key.split("#");
  const title = series.replace(/\b\w/g, (c) => c.toUpperCase());
  const num = issue ? String(parseInt(issue, 10)) : "";
  return num ? `${title} #${num}` : title;
}
