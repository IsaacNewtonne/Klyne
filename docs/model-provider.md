# First real model provider

Status: **IMPLEMENTED** as an OpenAI-compatible chat-completions adapter in the
new `harness-provider` crate. Model routing, retry policies beyond fixed
backoff, and persisted usage accounting remain **PLANNED**.

## What changed

- Provider-neutral boundary: adapters implement the existing sync `Model`
  trait, so a real model proposes typed actions through the same permissions,
  tool-call budgets, SQLite persistence, reconciliation, and independent
  verification as the deterministic adapter. A `Box<M: Model>` blanket impl
  lets callers select models dynamically.
- `OpenAiCompat` posts to any chat-completions endpoint (including local
  servers where compatible) with `response_format: json_object`, using
  reqwest blocking I/O with rustls + built-in webPKI roots. No tokio, no
  native TLS, no shell-outs.
- Strict structured decisions: the closed schema is act/verify/complete/fail
  with twelve file tools (`write_file`, `read_file`, `read_range`, `hash_file`,
  `patch_file`, `search_file`, `list_dir`, `stat_path`, `make_dir`,
  `copy_file`, `move_file`, `delete_path`). Shell execution is never proposed.
  Unknown tools, missing fields, wrong types, and non-JSON responses become `Fail`,
  never an executed action. Oversized text fields (>2 MiB) are rejected.
- Credentials and endpoint configuration stay out of prompts and persisted
  task data: `ProviderConfig` holds the endpoint URL and model name plus the
  *name* of an environment variable; the key value is read per call and sent
  only as an `Authorization` header.
- Bounds: per-attempt timeout (default 60 s), response-size cap (default
  256 KiB, enforced while streaming), bounded retries (default 2, only for
  transport errors and HTTP 429/500/502/503/504 with capped backoff), bounded
   history (default last 20 entries, observations truncated to 2000 bytes at a
   UTF-8 character boundary; see `truncate_to_char_boundary`).
  Everything fails closed to `Fail`.
- Client-side usage accounting (`http_calls`, `retries`, `prompt_tokens`,
  `completion_tokens`) is exposed via `usage()`; the runtime does not yet
  persist provider usage into events.
- CLI opt-in: `--openai-compat` reads `HARNESS_MODEL_ENDPOINT` (required),
  `HARNESS_MODEL_NAME` (required), and `HARNESS_MODEL_API_KEY` (optional, for
  local servers without auth). Secrets travel via environment only, never argv.

## Verified

`crates/harness-provider/tests/provider.rs` uses a std-only loopback mock
HTTP server (no extra dependencies): 10 tests pass on Windows/MSVC.

- Full repair loop through `AgentRuntime`: mock decisions
  search → range → hash → digest-guarded patch → complete verify a controlled
  `41` → `42` fix with `FileRange` evidence; 5 model calls observed.
- Full file-creation run through the CLI-shaped path completes with
  independent verification.
- Strictness: unknown tools, missing fields, wrong decision kinds, and
  non-JSON bodies fail without any `ToolCalled` event or file side effect.
- Retries: 500-then-200 succeeds with `retries == 1`; persistent 500s fail
  after exactly `max_retries + 1` calls; 401 never retries; 10 s server sleep
  with a 1 s timeout fails fast; 300 KiB bodies are rejected at a 64 KiB cap.
- Secret hygiene: every request bore the header; no prompt body, event, or
  checkpoint snapshot contained the key.
- Live check beyond tests: the CLI drove a local Python mock endpoint to
  `COMPLETED` with independently verified bytes, and the key was absent from
  the event log and database afterwards.

Full suite after this change: 64 passing (54 prior + 10 provider).
`cargo fmt --check`, `cargo check --locked`, `cargo test --locked`, and
`cargo clippy -- -D warnings` pass. `file-core-v1` is unchanged (10/10,
10/10 denied).

## Limits (honest)

- One adapter, no router: no quality/cost/latency routing, no Anthropic or
  Gemini dialects, no vision/reasoning-effort knobs.
- Cancellation is timeout-only; there is no cross-thread cancel token (runtime
  owns lifecycle; planned separately).
- Usage is client-side memory only; token/cost budgets and persisted usage
  events are still planned, so provider runs are not yet cost-bounded.
- History packing is naive truncation; no relevance-ranked context selection.
- Real hosted endpoints were not exercised here; TLS roots are built in, but
  claims cover the mock-verified contract, not any vendor's behavior.
