# Capability-wide execution repair, reconstructed on Windows

This implementation was reconstructed from the supplied capability-wide report after the original patch was deleted. It extends checkout `1391554`; it is not a recovered copy of that patch. Earlier reliability and report-contract repairs remain in place.

## Behavior

- `resource_route {operation,path?,format?,existing_session?}` provides ordered routes for `create_file`, `read_file`, `edit_file`, `convert_file`, `open_file`, `research`, `browse`, `api`, `command`, `recall`, `build_tool`, and `runtime`. Routes explain prerequisites and verification. They never dispatch actions or grant access. A pending effect blocks routing. Existing browser-session requirements exclude an isolated browser fallback.
- `artifact_read {path,offset?,length?}` returns base64 pages of at most 8192 bytes, a whole-file digest, size and continuation offset. `artifact_write {path,encoding:"utf8|base64",data}` publishes without replacing different existing contents, reads back the destination and produces a digest receipt. Identical existing contents are accepted without rewriting.
- `artifact_inspect {path}` reports available checks. `artifact_verify {path,expected_sha256?}` explicitly separates byte integrity from format structure. Unknown formats without an expected digest require a real adapter. A digest never proves meaningful content or rendering.
- Artifact files are limited to 16 MiB in this process. ZIP inspection reads CRC-checked contents, with 512 entries, 8 MiB per entry, and 32 MiB cumulative decompression limits. Unsafe entry paths and symlink entries are refused. JSON syntax, bounded UTF-8 XML/SVG parsing without DTDs, and required Office/OpenDocument package parts are checked. These are structural checks, not full schemas or application validation. Other text formats receive UTF-8 checks only; images, media, PDF and proprietary decoding require real adapters.
- `document_create {path,title?,paragraphs:[...]}` creates a minimal DOCX with an independent text roundtrip. `document_read` extracts DOCX text in addition to its existing text/PDF paths. This does not open Word or certify layout. Creation accepts up to 1024 paragraphs and 1 MiB of text; extraction is truncated at a Unicode boundary after 60 KB.
- Ordinary `write_file` refuses common binary/document/media extensions, case-insensitively. Plain prose cannot acquire a successful text-write receipt by being named `file.xlsx` or `image.png`.
- `tool_save` accepts bounded, unique lowercase `formats` extensions. `capability_list {format?,query?,offset?}` filters before pagination. Route discovery finds matches beyond the first unfiltered page. Metadata and persisted `tested` flags remain advisory; execution still uses existing executable/argv/input qualification and command policy.
- Routine malformed route/artifact arguments become failed, known-not-applied observations. Reviewer writes remain denied. Artifact operations participate in the durable stall journal. Before publication, Studio saves the intended path, byte count and digest. Restart reconciliation only reads: exact bytes produce a receipt; absent, partial or changed contents remain unresolved.
- Runtime status separates the running executable from configured activation and the last activation result. The readiness endpoint includes server identity and serving status while preserving its existing idle `ready` flag. The launcher checks reported data-directory identity when available, without requiring a busy recovered runtime to be idle.

## Validation

Windows/MSVC host execution with deterministic model fixtures was used. Focused checks cover binary pages/no-clobber, invalid inputs, reviewer denial, cancellation, Unicode DOCX extraction, malformed XML/JSON, corrupt ZIP CRCs, package mismatch, format discovery/qualification, and exact-byte reconciliation. End-to-end chat fixtures cover malformed-argument recovery, native DOCX, a real loopback App API response saved as source-linked JSON with Web disabled, and actual terminal output inspected as CSV.

The workspace regression command passed, including core, provider, memory, experiment, supervisor/runtime and Studio suites:

```powershell
cargo test --locked --workspace --exclude harness-browser --no-fail-fast -- --test-threads=1 --skip production_view --skip chat_browser --skip recovery_screenshot
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all --check
git diff --check
powershell -NoProfile -File scripts/test-launcher-path.ps1
```

The initial sandboxed chat run had four desktop/browser fixture failures. The unrestricted Windows run passed those same cases: 73 passed, zero failed, three filtered out. A concurrent rebuild also hit Windows executable locking while tests were using that binary; builds/tests were subsequently serialized. Neither environment failure was fixed by weakening tests or permissions.

After the final restart fixture, size-limit checks and runtime-status refinements, Studio was rechecked separately: **118 unit tests passed, three ignored; 74 chat integration tests passed, three filtered out**. The last XML name/attribute/character-reference checks passed in the focused artifact suite (**six passed**); strict workspace Clippy remained clean. The restart fixture reconstructs the saved-intent/publication boundary in a disposable SQLite conversation, then starts a real Studio process; it is not a random power-loss experiment.

No live language model, authenticated user service, Microsoft Word rendering, or user's signed-in browser session was evaluated. Browser fixtures that passed used disposable pages. No general-purpose autonomy or universal format support is claimed. The three explicitly filtered UI/browser cases and the standalone harness-browser suite are not part of this run.

## Build and use

```powershell
cargo build --locked -p klyne-studio --bins
```

An already running or separately activated runtime does not change merely because source was edited or built. Use the existing [activation instructions](voice.md#updating-an-existing-runtime), then inspect the running executable and activation result. The new `runtime_status` distinguishes these states. Keep prompt-maker mode off for execution tasks, select the actual provider, and enable only the needed routes.

For live acceptance, use disposable files and check actual destination content for create/edit/convert/open tasks. Include missing adapters, disabled permissions, an existing-session requirement, and interrupted publication. Inspect rendering with the real application where rendering is part of the goal.
