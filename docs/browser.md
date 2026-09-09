# Structured browser control

Status: **IMPLEMENTED** for Chrome over CDP with isolated profiles,
named-profile launch, and attach-to-live-instance. Full browser
automation (downloads, file uploads, auth flows, multi-tab orchestration)
and non-Chrome browsers remain **PLANNED**.

## What changed

- New `harness-browser` crate (serde/serde_json only, zero new
  dependencies). The WebSocket layer is hand-rolled over `TcpStream`
  (masked sends, fragmented receives, ping/pong, close, size bounds);
  Chrome 150+ speaks and is tested against it, including multi-hundred-KB
  screenshot frames. CDP HTTP uses 1.1 (Chrome refuses 1.0) with
  Content-Length framing.
- `ControlledBrowser` operations: navigate (waits for load), title, text,
  raw eval, click/fill by CSS selector (missing selectors fail, never act
  blindly), bounded PNG screenshots, tab listing, and deterministic close.
- Three session kinds with a hard ownership rule:
  - Isolated (default): headless Chrome in a temp profile, destroyed on
    close. Nothing personal is reachable.
  - Personal launch (`launch_with_profile` by display name or directory,
    e.g. `"MrM"` or `"Default"`): browses with the user's full identity.
    Fails closed while another Chrome holds the profile.
  - Attach (`attach` / `list_tabs`): drives an instance the user started
    with `--remote-debugging-port` — that flag is the explicit consent.
    Attached sessions only disconnect on close; they can never terminate
    the user's browser (structurally separate handle, plus a test proving
    the owner keeps working after detach).
- Profile discovery parses Chrome's `Local State` read-only
  (`CHROME_USER_DATA_DIR` / `CHROME_BINARY` overrides for tests).
- CLI one-shots: `--browse-profiles`, `--browse-tabs`, `--browse-read
  <filter>` with `--browse-endpoint` (default `http://127.0.0.1:9222`).

## Verified

- `harness-browser/tests/controlled.rs` (8) against real headless Chrome
  with a loopback fixture server: navigation/reads, click/fill with
  missing-selector refusal, PNG screenshot bounds, cross-profile storage
  isolation, attach-without-ownership, profile cleanup, Local State
  fixture parsing.
- Live checks (demos, not tests): `--browse-profiles` resolved the real
  `MrM`/`Hiếu`/`CNX` profiles; `--browse-tabs`/`--browse-read` drove an
  attach probe end to end.
- Full suite: 138 passing. fmt/check/test/clippy clean.

## Limits (honest)

- Chrome only, by user choice; no Edge fallback, no Firefox/WebDriver.
- Personal browsing acts with ambient authority: sessions, logins, and
  purchases are one confused click away. Treat every page as untrusted
  data; never auto-fill credentials or confirm transactions.
- No downloads/uploads handling, no auth-flow helpers, no tab orchestration
  beyond select-by-substring, no vision fallback.
- To browse as yourself: close Chrome and launch the profile through the
  API, or start Chrome once with `--remote-debugging-port=9222` and use
  attach. The harness will not restart your browser for you.
