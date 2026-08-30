import { useCallback, useEffect, useRef, useState } from "react";
import type { AuditRow, ShareConfig, ShareStatus } from "../types";
import {
  shareAuditLog,
  shareClearAudit,
  shareGeneratePin,
  shareGetConfig,
  shareRegenerateCert,
  shareSetConfig,
  shareSetPin,
  shareStart,
  shareStatus,
  shareStop,
} from "../lib/api";

const PLATFORMS: { name: string; steps: string[] }[] = [
  {
    name: "iPhone / iPad",
    steps: [
      "Open the trust URL in Safari and allow the profile download.",
      "Settings → General → VPN & Device Management → install the profile.",
      "Settings → General → About → Certificate Trust Settings → turn it on.",
    ],
  },
  {
    name: "Android",
    steps: [
      "Download the certificate from the trust URL.",
      "Settings → Security → Encryption & credentials → Install a certificate → CA certificate.",
      "Pick the downloaded .pem file.",
    ],
  },
  {
    name: "Windows",
    steps: [
      "Save the file, then double-click it.",
      "Install Certificate → Local Machine → Trusted Root Certification Authorities.",
    ],
  },
  {
    name: "macOS",
    steps: [
      "Open the file in Keychain Access, adding it to the System keychain.",
      "Double-click it → Trust → When using this certificate → Always Trust.",
    ],
  },
  {
    name: "Firefox (any system)",
    steps: [
      "Settings → Privacy & Security → Certificates → View Certificates → Import.",
      "Select the file and tick “Trust this CA to identify websites”.",
    ],
  },
];

function fmtTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

