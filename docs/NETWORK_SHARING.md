# Network sharing — design

**Status:** b4 (share server) **implemented** — see *b4 as built* at the end for
what shipped vs. what was deferred. b5 (discovery + import) not started.

Readaity gains the ability to serve its libraries over the local network so that:

1. any device with a browser can browse a Readaity library and download books, and
2. another Readaity instance can discover peers on the LAN, browse them, and
   import selected books into its own library.

Everything stays on the LAN, over TLS 1.3, behind a PIN. Nothing is exposed to
the internet, and no Readaity instance ever reads a file that isn't a catalogued
book.

---

## Part 1 — the share server ("Readaity Share")

### Runtime

- An embedded **HTTPS** server on the Rust side using **`axum`** served over
  **`axum-server`** with `rustls` (Tauri 2 already runs a tokio runtime, so this
  adds little weight), plus `tower-http` for byte-range file streaming. Plain
  HTTP is never served — see TLS below.
- Lifecycle: a `ShareServer` handle in Tauri managed state. Commands
  `share_start`, `share_stop`, `share_status`, `share_regenerate_cert`. The
  server is **off by default** and never auto-starts.
- Config, persisted in the existing `settings` table:
  | key | default | meaning |
  |---|---|---|
  | `share_enabled` | `false` | start the server on launch |
  | `share_port` | `8787` | listen port (HTTPS) |
  | `share_name` | hostname | how this instance shows up to peers |
  | `share_pin` | random 6-digit | access code, 6–10 digits (see Security) |
  | `share_cert_pem` / `share_key_pem` | generated on first start | self-signed TLS material |
  | `share_allowlist` | empty | optional comma-separated client IP/CIDR allowlist |
  | `share_audit` | `true` | log connections + downloads |
- Bind address is always `0.0.0.0` on the chosen port (LAN reachable). There is
  no loopback-only mode — the feature is pointless without LAN exposure, and the
  PIN + TLS are the gate.
- Settings UI (new "Sharing" tab): the toggle, port field, display name, the
  PIN field (numeric, 6–10 digits, with a "generate random" button and a
  show/hide), the **certificate fingerprint** (SHA-256, for peer verification)
  with "regenerate", an optional client-IP allowlist box, a **"Trust this
  device"** panel (`/trust` URL + QR + per-platform steps — see Browsing from a
  web browser), and — when running — the reachable URLs
  (`https://<each-lan-ip>:<port>`) plus a QR of the primary one and a live
  connection/audit list.

### HTTP surface

All `/api/*` responses are JSON unless noted. Every route except `/healthz`
requires auth (see Security).

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Self-contained browse UI (single HTML file, `include_str!`-embedded, inline CSS/JS, no external assets) |
| GET | `/healthz` | `{ "app": "readaity", "version": "x.y.z", "fingerprint": "<sha256>" }` — unauthenticated, used for discovery probing and cert pinning |
| GET | `/trust` | The server's certificate as `application/x-pem-file` (`readaity-<name>.pem`), for "trust this device". Unauthenticated — it's a public cert, not the key |
| GET | `/trust/help` | A short static page with per-platform install steps (rendered from the same explainer the Sharing tab uses) |
| POST | `/api/auth` | Body `{ pin }` → sets an auth cookie on success, 401 on failure (rate-limited + lockout) |
| GET | `/api/manifest` | `{ name, version, libraries: { comics, ebooks }, generated_at }` — cheap summary for peers |
| GET | `/api/books?library=comics\|ebooks` | Array of `{ id, title, format, size, page_count, md5, has_cover }`. `id` is an opaque token (see below), **never a filesystem path** |
| GET | `/api/cover/:id` | `image/jpeg` bytes from the `cover` BLOB, or 404 |
| GET | `/api/download/:id` | The book file, `Content-Disposition: attachment; filename="<title>.<ext>"`, supports `Range` |

### The `id` token

The browse UI and peers must never see or send real paths.

