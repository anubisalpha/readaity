import type { ScanStatus } from "../types";
import { RefreshMenu } from "./RefreshMenu";

/** The aity brand mark — matches the app icon and aity.uk's favicon. */
function BrandMark() {
  return (
    <svg
      className="brand-mark"
      viewBox="0 0 32 32"
      width="22"
      height="22"
      aria-hidden="true"
    >
      <rect width="32" height="32" rx="8" fill="#12172B" />
      <text
        x="16"
        y="23"
        fontFamily="Arial, Helvetica, sans-serif"
        fontWeight="700"
        fontSize="20"
        fill="#2FD3B8"
        textAnchor="middle"
      >
        a
      </text>
      <rect x="9" y="25" width="14" height="2" rx="1" fill="#2FD3B8" />
    </svg>
  );
}

interface Props {
  status: ScanStatus;
  comicsCount: number;
  ebooksCount: number;
  onPause: () => void;
  onResume: () => void;
  onAddFolder: () => void;
  onRescan: () => void;
  onReindex: () => void;
  onOpenSettings: () => void;
}

/**
 * The one and only header strip: Readaity wordmark on the left, a live
 * status / book-count readout next to it, actions on the right, and a hairline
 * progress bar along the bottom edge while a scan or index is running.
 */
export function AppHeader({
  status,
  comicsCount,
  ebooksCount,
  onPause,
  onResume,
  onAddFolder,
  onRescan,
  onReindex,
  onOpenSettings,
}: Props) {
  const { phase, current, total } = status;
  const pct =
    (phase === "indexing" || phase === "paused") && total > 0
      ? Math.round((current / total) * 100)
      : 0;
  const left = Math.max(0, total - current);

  const label =
    phase === "scanning"
      ? "Scanning folders…"
      : phase === "indexing"
        ? `Indexing files… ${current} / ${total} · ${pct}%`
        : phase === "paused"
          ? `Indexing paused · ${left} remaining`
          : `${comicsCount} comic${comicsCount === 1 ? "" : "s"} · ${ebooksCount} book${ebooksCount === 1 ? "" : "s"}`;

  return (
    <header className={`app-header ${phase}`}>
      <div className="app-logo" aria-label="Readaity">
        <BrandMark />
        Read<span className="brand-a">a</span>ity
      </div>

      <div className="app-status">
        <span className="status-dot" />
        <span className="status-label">{label}</span>
        {phase === "indexing" && (
          <button className="status-btn" onClick={onPause} title="Pause indexing">
            ❚❚
          </button>
        )}
        {phase === "paused" && (
          <button
            className="status-btn"
            onClick={onResume}
            title="Resume indexing"
          >
            ▶
          </button>
        )}
      </div>

      <div className="app-header-actions">
        <button className="btn primary" onClick={onAddFolder}>
          ＋ Add folder
        </button>
        <RefreshMenu onRescan={onRescan} onReindex={onReindex} />
        <button
          className="btn ghost icon"
          onClick={onOpenSettings}
          title="Settings"
          aria-label="Settings"
        >
          ⚙
        </button>
      </div>

      {(phase === "scanning" || phase === "indexing" || phase === "paused") && (
        <span
          className={`app-header-progress${
            phase === "scanning" ? " indeterminate" : ""
          }`}
          style={phase === "scanning" ? undefined : { width: `${pct}%` }}
        />
      )}
    </header>
  );
}
