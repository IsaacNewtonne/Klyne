# Typed decisions, document ingestion and procedure reuse

Implemented as optional additions to Studio. Settings → Routing and learning controls both features; both default off. The document reader works without Jev. Existing chat permissions, budgets, reviewer policy and acceptance checks remain authoritative.

## Decision routing

Set `TYPESAFE_API_KEY` in the environment of the process that launches Klyne, then enable TypeSafe Jev decisions. Settings accepts the environment variable **name**, never the key. The default model is `jev-latest`. No credentials were configured or live paid calls made during implementation.

Once per new non-greeting task, the service receives the original request and access flags. It returns host-defined workflow and complexity choices. Confident complex tasks can use the explicitly selected specialist; the separate review connection remains independent. Low-confidence workflow labels become `general`. Missing keys, malformed responses and service errors fall back to the ordinary worker without changing access. Resume preserves the attempted routing rather than silently repeating a billed call.

Only the official TypeSafe HTTPS endpoint accepts a credential reference. Credential-free loopback endpoints support tests. Requests do not follow redirects or retry automatically. Known granted secrets and the configured API key are scrubbed before sending decision state and saving chat history. The document content itself is sent when classification is requested.

Calls share task steps, elapsed time, metered tokens and estimated spend. Configure input USD per million tokens to meter spend; zero means unknown price, not free. Spend-limited tasks refuse an unpriced decision request. Valid usage is retained even when typed answers are rejected. A timeout, malformed JSON or missing usage can leave provider billing unknown; these counters are not a provider billing ledger. Limits are checked before and after a call, so one call can cross a limit.

The schema checks exact answer keys, allowed choices, probability bounds, distribution totals and selected maxima. A confidence threshold is a heuristic, not a calibrated probability of success. See the [TypeSafe quickstart](https://docs.typesafe.ai/introduction/quickstart) and [confidence definition](https://docs.typesafe.ai/confidence).

## Documents

`document_read {path}` extracts a workspace UTF-8 text file or text-bearing PDF. `document_classify {path}` additionally sends extracted text to the enabled service. It asks for one primary category and independent overlapping category scores; a document can contain both invoice and receipt evidence. Classification does not move or modify files.

Inputs are capped at 4 MiB, extracted text at 60,000 UTF-8 bytes. The result includes a content hash and truncation flag. Classification flags low-confidence, `other`, or truncated inputs as needing review. It does not disclose the filename to the classifier. Scanned PDFs and image files need a separate OCR step, which this implementation does not provide. Encrypted and unsupported PDFs return errors.

PDF parsing uses a hidden child process with cleared environment, bounded output, cancellation and a ten-second deadline. The child is crash-isolated; this is not an OS memory sandbox. The parser dependency is pinned in Cargo.lock.

## Procedures

This is evaluated reuse of plans, not model-weight training or automatic code rewriting. A task needs caller-owned acceptance criteria, enabled learning, no unresolved action and freshly verified output. A model's completion claim alone cannot create a procedure. Work already satisfying the criteria at the initial check does not qualify.

The library stores bounded, scrubbed instructions and expected results from the successful task. Matching requires the exact original goal, acceptance contract and workspace scope. Private workspaces share a scope so a scheduled task can qualify on new files in a fresh workspace; host projects match their actual path.

A candidate is supplied as advisory context to later matching runs. Two later distinct runs must independently pass before it becomes active. A failed run rejects the selected procedure; an existing standby version can be restored. Run IDs prevent duplicate credit. The library is capped at 500 records with up to 32 recent trials per record. Settings lists the latest 100 records and allows retirement. Schedules capture the source task's decision and learning settings.

Passing proves the specific acceptance checks passed on those runs. It does not establish that the plan caused the success, improved efficiency, generalizes to other work or understands document semantics. This version does not generate stronger-model rewrites, run comparative holdout benchmarks, or replace source code. Protected code-improvement experiments remain a separate subsystem.

## Validation

Local fixture tests cover specialist/reviewer separation, malformed-route fallback, usage accounting, content-only overlapping classification, real PDF extraction, workspace boundaries, UTF-8 truncation, promotion after subsequent verified runs, rejection, retirement, idempotence, and refusal to credit claimed or preexisting success. Browser settings tests cover submission, persisted defaults and saved-chat overrides. Fixtures verify the integration and host rules; they do not benchmark Jev accuracy.