- `id = base64url(hmac_sha256(server_session_key, book.path))` truncated to 16
  bytes, mapped back to the path via an in-memory `HashMap<Id, PathBuf>` rebuilt
  from `db::list_books` when the server starts and after any library change.
- `/api/download/:id` looks the id up in that map, re-checks the row is still
  `status = 'ready'`, then streams. An id that isn't in the map → 404. This makes
  path traversal structurally impossible — there is no path input.
- `server_session_key` is random per server start, so ids don't survive a
  restart (fine — peers re-fetch `/api/books`).

### Security

#### TLS (self-signed)

- On first `share_start` with no stored cert, generate a self-signed certificate
  with **`rcgen`**: EC P-256 key, CN `readaity`, SANs for `readaity.local`, the
  detected LAN IPs and `localhost`, ~2-year validity. Store cert + key PEM in the
  `settings` table (`share_cert_pem` / `share_key_pem`); reuse on later starts.
- The server **only** speaks HTTPS (`axum-server` + `rustls`). No HTTP listener,
  no HTTP→HTTPS redirect port.
- **TLS 1.3 only.** `ServerConfig` is built with
  `.with_protocol_versions(&[&rustls::version::TLS13])` — TLS 1.2 and below are
  refused. Cipher suites are restricted to the AEAD trio, in this preference
  order: `TLS13_AES_256_GCM_SHA384`, `TLS13_CHACHA20_POLY1305_SHA256`,
  `TLS13_AES_128_GCM_SHA256`. Key exchange is limited to `X25519` (with
  `secp384r1` as a fallback). No renegotiation, no compression, no session
  tickets across restarts (the session key rotates anyway).
- The client half (b5) builds its `rustls::ClientConfig` with the same
  TLS-1.3-only restriction, so a downgraded peer is rejected before the
  fingerprint check.
- Browsers will show a "not trusted" warning (expected for self-signed). Two
  paths past it — a one-time click-through, or installing the cert once so the
  warning is gone for good. See **Browsing from a web browser** below.
- **No HSTS.** The server never sends `Strict-Transport-Security`. A self-signed
  setup where the IP, port or cert can change must always allow a fresh
  exception — HSTS would wedge the browser with no way through.
- **Readaity-to-Readaity trust is fingerprint pinning, not the CA chain.** The
  discovering peer reads the fingerprint from `/healthz` (or mDNS TXT), shows it
  to the user on first connect ("Trust <name> — fingerprint AB:CD:…?"), and pins
  it. A later fingerprint change forces re-confirmation (detects MITM / a
  regenerated cert).
- `share_regenerate_cert` throws the old cert away and makes a new one (used if a
  key is thought compromised); all peers must re-trust.

#### PIN

- 6–10 digits, chosen by the user or generated (default: random 6). Stored as an
  **Argon2id hash** in `settings`, never in plaintext; the raw PIN is only held
  in memory long enough to display once after generation.
- `/api/auth` compares with a constant-time check against the hash.
- First request without a valid `readaity_share` cookie → the browse UI shows a
  PIN prompt; peers call `/api/auth` first. The cookie is `HttpOnly`, `Secure`,
  `SameSite=Strict`, bound to the client IP, signed with the session key,
  ~12 h expiry.

#### Brute-force / abuse

- **Lockout:** `/api/auth` failures counted per client IP — **5 failures → that
  IP locked out 15 minutes** (window resets on success). Lockouts surface in the
  Settings connection list.
- **Global auth throttle:** at most N `/api/auth` attempts/second across all IPs,
  so a botnet-style spread can't parallelise around the per-IP lockout.
- **Rate limiting on every route** (not just auth) via `tower_governor` — a
  sane per-IP request/second cap with burst.
- **Connection caps:** max concurrent connections and max concurrent downloads;
  a per-download and aggregate bandwidth ceiling is configurable.

#### Network boundary

- **Private-range only:** refuse (403) any request whose client IP is not in
  `10/8`, `172.16/12`, `192.168/16`, `169.254/16` or loopback. Blocks accidental
  exposure if the machine is on a public IP or the port is forwarded.
