import { useEffect, useState } from "react";
import type { ScanStatus } from "../types";
import { appVersion, shareStatus } from "../lib/api";

interface Props {
  status: ScanStatus;
  onOpenNetwork: () => void;
}

/**
 * Thin bar pinned to the bottom of the app (mirrors Narraity's status bar):
 * app identity + version on the left, a row of subsystem status icons on the
 * right — nearest the content they describe. Each icon's colour carries the
 * meaning; the tooltip explains it.
 */
export function Footer({ status, onOpenNetwork }: Props) {
  const [version, setVersion] = useState<string | null>(null);
  const [sharing, setSharing] = useState<{ on: boolean; url: string | null }>({
    on: false,
    url: null,
  });

  useEffect(() => {
    appVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  // Poll the share server state — it can be started/stopped from Settings or
  // the Network view, and toggles itself on network changes.
  useEffect(() => {
    let alive = true;
    const check = () =>
      shareStatus()
        .then((s) => {
          if (alive) setSharing({ on: s.running, url: s.urls[0] ?? null });
        })
        .catch(() => {
          if (alive) setSharing({ on: false, url: null });
        });
    check();
    const id = setInterval(check, 15000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const scan =
    status.phase === "idle"
      ? { color: "var(--ok, #4caf50)", label: "Library idle — nothing scanning" }
      : status.phase === "paused"
        ? { color: "#e0a33e", label: "Indexing paused" }
        : {
            color: "#e0a33e",
            label:
              status.phase === "scanning"
                ? "Scanning folders for changes…"
                : `Indexing files… ${status.current} / ${status.total}`,
          };

  return (
    <footer className="app-footer">
      <span className="footer-id">
        Readaity{version ? ` v${version}` : ""} · © 2026 Anubis Productions
      </span>
      <span className="footer-status">
        <button
          type="button"
          className="footer-icon"
          onClick={onOpenNetwork}
          title={
            sharing.on
              ? `Network sharing: on${sharing.url ? ` — ${sharing.url}` : ""}`
              : "Network sharing: off"
          }
          aria-label="Network sharing status"
          style={{ color: sharing.on ? "var(--accent)" : "var(--text-dim)" }}
        >
          🖧
        </button>
        <span
          className="footer-dot"
          title={scan.label}
          style={{ background: scan.color }}
        />
      </span>
    </footer>
  );
}
