import { useCallback, useEffect, useMemo, useState } from "react";
import type { BookRow, LibraryKind, Peer, PeerBook } from "../types";
import {
  listFolders,
  onPeerImportStatus,
  peerBooks,
  peerBrowse,
  peerCheck,
  peerImport,
  peerTrust,
} from "../lib/api";

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}
function fmtSize(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1048576).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

type Step = "peers" | "trust" | "pin" | "browse";

interface Props {
  library: LibraryKind;
  onImported: (books: BookRow[]) => void;
  onClose: () => void;
}

/** Discover other Readaity instances on the LAN and import books from them. */
export function NetworkView({ library, onImported, onClose }: Props) {
  const [step, setStep] = useState<Step>("peers");
  const [scanning, setScanning] = useState(false);
  const [peers, setPeers] = useState<Peer[]>([]);
  const [err, setErr] = useState<string | null>(null);

  const [peer, setPeer] = useState<Peer | null>(null);
  const [fingerprint, setFingerprint] = useState("");
  const [pin, setPin] = useState("");
  const [lib, setLib] = useState<LibraryKind>(library);
  const [books, setBooks] = useState<PeerBook[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [folders, setFolders] = useState<string[]>([]);
  const [dest, setDest] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null,
  );

  useEffect(() => {
    listFolders(library)
      .then((fs) => {
        const paths = fs.map((f) => f.path);
        setFolders(paths);
        if (paths[0]) setDest(paths[0]);
      })
      .catch(() => {});
  }, [library]);

  useEffect(() => {
    let off: (() => void) | undefined;
    onPeerImportStatus(setProgress).then((u) => (off = u));
    return () => off?.();
  }, []);

  const scan = useCallback(async () => {
    setScanning(true);
    setErr(null);
    try {
      setPeers(await peerBrowse());
    } catch (e) {
      setErr(String(e));
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    scan();
  }, [scan]);

  const openPeer = useCallback(async (p: Peer) => {
    setPeer(p);
    setErr(null);
    setBusy(true);
    try {
      const check = await peerCheck(p.host || p.addr, p.port);
      setFingerprint(check.fingerprint);
      setStep(check.trusted ? "pin" : "trust");
    } catch (e) {
      setErr(String(e));
      setStep("peers");
    } finally {
      setBusy(false);
    }
  }, []);

  const trustAndContinue = useCallback(async () => {
    if (!peer) return;
    setBusy(true);
    try {
      await peerTrust(peer.host || peer.addr, fingerprint);
      setStep("pin");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }, [peer, fingerprint]);

  const loadBooks = useCallback(
    async (which: LibraryKind) => {
      if (!peer) return;
      setBusy(true);
      setErr(null);
      try {
        const list = await peerBooks(peer.host || peer.addr, peer.port, pin, which);
        setBooks(list);
        setLib(which);
        setPicked(new Set());
        setStep("browse");
      } catch (e) {
        setErr(String(e));
      } finally {
        setBusy(false);
      }
    },
    [peer, pin],
  );

  const importable = useMemo(
    () => books.filter((b) => picked.has(b.id) && !b.dupe),
    [books, picked],
  );

  const runImport = useCallback(async () => {
    if (!peer || !dest || importable.length === 0) return;
    setBusy(true);
    setErr(null);
    setProgress({ done: 0, total: importable.length });
    try {
      onImported(
        await peerImport(
          peer.host || peer.addr,
          peer.port,
          pin,
          library,
          importable.map((b) => b.id),
          dest,
        ),
      );
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }, [peer, dest, importable, pin, library, onImported, onClose]);

  return (
    <div className="settings">
      <header className="settings-header">
        <button className="btn ghost" onClick={onClose}>
          ‹ Library
        </button>
        <h1>Network</h1>
        <div style={{ width: 80 }} />
      </header>

      <div className="netview">
        {err && <p className="sharing-err">{err}</p>}

        {step === "peers" && (
          <>
            <div className="settings-row-actions">
              <p className="settings-hint">
                Other devices running Readaity with sharing turned on show up
                here.
              </p>
              <button className="btn" onClick={scan} disabled={scanning}>
                {scanning ? "Scanning…" : "Scan again"}
              </button>
            </div>
            {peers.length === 0 && !scanning ? (
              <p className="settings-empty">
                No devices found. Make sure the other device has sharing on and
                is on the same network.
              </p>
            ) : (
              <ul className="peer-list">
                {peers.map((p) => (
                  <li key={`${p.addr}:${p.port}`}>
                    <button
                      className="peer-row"
                      onClick={() => openPeer(p)}
                      disabled={busy}
                    >
                      <span className="peer-name">{p.name}</span>
                      <span className="peer-addr">
                        {p.addr}:{p.port} · v{p.version}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}

        {step === "trust" && peer && (
          <div className="peer-panel">
            <h2>Trust “{peer.name}”?</h2>
            <p className="settings-hint">
              This is the first time you’ve connected to this device. Check that
              this fingerprint matches the one shown in its Network sharing
              settings, then trust it.
            </p>
            <code className="sharing-fp">{fingerprint}</code>
            <div className="modal-actions">
              <button className="btn ghost" onClick={() => setStep("peers")}>
                Back
              </button>
              <button
                className="btn primary"
                onClick={trustAndContinue}
                disabled={busy}
              >
                Trust &amp; continue
              </button>
            </div>
          </div>
        )}

        {step === "pin" && peer && (
          <div className="peer-panel">
            <h2>Enter the PIN for “{peer.name}”</h2>
            <p className="settings-hint">
              The access PIN set on the other device. It’s kept only for this
              session.
            </p>
            <div className="sharing-row">
              <input
                className="sharing-input"
                type="text"
                inputMode="numeric"
                maxLength={10}
                placeholder="6–10 digits"
                value={pin}
                onChange={(e) => setPin(e.target.value.replace(/\D/g, ""))}
              />
              <button
                className="btn primary"
                onClick={() => loadBooks(library)}
                disabled={busy || pin.length < 6}
              >
                Connect
              </button>
              <button className="btn ghost" onClick={() => setStep("peers")}>
                Back
              </button>
            </div>
          </div>
        )}

        {step === "browse" && peer && (
          <div className="peer-panel">
            <div className="settings-row-actions">
              <h2 style={{ margin: 0 }}>{peer.name}</h2>
              <div className="seg">
                {(["comics", "ebooks"] as LibraryKind[]).map((k) => (
                  <button
                    key={k}
                    className={`seg-btn${lib === k ? " active" : ""}`}
                    onClick={() => loadBooks(k)}
                    disabled={busy}
                  >
                    {k === "comics" ? "Comics" : "Ebooks"}
                  </button>
                ))}
              </div>
            </div>

            {books.length === 0 ? (
              <p className="settings-empty">Nothing shared in this library.</p>
            ) : (
              <ul className="peer-books">
                {books.map((b) => (
                  <li key={b.id} className={b.dupe ? "dupe" : ""}>
                    <label>
                      <input
                        type="checkbox"
                        disabled={b.dupe}
                        checked={picked.has(b.id)}
                        onChange={(e) =>
                          setPicked((s) => {
                            const n = new Set(s);
                            if (e.target.checked) n.add(b.id);
                            else n.delete(b.id);
                            return n;
                          })
                        }
                      />
                      <span className="pb-title">{b.title}</span>
                      <span className={`format-tag ${b.format}`}>{b.format}</span>
                      <span className="pb-size">{fmtSize(b.size)}</span>
                      {b.dupe && <span className="pb-dupe">already in your library</span>}
                    </label>
                  </li>
                ))}
              </ul>
            )}

            <div className="peer-import-bar">
              <label className="settings-field-label">
                Import {importable.length} book
                {importable.length === 1 ? "" : "s"} into
              </label>
              <select
                className="sharing-input"
                value={dest}
                onChange={(e) => setDest(e.target.value)}
              >
                {folders.length === 0 && <option value="">no folders yet</option>}
                {folders.map((f) => (
                  <option key={f} value={f}>
                    {baseName(f)} ({library === "comics" ? "Comics" : "Ebooks"})
                  </option>
                ))}
              </select>
              <button
                className="btn primary"
                onClick={runImport}
                disabled={busy || !dest || importable.length === 0}
              >
                {progress
                  ? `Importing ${progress.done}/${progress.total}…`
                  : "Import"}
              </button>
              <button className="btn ghost" onClick={() => setStep("peers")}>
                Done
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