- **Optional allowlist:** if `share_allowlist` is set, only those IPs/CIDRs may
  connect at all (checked before auth).
- **mDNS hygiene:** the TXT record carries only `v=` and `pin=required` — no
  library names, counts, or user identity.
- Advertising stops the instant the server stops.

#### Data boundary

- **Read-only.** No route writes anything. A peer importing books pulls from
  *this* server; this server never pushes and never deletes.
- **Catalogue-bounded.** Only `ready` books currently in `books` are reachable,
  and only via their opaque HMAC id (see above) — there is no path, filename or
  glob input anywhere in the API. Covers likewise.
- **No CORS.** `Access-Control-Allow-Origin` is never sent; the browse UI is
  same-origin. Peers are not browsers and don't need it.
- Security headers on every response: `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer`, a restrictive `Content-Security-Policy` for the
  browse UI, `Cache-Control: no-store` on API responses. **No
  `Strict-Transport-Security`** (see TLS — HSTS would trap the browser on a
  self-signed setup).

#### Operational

- **Visible when on.** Tray/title reflects "Sharing on"; the Settings tab shows a
  live list of connected clients and recent downloads.
- **Audit log** (`share_audit`, on by default): append-only log of
  connect / auth-fail / lockout / download events (time, IP, book title) kept in
  a `share_audit` table, viewable and clearable from Settings.
- **Stops on network change / sleep:** if the active network interface or its
  subnet changes, or the machine resumes from sleep, the server stops and must be
  re-armed (prevents "followed me onto a coffee-shop wifi" exposure).
- **Session ends cleanly:** `share_stop` drops all cookies (rotates the session
  key), closes listeners, and withdraws the mDNS advert.

#### Still open / later

- Trust-on-first-use is as good as the user checking the fingerprint once; a
  future option could let two instances pair via a short code that also exchanges
  pinned fingerprints, removing the manual step.
- No at-rest encryption of the cert/PIN-hash beyond the OS user profile — the
  `settings` DB is already only readable by the user account.
- Per-book / per-folder share scoping (share only some libraries or folders) is
  not in b4; the whole catalogue of a library is shared or nothing.

### Browse UI (`/`)

One HTML file, same dark theme as the app. Library switcher, cover grid,
search-by-title, click a book → download. Fully keyboard accessible. Degrades to
a plain list if JS is disabled. No build step — it's hand-written and embedded.

### Browsing from a web browser

Any device with a modern browser can use `/` — but the self-signed cert means
the first visit hits the browser's "not private" interstitial. Two ways through:

**A. Click through (default, zero setup).**
The Sharing tab and the PIN-prompt page both spell out the exact steps per
browser ("click *Advanced*, then *Proceed to …*"). It's a one-time action per
device — the browser remembers the exception. Fine for a quick "grab that book
onto my tablet".

**B. Trust this device (removes the warning for good).**
- The Sharing tab shows a **"Trust this device"** panel: the `/trust` URL, a QR
  of it, the SHA-256 fingerprint to verify against, and collapsible per-platform
  steps. `/trust/help` serves the same steps as a page reachable from the phone
  itself.
- Steps, in brief:
  - **iOS/iPadOS** — open `/trust` in Safari → install the profile in Settings →
    **also** enable it under *General → About → Certificate Trust Settings*
    (two separate screens; the second is the one people miss).
  - **Android** — download from `/trust` → *Settings → Security → Encryption &
    credentials → Install a certificate → CA certificate*. Chrome/Firefox on
    Android honour user-added CAs for browsing.
  - **Windows** — double-click the `.pem` → *Install Certificate → Local Machine
    → Trusted Root Certification Authorities*.
  - **macOS** — open in Keychain Access → *System* keychain → set to *Always
    Trust*.
  - **Firefox (any OS)** — *Settings → Certificates → View Certificates →
    Import*, tick "trust for websites" (Firefox has its own store).
- After trusting, `https://<ip>:<port>` loads clean. If the cert is regenerated
  (`share_regenerate_cert`), the old trust entry must be removed and `/trust`
  re-run — the fingerprint on the Sharing tab will have changed.

