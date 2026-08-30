import type { ScanStatus } from "../types";

interface Props {
  status: ScanStatus;
  comicsCount: number;
  booksCount: number;
  onPause: () => void;
  onResume: () => void;
}

/**
 * Persistent top bar showing what the app is doing, with a pause/resume control
 * for the indexing sweep:
 *   idle      → "Idle · N books"
 *   scanning  → "Scanning folders…", indeterminate track
 *   indexing  → "Indexing files… c / t" + Pause, determinate fill
 *   paused    → "Paused · N left" + Resume
 */
export function StatusBar({
  status,
  comicsCount,
  booksCount,
  onPause,
  onResume,
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
        ? `Indexing files… ${current} / ${total}`
        : phase === "paused"
          ? `Indexing paused · ${left} remaining`
          : `${comicsCount} comic${comicsCount === 1 ? "" : "s"} · ${booksCount} book${booksCount === 1 ? "" : "s"}`;

  return (
    <div className={`status-bar ${phase}`}>
      <div className="status-row">
        <span className="status-dot" />
        <span className="status-label">{label}</span>
        {(phase === "indexing" || phase === "paused") && (
          <span className="status-pct">{pct}%</span>
        )}
        {phase === "indexing" && (
          <button className="status-btn" onClick={onPause} title="Pause indexing">
            ❚❚ Pause
          </button>
        )}
        {phase === "paused" && (
          <button className="status-btn" onClick={onResume} title="Resume indexing">
            ▶ Resume
          </button>
        )}
      </div>
      <div className="status-track">
        {phase === "scanning" ? (
          <span className="status-fill indeterminate" />
        ) : (
          <span
            className="status-fill"
            style={{
              width:
                phase === "indexing" || phase === "paused" ? `${pct}%` : "0%",
            }}
          />
        )}
      </div>
    </div>
  );
}
