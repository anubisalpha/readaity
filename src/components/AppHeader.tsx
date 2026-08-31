import type { ScanStatus } from "../types";
import { RefreshMenu } from "./RefreshMenu";

/** The books badge — matches the app icon and the aity.uk hero badge. */
function BrandMark() {
  return (
    <span className="brand-mark" aria-hidden="true">
      📚
    </span>
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