**Browser compatibility.** TLS 1.3 is required, which every browser has shipped
since ~2020 (Chrome 70, Firefox 63, Safari 14, Edge 79). Older embedded browsers
— some e-readers, older smart TVs, kiosks — cap at TLS 1.2 and **cannot connect
at all**, click-through or not. Documented as a known limitation; the target
devices are current phones/laptops.

**Future — remove the warning entirely.** The Plex / Home Assistant approach: a
publicly-trusted wildcard cert plus a DNS service that resolves
hashed-private-IP hostnames (e.g. `a1b2c3.readaity.direct → 192.168.1.42`). No
interstitial anywhere, no per-device trust. Needs a domain, a wildcard cert and
a tiny DNS service Readaity doesn't run yet — out of scope for b4, noted as the
eventual clean answer.

---

## Part 2 — discovery and import

### Advertising

- When the share server is running, advertise via mDNS / DNS-SD using
  **`mdns-sd`** (pure Rust):
  - service type `_readaity._tcp.local.`
  - instance name = `share_name`
  - port = `share_port`
  - TXT records: `v=<app version>`, `pin=required`
- Stop advertising when the server stops.

### Discovering (the client half)

- New **"Network"** entry in the sidebar, below the library switcher.
- Browsing `_readaity._tcp` lists discovered peers: name, host, `GET /healthz`
  reachability, and (after auth) library counts from `/api/manifest`.
- On first connect to a peer, show its TLS fingerprint (from `/healthz`) and ask
  the user to trust it; the pin is stored per peer in `settings`
  (`peer_trust_<host>`). A changed fingerprint later blocks the connection until
  re-confirmed.
- Then prompts for its PIN once (kept for the session only, in memory), and shows
  its shelf using the same `/api/books` + `/api/cover/:id` the browser UI uses.
- The peer's self-signed cert is verified against the pinned fingerprint only
  (custom `rustls` `ServerCertVerifier`) — the system trust store is not
  consulted.

### Importing

- Multi-select books on a peer → **Import**.
- Destination: a picker limited to the current library's existing folders (or
  "Add a folder…" first). Readaity does not invent locations.
- For each selected book:
  1. Skip if its `md5` matches a local book already in that library
     (dedupe against `db::list_books` hashes) — surfaced as "3 already in your
     library, will skip".
  2. `GET /api/download/:id` → write to `<dest>/<sanitised title>.<ext>`,
     renaming on collision (`title (2).epub`).
  3. After all downloads, `rescan(library)` so the new files are catalogued and
     swept normally (covers regenerate locally — we don't trust a peer's cover
     blob as canonical, though we could show it during browsing).
- Progress reported through the existing `scan-status` / a new `import-status`
  event.
- Import is **copy**, not move — the peer keeps its copy.

### Not in scope (yet)

- Auth beyond a shared PIN (per-user accounts, proper CA-issued certs).
- Syncing reading progress between instances.
- Writing to a peer / remote deletion.
- WAN / relay / anything off the local segment.

---

## Phasing

| Phase | Ship | Contents |
|---|---|---|
| b4 | Share server | HTTPS `axum` server (self-signed cert via `rcgen`, `rustls`, **TLS 1.3 only**, AEAD suites, no HSTS), `settings` config, opaque-id map, Argon2 PIN (6–10 digits) with per-IP lockout + global throttle + route rate-limiting, private-range guard + optional allowlist, connection/bandwidth caps, audit log, security headers, embedded browse UI, `/trust` cert download + `/trust/help`, Settings "Sharing" tab (PIN, fingerprint, "Trust this device", allowlist, live connections) |
| b5 | Discovery + import | `mdns-sd` advertise + browse, "Network" sidebar view, TLS fingerprint trust-on-first-use + pinning, peer PIN prompt, multi-select import with md5 dedupe and `rescan` |