export function SharingSettings() {
  const [cfg, setCfg] = useState<ShareConfig | null>(null);
  const [status, setStatus] = useState<ShareStatus | null>(null);
  const [audit, setAudit] = useState<AuditRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // editable fields
  const [port, setPort] = useState("8787");
  const [name, setName] = useState("");
  const [allowlist, setAllowlist] = useState("");
  const [auditOn, setAuditOn] = useState(true);
  const [pin, setPin] = useState("");
  const [showTrust, setShowTrust] = useState(false);

  const poll = useRef<number | null>(null);

  const loadAll = useCallback(async () => {
    const [c, s, a] = await Promise.all([
      shareGetConfig(),
      shareStatus(),
      shareAuditLog(100),
    ]);
    setCfg(c);
    setStatus(s);
    setAudit(a);
    setPort(String(c.port));
    setName(c.name);
    setAllowlist(c.allowlist);
    setAuditOn(c.audit);
  }, []);

  useEffect(() => {
    loadAll().catch((e) => setErr(String(e)));
  }, [loadAll]);

  // While the server runs, refresh status + audit every few seconds.
  useEffect(() => {
    if (!status?.running) {
      if (poll.current) window.clearInterval(poll.current);
      poll.current = null;
      return;
    }
    poll.current = window.setInterval(() => {
      Promise.all([shareStatus(), shareAuditLog(100)])
        .then(([s, a]) => {
          setStatus(s);
          setAudit(a);
        })
        .catch(() => {});
    }, 4000);
    return () => {
      if (poll.current) window.clearInterval(poll.current);
    };
  }, [status?.running]);

  const run = useCallback(
    async (fn: () => Promise<void>, ok?: string) => {
      setBusy(true);
      setErr(null);
      setNotice(null);
      try {
        await fn();
        if (ok) setNotice(ok);
      } catch (e) {
        setErr(String(e));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const saveConfig = () =>
    run(async () => {
      const c = await shareSetConfig(
        Number(port) || 8787,
        name.trim(),
        allowlist.trim(),
        auditOn,
      );
      setCfg(c);
    }, "Saved.");

  const savePin = () =>
    run(async () => {
      await shareSetPin(pin.trim());
      setPin("");
      await loadAll();
    }, "PIN set.");

  const genPin = () =>
    run(async () => {
      const p = await shareGeneratePin();
      await loadAll();
      setNotice(`New PIN: ${p} — write it down, it isn't shown again.`);
    });

  const start = () =>
    run(async () => setStatus(await shareStart()));

  const stop = () =>
    run(async () => {
      await shareStop();
      setStatus(await shareStatus());
    });

  const regen = () =>
    run(async () => {
      const fp = await shareRegenerateCert();
      await loadAll();
      setNotice(`New certificate — fingerprint ${fp}. Re-trust on every device.`);
    });

  const clearAudit = () =>
    run(async () => {
      await shareClearAudit();
      setAudit([]);
    });

  if (!cfg || !status) {
    return <p className="settings-empty">Loading…</p>;
  }

  const primaryUrl = status.urls[0];

  return (
    <div className="sharing">
      <p className="settings-hint">
        Serve your libraries to other devices on this network over HTTPS, behind a
        PIN. Nothing leaves this machine except book files you choose to download.
      </p>

      {err && <p className="sharing-err">{err}</p>}
      {notice && <p className="sharing-notice">{notice}</p>}

      {/* ---- server ---- */}
      <div className="sharing-block">
        <div className="sharing-row">
          <div>
            <strong>{status.running ? "Sharing is on" : "Sharing is off"}</strong>
            {status.running && primaryUrl && (
              <div className="settings-hint">Reachable at {primaryUrl}</div>
            )}
          </div>
          {status.running ? (
            <button className="btn" onClick={stop} disabled={busy}>
              Stop
            </button>
          ) : (
            <button
              className="btn primary"
              onClick={start}
              disabled={busy || !status.pin_set}
              title={status.pin_set ? "" : "Set a PIN first"}
            >
              Start
            </button>
          )}
        </div>
        {status.running && status.urls.length > 1 && (
          <ul className="sharing-urls">
            {status.urls.map((u) => (
              <li key={u}>{u}</li>
            ))}
          </ul>
        )}
      </div>

      {/* ---- PIN ---- */}
      <div className="sharing-block">
        <span className="settings-field-label">Access PIN</span>
        <p className="settings-hint">
          {status.pin_set
            ? "A PIN is set. Enter a new one to change it."
            : "Set a PIN (6–10 digits) before you can start sharing."}
        </p>
        <div className="sharing-row">
          <input
            className="sharing-input"
            type="text"
            inputMode="numeric"
            placeholder="6–10 digits"
            maxLength={10}
            value={pin}
            onChange={(e) => setPin(e.target.value.replace(/\D/g, ""))}
          />
          <button className="btn" onClick={savePin} disabled={busy || pin.length < 6}>
            {status.pin_set ? "Change PIN" : "Set PIN"}
          </button>
          <button className="btn ghost" onClick={genPin} disabled={busy}>
            Generate
          </button>
        </div>
      </div>

      {/* ---- settings ---- */}
      <div className="sharing-block">
        <div className="sharing-field">
          <span className="settings-field-label">Display name</span>
          <input
            className="sharing-input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className="sharing-field">
          <span className="settings-field-label">Port</span>
          <input
            className="sharing-input short"
            value={port}
            onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
          />
        </div>
        <div className="sharing-field col">
          <span className="settings-field-label">
            Allowed devices (optional, one IP or CIDR per line)
          </span>
          <textarea
            className="sharing-input"
            rows={2}
            placeholder="leave empty to allow any device on the LAN"
            value={allowlist}
            onChange={(e) => setAllowlist(e.target.value)}
          />
        </div>
        <label className="sharing-check">
          <input
            type="checkbox"
            checked={auditOn}
            onChange={(e) => setAuditOn(e.target.checked)}
          />
          Keep an activity log
        </label>
        <div className="sharing-row">
          <button className="btn" onClick={saveConfig} disabled={busy}>
            Save
          </button>
          <span className="settings-hint">
            Port and name changes apply next time you start sharing.
          </span>
        </div>
      </div>

      {/* ---- trust ---- */}
      <div className="sharing-block">
        <span className="settings-field-label">Trust this device</span>
        <p className="settings-hint">
          Browsers warn about the self-signed certificate. Click through the
          warning each visit, or install the certificate once to remove it.
          Verify this fingerprint on the other device:
        </p>
        <code className="sharing-fp">{status.fingerprint || "—"}</code>
        {status.running && primaryUrl && (
          <p className="settings-hint">
            On the other device open{" "}
            <strong>{primaryUrl}/trust/help</strong> for step-by-step instructions,
            or <strong>{primaryUrl}/trust</strong> to download the certificate.
          </p>
        )}
        <button
          className="btn ghost small"
          onClick={() => setShowTrust((v) => !v)}
        >
          {showTrust ? "Hide steps" : "Show steps"}
        </button>
        {showTrust && (
          <div className="trust-steps">
            {PLATFORMS.map((p) => (
              <details key={p.name}>
                <summary>{p.name}</summary>
                <ol>
                  {p.steps.map((s, i) => (
                    <li key={i}>{s}</li>
                  ))}
                </ol>
              </details>
            ))}
          </div>
        )}
        <div className="sharing-row">
          <button className="btn ghost small" onClick={regen} disabled={busy}>
            Regenerate certificate
          </button>
        </div>
      </div>

      {/* ---- audit ---- */}
      <div className="sharing-block">
        <div className="sharing-row">
          <span className="settings-field-label">Activity</span>
          {audit.length > 0 && (
            <button className="btn ghost small" onClick={clearAudit} disabled={busy}>
              Clear
            </button>
          )}
        </div>
        {audit.length === 0 ? (
          <p className="settings-hint">Nothing recorded yet.</p>
        ) : (
          <ul className="audit-list">
            {audit.map((a, i) => (
              <li key={i}>
                <span className="audit-time">{fmtTime(a.ts)}</span>
                <span className="audit-ip">{a.ip}</span>
                <span className={`audit-event ${a.event}`}>{a.event}</span>
                {a.detail && <span className="audit-detail">{a.detail}</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
