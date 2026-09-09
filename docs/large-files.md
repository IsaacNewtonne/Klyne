# Large-file operations

**IMPLEMENTED:** Three new typed actions in the Rust tool registry. The deterministic CLI objective adapter still understands only file creation; these actions are available to model implementations through the typed runtime interface. They receive the existing permission checks, persisted tool-call reservations and action audit records.

| Action | Inputs | Successful observation data (JSON) |
| --- | --- | --- |
| `ReadFileRange` | path, byte offset, byte length | offset, length, observed file size, EOF flag, UTF-8 text |
| `HashFile` | path | algorithm `sha256`, digest, byte count |
| `PatchFile` | path, byte offset, expected text, replacement text, expected whole-file SHA-256 | before/after digests and output byte count |

Ranges may select up to 1 MiB from a regular file of any size. Past-EOF requests fail instead of silently shortening. Empty ranges at EOF are allowed. Invalid UTF-8 (including a split code point) fails with an instruction to adjust boundaries. JSON escaping can expand observation size beyond the selected byte count.

Hashes use a 64 KiB buffer and reject streams above 64 MiB. Patches cap both expected and replacement operands at 1 MiB and input/output files at 64 MiB. Offsets are bytes, not characters or line numbers. The unchanged parts may contain binary data; patch operands are UTF-8 strings. Empty expected text inserts; empty replacement deletes. The mandatory digest guards either operation against stale input.

Patching checks the source digest and expected span, streams the prefix/replacement/suffix to a same-directory temporary file, preserves basic permissions, synchronizes staged contents, computes the output digest, rechecks permission and the current source digest, then publishes through temporary-file persistence. This avoids exposing a partially constructed file during ordinary replacement. [The tempfile API](https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html#method.persist) documents atomic replacement but does not synchronize the parent directory.

**VERIFIED:** Known SHA-256 test vector; range reads and variable-length patches above 1 MiB; unchanged prefix/suffix comparison; insert/delete; stale digest, wrong span, oversized operand, overflow and EOF rejection; UTF-8 errors; 64 MiB ceiling; path denials; temporary-file cleanup after success; runtime budget and checkpoint integration; refusal to replay a pending patch.

Run `cargo run -p harness-core --example large_file_benchmark --locked` for a controlled 8 MiB hash/patch/range test with independent full-byte verification. Its fixture and verification deliberately load the file; the production tool hashes/copies incrementally. It measures tool latency rather than general agent intelligence or peak memory.

**PARTIAL:** No dedicated CLI action command, general coding model, byte-range completion verifier, or recovery reconciler for these new actions. A pending patch/hash/range action currently refuses automatic reconciliation. Host power-loss durability, temp cleanup after process kill, ACL/extended-attribute preservation, I/O deadlines and adversarial concurrent mutation remain unresolved. Rechecking a digest is not a filesystem compare-and-swap: another process may change the target between the final check and replacement. Use a trusted, exclusive workspace. A hash of a concurrently modified file is not a guaranteed snapshot.

The original 1 MiB whole-file text limit remains in force.

Measured Windows/MSVC debug sample (2026-09-09): 8,388,608-byte fixture, three tool calls, 0.5237379 seconds, independent byte verification passed. Before digest: `e3cfb1faced3ece06f521842c90f25aabc49ec64b16f42a74e378652d86e7f28`; after digest: `2019289ba860e5f434ccbb756b5124885f59f02ff628ae9cab002ef20dce5163`. One sample, not a performance guarantee.
