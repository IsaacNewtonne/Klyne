# Local apps and capability improvement

Studio supports three routes: Windows Desktop for visible applications, Terminal
for documented command-line tools, and App APIs for HTTPS and numeric private-LAN/loopback HTTP APIs.
Enable the relevant switch for the next instruction. These are broad grants;
local API calls can change app data. No universal compatibility is claimed.

With Local APIs enabled, workers can create reusable connections automatically:

```json
{"decision":"act","action":{"tool":"app_connect","name":"my-app","base_url":"http://127.0.0.1:8000"}}
```

The Apps panel can save connections, discover operations, display saved operation
details, prepare an instruction with Local APIs enabled, and forget a connection.
Desktop apps remain available in its separate expandable picker.

`app_list` retrieves definitions shared across conversations in the Studio root.
`app_call` takes `name`, a `/path`, an optional `method` (GET by default), and an
optional JSON `body`. GET, POST, PUT, PATCH and DELETE are supported. Connection
creation saves an unverified definition; only a real call provides reachability
evidence. The model is instructed to inspect documented schemas and verify results.
Definitions survive restarts. The registry holds at most 64 named origins.

## Learning an API

`app_inspect {name,path?}` reads `/openapi.json` by default, or the supplied schema
path. It compiles OpenAPI 3.0/3.1 JSON into up to 256 saved operation definitions.
The 1 MiB schema response limit is separate from ordinary call limits. Schemas
are untrusted descriptions, not proof that operations work. Discovery performs
one GET; it does not scan ports or try endpoints automatically.

`app_operations {name,query?,offset?}` searches paths and summaries and returns
one saved operation per page, including parameters, a bounded body-schema summary,
and reasons an operation cannot yet be invoked. It makes no network requests and
is available to reviewers. Large body schemas must be inspected in the original
document; remote and cyclic references are refused.

```json
{"decision":"act","action":{"tool":"app_invoke","name":"my-app","operation":"GET /items/{id}","parameters":{"path":{"id":"item-1"}}}}
```

`app_invoke` uses the saved method and route. It validates required scalar path
and query parameters, their primitive types, unknown parameters, and required
JSON body presence. Parameter values are URL encoded. This is not full JSON Schema
validation. Configured bearer and API-key header authentication are supported. Other authentication, non-JSON request bodies, header/cookie parameters,
complex parameter serialization and alternate server URLs are marked unsupported.
An inspection never follows schema server URLs or remote references. Changing a
connection origin or authentication invalidates its old operations; forgetting it removes both.
Failed discovery preserves the last successfully saved definitions.

Adapter behavior follows the relevant subset of the
[OpenAPI specification](https://spec.openapis.org/oas/v3.0.3.html).

Calls disable proxies and redirects, accept HTTPS and numeric private-LAN/loopback HTTP origins,
and enforce 20-second requests, 32 KiB bodies and 64 KiB responses. API tools
reserve a step and persist pending work before executing. Results enter the chat
evidence ledger. Interrupted pending calls are never replayed by recovery.
An HTTP error or timeout may follow a side effect: inspect before retrying.
Reviewers may list definitions and saved operations but cannot invoke APIs,
including GET endpoints, or fetch a schema.
App data remains untrusted. Connections accept `auth.bearer_env` or
`auth.headers_env` mappings from header names to environment variable names.
Every named secret needs a user grant for the exact origin — declared in the
request `grants` or approved as a pending proposal — otherwise the call pauses
before anything transmits. DELETE calls additionally need an exact
(connection, method, path) approval. Credential values are resolved in the
server process, redacted from echoes, and scrubbed from prompts and stored
history for granted names.
Interactive OAuth, streaming, automatic port scans and launcher lifecycle integration
remain unimplemented. Stop cancels active HTTP requests; uncertain mutating outcomes
remain pending for inspection.

With Terminal enabled, workers may build and test reusable client files in their
workspace. They can also propose `self_improve` with `repo` (absolute Rust
repository; dirty trees are first snapshotted without altering the original), `id`, `changes: [{path,contents}]`, and `description`. Candidates
contain 1–16 files and at most 1 MiB of contents, within the model response limit.
The legacy single-file `path`/`contents` form remains accepted. Implementation
and new regression tests can be included together. The experiment runner writes
files in an isolated worktree, independently reads them back, and runs
`cargo test --offline` against baseline and candidate.
Both must have passing tests, zero failures, and no reduction in passed-test count.
Passing candidates remain on `klyne/<id>`; failures roll back. Reports live beside
the conversation database in `experiments/`. Experiments do not merge themselves.
The separate supervised `runtime_stage` tool can activate a built candidate with
digest/protocol checks and startup rollback. See the [upgrade guide](harness-upgrade-2026-09-11.md)
for persistent skills/tools, authentication examples and runtime activation.

Limits: test counts do not establish semantic correctness or prevent a candidate
from replacing tests. Suites inherit the environment and are not OS-sandboxed.
Stop is checked during test supervision and before promotion. It stops a running
suite (including Windows process-tree cleanup), skips subsequent work and records
rollback. Failed or empty baselines also skip mutation and candidate execution.
Git operations and mutation callbacks are not interruptible; Unix descendant
cleanup remains incomplete. Extensions requiring authentication, unsupported UI controls, external
installation, or runtime replacement still need additional implementation.
