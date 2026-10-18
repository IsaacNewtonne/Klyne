# Independently checked Markdown reports

In Studio, open **Settings → Report requirements**, enable the checkbox, and
specify the report path, minimum body length, required section headings and
optional required source URLs. The settings apply to the next submitted task.
They are cleared after successful submission and when starting a new chat.
Resume retains the original task's requirements; changing settings cannot
weaken that task's saved contract.

The host reopens the actual saved file before accepting completion. Model
claims and supportive reviewer responses cannot substitute for these checks.
Results expose each check as `passed`, `failed`, or `inconclusive`. Failed and
inconclusive checks both prevent Verified completion. Details appear in the
result inspector. No evaluator supplied by a model is executed.

## What is checked

- A workspace-relative `.md` or `.markdown` path with no traversal or ambiguous
  separators. Existing path components are checked against workspace policy at
  admission and again on verification. The output need not exist at admission.
- A complete UTF-8 file, bounded by the existing 1 MiB read limit. Read errors,
  missing files, invalid UTF-8 and over-limit files produce inconclusive checks,
  never partial-read success.
- At least the requested number of non-whitespace Unicode characters in parsed
  body text. Headings, fenced/indented code blocks and image alt text do not
  count. Inline code does count as body text.
- Exact, case-sensitive visible section headings, with whitespace normalized.
  ATX (`## Heading`) and Setext headings are supported, including inline
  formatting. Headings nested in lists or blockquotes are not document sections.
- Every required source URL must be an actual parsed Markdown link destination
  or autolink. Reference-style links are supported. Bare URL text, images, unused
  link definitions and links in code blocks do not count. Matching is exact;
  redirects and URL equivalence are not inferred. Extra references are allowed.
- Raw HTML produces an inconclusive result because visibility cannot be checked
  from Markdown alone. Use Markdown syntax instead.

Requirements are bounded to 1–32 unique headings of at most 200 bytes each,
0–32 unique HTTPS source URLs of at most 2,048 bytes each, and a positive minimum body length no greater
than 1,048,576 characters. The existing whole-contract 16 KiB limit still applies.
URLs are parsed and reject embedded credentials, invalid ports and whitespace.
No network access is performed by the verifier.

**Verified means these explicit structural requirements passed.** It does not
mean the report is factually correct, the sources were retrieved, the sources
are trustworthy, or the references support the claims. Those remain separate
research and review responsibilities. Studio labels this result “Report
requirements verified.” Existing file contracts and historical results retain
their meaning. Ordinary conversation requires no artifact.

## API

Send the contract alongside the user message to `POST /api/chats`. The `goal`
must exactly match `message`. Requirements originate with the caller, not a
worker-generated evaluator or completion claim.

```json
{
  "message": "Compare the options and save a report",
  "contract": {
    "goal": "Compare the options and save a report",
    "criteria": [
      {
        "ArtifactRequirements": {
          "path": "research/report.md",
          "min_content_chars": 500,
          "required_headings": ["Summary", "Comparison", "Sources"],
          "source_references": ["https://example.com/source"]
        }
      }
    ]
  }
}
```

Include the usual provider/access settings. Path selection grants no additional
permissions. An explicitly selected external project still requires Terminal
access. The new criterion is opt-in; no natural-language request is silently
converted into inferred report requirements. Existing FileContents, FileDigest
and FileRange contracts remain supported.

Older runtimes do not recognize the new criterion. Preserve/export conversations
using it before reverting to a binary that predates this feature; old file-only
conversations remain readable by the new runtime.

## Regression coverage

`crates/harness-core/tests/artifact_requirements.rs` covers independent reads,
false headings/references, raw HTML, malformed and out-of-scope criteria,
Unicode, missing/oversized/non-UTF-8 files, legacy serialization, and binding
observations to the correct target. Studio's `report_contract` integration tests
cover supportive false review, failed save, admission, immutable resume, and
process death after saving without duplicate writes. The browser fixture
`scripts/test-report-requirements.cjs` checks real form submission, result
limitations, resume behavior and ordinary messages without report contracts.

These are deterministic fixture checks, not a live-model research-quality
benchmark. This milestone does not introduce mail adapters, semantic tool
qualification, OS isolation, functional runtime canaries or automatic runtime
activation.

## Validation on 18 October 2026

Windows/MSVC, Rust 1.98.1. The selected Studio chat suite passed 69 tests with
three browser/UI cases explicitly filtered (`production_view`, `chat_browser`,
`recovery_screenshot`). Studio unit tests passed 110 with three ignored. The
five new core artifact tests and all three report integration tests passed,
including a real process kill after saving and an actual failed file write.
The full `npm test` UI suite, including the report form regression, passed.
Strict workspace/all-target Clippy, formatting and whitespace checks passed.

The workspace regression command excludes `harness-browser`; this is not a
claim of full browser-crate or live-provider evaluation. A final targeted core
rerun covered the added raw-HTML rejection, nested-heading handling and source
URL length bound after the broad suite was built.

```powershell
cargo test --locked --workspace --exclude harness-browser --no-fail-fast -- --test-threads=1 --skip production_view --skip chat_browser --skip recovery_screenshot
cargo test --locked -p harness-core --test artifact_requirements
npm test
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```
