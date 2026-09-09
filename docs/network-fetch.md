# Supervised network fetch and repo radar

Status: **IMPLEMENTED** for read-only HTTPS GET to allowlisted hosts plus a
GitHub trending digest. Authenticated fetching, POST/upload, and general
browsing remain **PLANNED**; each would need its own threat review.

## What changed

- New `NetworkFetch` capability (disabled by default) and `FetchUrl`
  action. `allow_network_domain` grants one exact hostname
  (case-insensitive, all ports; subdomains not implied). Loopback hosts
  need the same explicit grant, so tests stay honest.
- Strict URL parsing before any grant is consulted: `https` only, except
  `http` for loopback (`localhost`, `127.0.0.1`, `[::1]`). Userinfo,
  non-numeric ports, malformed hosts, overlong URLs, and every other
  scheme (`ftp`, `file`, `data`, …) are rejected.
- `FetchTool` (provider crate, opt-in via `ToolRegistry::register`) does
  GET-only with a fixed User-Agent, no credentials, no custom headers,
  30 s timeout, 256 KiB streamed cap, and strict UTF-8. Denials happen
  before any byte moves; timeouts, overflows, non-UTF-8, and error statuses
  fail closed.
- Delegation parity: narrowed children fetch nothing until re-granted via
  `grant_network_from`, which verifies the parent holds the exact host.
- Repo radar: `github_search_url` (GitHub query encoding, per-page clamped),
  `default_query` (created in the last 30 days, dependency-free date math),
  `fetch_text` through the permission-checked registry, and `render_digest`
  (strict shape, labeled untrusted). CLI: `--radar [--radar-query Q]
  [--radar-limit N]`, granted `api.github.com` only.
- Threat posture: fetched bytes are **untrusted data**. They travel only in
  observation payloads, no success criterion can be satisfied by them, and
  the digest header says to verify before acting. Remote content is the
  classic prompt-injection channel; models must treat it as data, and any
  future model integration must preserve that separation.

## Verified

- Core URL/gating matrix (in `permissions.rs` tests): canonical forms,
  normalization, 15 hostile shapes, default-deny, exact matching,
  subdomain/lookalike rejection, loopback grant requirement, delegation
  re-grant parity.
- `harness-provider/tests/fetch.rs` (6, loopback mock server): body +
  fixed User-Agent, denial before any byte moves (no server running),
  default-deny, non-loopback http rejection, 300 KiB overflow, non-UTF-8,
  500s, 1 s timeout failing fast, digest helpers, end-to-end mock radar.
- Live check (not a test): `--radar --radar-limit 5` returned the actual
  hottest month-old repos with stars/languages/descriptions; the recency
  filter was independently confirmed against the API (`created_at` dates
  in range). Live data is non-deterministic and claimed as a demo only.
- Full suite: 125 passing. fmt/check/test/clippy clean.

## Limits (honest)

- Unauthenticated GitHub API: 60 requests/hour per IP; no caching or
  conditional requests yet. The digest prints whatever the API returns.
- No POST, no auth, no cookies, no redirects special-casing (reqwest
  follows same-host redirects by default policy; cross-host redirect
  targets are NOT re-checked against the allowlist — avoid sensitive
  contexts until redirect pinning lands).
- Search relevance is GitHub's, not ours; "latest" means created-range +
  stars sort, which favors already-popular newcomers.
- Redirects are disabled client-wide: a 3xx is a failed observation, so the
  allowlist decision always covers the bytes received. Treat fetched
  content as untrusted regardless.
