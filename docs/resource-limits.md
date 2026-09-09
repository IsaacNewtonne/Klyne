# Tool-call and file-size limits

**IMPLEMENTED:** New runs persist a tool budget with `used` and `limit` counters. Default limit is 32; CLI `--max-tool-calls` and the runtime builder configure new runs only. Resumption uses persisted accounting. Verification and reconciliation share the same budget.

Before invocation, the runtime increments used credit and commits a checkpoint. Normal actions commit their pending intent with this reservation. No tool runs after a failed reservation commit. Reads that fail or cannot reconcile a write still consume credit. A crash between reservation and invocation may consume unused credit; credits are never refunded automatically. Exhaustion emits `BudgetExhausted` and returns an error while retaining recoverable state. This is not a terminal success or automatic budget extension.

Older checkpoints decode with absent accounting. Terminal results remain accessible; active legacy states refuse execution because prior reconciliation attempts cannot be inferred accurately from action history. No silent zero-accounting migration occurs.

**IMPLEMENTED:** Filesystem reads and writes have a hard 1 MiB limit. Reads require regular files and UTF-8, use a bounded reader with one extra byte to detect overflow, and return errors without partial successful observations. Oversized writes fail before creating directories or changing file contents. Reconciliation uses the same filesystem tool.

**VERIFIED:** CLI zero-budget denial and exact three-call completion; refusal to override resumed budgets; repeated failed reconciliation calls across store reopenings; verification charged; real process-kill reservation persistence; injected reservation failure preventing invocation; absent legacy accounting; byte boundary, invalid UTF-8, directories, and oversized reconciliation evidence.

Benchmark after initial limit implementation: file-core-v1, Windows debug, 10/10 positive tasks verified, 10/10 traversal cases denied, 30 tool calls, 40 deterministic model decisions, 0.4971951 seconds. This single sample measures the file task baseline, not throughput guarantees.

**PARTIAL:** These bounds do not cap model output, total history, database size, CPU time or wall time. File checks do not solve hostile path replacement races or establish an OS sandbox. Process supervision, time/token/cost budgets, explicit paused/blocked run states and controlled budget amendments remain planned.