New crates (b4): `axum`, `axum-server` (rustls), `tokio-rustls` / `rustls`,
`rcgen`, `tower`, `tower-http` (`fs`, `set-header`), `tower_governor`,
`argon2`, `hmac`, `sha2`, `time` or `cookie`. (b5): `mdns-sd` — plus a custom
`rustls` `ServerCertVerifier` for fingerprint pinning, no new crate.

---

## b4 as built

Module: `src-tauri/src/share/` (`mod`, `cert`, `tls`, `guard`, `auth`, `ids`,
`routes`, `assets/`). Commands: `share_get_config`, `share_set_config`,
`share_set_pin`, `share_generate_pin`, `share_start`, `share_stop`,
`share_status`, `share_regenerate_cert`, `share_audit_log`, `share_clear_audit`.
UI: `src/components/SharingSettings.tsx`, a "Network sharing" tab in Settings.

Crates actually used: `aws-lc-rs` (feature `prebuilt-nasm` — the build box has no
nasm/cmake), `rustls` + `axum-server` (`tls-rustls-no-provider`), `rcgen`,
`axum` 0.8, `tower-http` (`set-header`), `tokio`/`tokio-util`, `rustls-pemfile`,
`argon2`, `hmac`, `sha2`, `hex`, `rand`, `local-ip-address`.

**Shipped as designed:** HTTPS-only, **TLS 1.3 only** with the three AEAD suites
(strongest first) and no HSTS; self-signed cert via `rcgen` (CN `Readaity`, SANs
for `localhost` / `readaity.local` / detected LAN IPs), persisted in `settings`,
SHA-256 fingerprint on `/healthz` and in the UI, `share_regenerate_cert`.
Argon2id PIN 6–10 digits, constant-time verify, `HttpOnly`/`Secure`/`SameSite`
IP-bound signed cookie (12 h). Per-IP lockout (5 fails → 15 min), global
`/api/auth` throttle, per-IP request rate limit. Private-range guard + optional
allowlist. Opaque per-request HMAC book ids (recomputed each call — no id map to
invalidate, no path in the API). Read-only, no CORS, `nosniff` / `no-referrer` /
`no-store` / CSP-on-HTML. Audit log in a `share_audit` table (viewable/clearable
from Settings). Endpoints `/`, `/healthz`, `/trust`, `/trust/help`, `/api/auth`,
`/api/manifest`, `/api/books`, `/api/cover/{id}`, `/api/download/{id}`. Embedded
single-file browse UI with its own PIN gate. Autostart on launch when the user
left it enabled.

**Verified** (2026-08-30): 9 Rust unit tests (`share::auth` / `guard` / `ids`)
plus a 28-check live suite against the running server — TLS 1.3 negotiated,
TLS 1.2 refused, no HSTS, security headers, PIN auth + IP-bound cookie,
per-IP lockout after 5 fails, opaque ids, full books/manifest/download cycle,
`/`, `/trust`, `/trust/help`.

Two things learned while testing:
- The signed cookie first used `.` as its field separator, which broke on
  IPv4 client addresses (they contain dots) — now `|`.
- **Windows 10's Schannel has no client-side TLS 1.3**, so the system `curl`
  and PowerShell `Invoke-WebRequest` cannot reach the server on Win10. Real
  browsers (Chrome/Firefox/Edge/Safari — own TLS stacks) are fine; this only
  bites native Win10 HTTP clients, which is acceptable for the target use.

**Deferred to a b4.x / b5:**
- **QR codes** — the Sharing tab lists the `https://…` URLs as text and points at
  `…/trust/help`; no QR image yet.
- **Range / byte-serving** on `/api/download` — currently a plain streamed 200
  (fine for "Save As"; resumable downloads need `Range`).
- **Bandwidth ceilings and max-concurrent-connection/download caps** — only the
  per-IP rate limit + auth throttle are in.
- **Stop-on-network-change / resume-from-sleep.**
- **Live "connected clients" panel** — the audit log covers recent activity
  instead.
- **Tray/title "Sharing on" indicator** — status shows in the Settings tab only.
- **`tower_governor`** — replaced by a small in-house per-IP limiter in `guard`.
