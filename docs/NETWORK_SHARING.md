# Network sharing — design

**Status:** design only, not built. Target: b4 (server) then b5 (discovery + import).

Readaity gains the ability to serve its libraries over the local network so that:

1. any device with a browser can browse a Readaity library and download books, and
2. another Readaity instance can discover peers on the LAN, browse them, and
   import selected books into its own library.

Everything stays on the LAN. Nothing is exposed to the internet, and no Readaity
instance ever reads a file that isn't a catalogued book.

---

## Part 1 — the share server ("Readaity Share")

### Runtime

- An embedded HTTP server on the Rust side using **`axum`** (Tauri 2 already runs
  a tokio runtime, so this adds little weight) plus `tower-http` for byte-range
  file streaming.
- Lifecycle: a `ShareServer` handle in Tauri managed state. Commands
  `share_start`, `share_stop`, `share_status`. The server is **off by default**
  and never auto-starts.
- Config, persisted in the existing `settings` table:
  | key | default | meaning |
  |---|---|---|
  | `share_enabled` | `false` | start the server on launch |
  | `share_port` | `8787` | listen port |
  | `share_name` | hostname | how this instance shows up to peers |
  | `share_pin` | random 6-digit | access code (see Security) |
- Bind address is always `0.0.0.0` on the chosen port (LAN reachable). There is
  no loopback-only mode — the feature is pointless without LAN exposure, and the
  PIN is the gate.
- Settings UI (new "Sharing" tab): the toggle, port field, display name, the
  current PIN with a "regenerate" button, and — when running — the reachable
  URLs (`http://<each-lan-ip>:<port>`) plus a small QR of the primary one.

### HTTP surface

All `/api/*` responses are JSON unless noted. Every route except `/healthz`
requires auth (see Security).

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Self-contained browse UI (single HTML file, `include_str!`-embedded, inline CSS/JS, no external assets) |
| GET | `/healthz` | `{ "app": "readaity", "version": "x.y.z" }` — unauthenticated, used for discovery probing |
| POST | `/api/auth` | Body `{ pin }` → sets an auth cookie on success, 401 on failure (rate-limited) |
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

- **PIN gate.** First request without a valid `readaity_share` cookie → the
  browse UI shows a PIN prompt; peers call `/api/auth` first. The cookie is
  `HttpOnly`, `SameSite=Strict`, signed with the session key, ~12 h expiry.
- **Brute-force protection.** `/api/auth` failures are counted per client IP;
  after 5 failures in 5 minutes that IP is locked out for 15 minutes. PIN is
  6 digits, so this keeps online guessing infeasible.
- **Read-only.** No route writes anything. A peer importing books pulls from
  *this* server; this server never pushes.
- **Catalogue-bounded.** Only `ready` books that are currently in `books` are
  reachable, and only via their opaque id. Covers likewise.
- **No internet.** We advertise and bind on the LAN only and document that users
  should not port-forward it. (Optional later: refuse to serve if the client IP
  isn't in a private range.)
- **Visible when on.** The tray/title reflects "Sharing on" so it can't run
  forgotten.

### Browse UI (`/`)

One HTML file, same dark theme as the app. Library switcher, cover grid,
search-by-title, click a book → download. Fully keyboard accessible. Degrades to
a plain list if JS is disabled. No build step — it's hand-written and embedded.

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
- Selecting a peer prompts for its PIN once (stored for the session only, in
  memory), then shows its shelf using the same `/api/books` + `/api/cover/:id`
  the browser UI uses.

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

- Auth beyond a shared PIN (per-user accounts, TLS).
- Syncing reading progress between instances.
- Writing to a peer / remote deletion.
- WAN / relay / anything off the local segment.

---

## Phasing

| Phase | Ship | Contents |
|---|---|---|
| b4 | Share server | `axum` server, `settings` config, opaque-id map, PIN auth + lockout, `/api/*`, embedded browse UI, Settings "Sharing" tab |
| b5 | Discovery + import | `mdns-sd` advertise + browse, "Network" sidebar view, peer PIN prompt, multi-select import with md5 dedupe and `rescan` |

New crates: `axum`, `tower`, `tower-http` (features: `fs`, `set-header`),
`mdns-sd`, `hmac`, `sha2` (b4); no new crates for b5 beyond `mdns-sd`.
