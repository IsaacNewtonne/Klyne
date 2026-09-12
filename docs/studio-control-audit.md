# Studio control audit — simple interface

## What changed

The interface now uses an ivory and sage workspace with a prominent file form,
a live status and evidence panel, and compact searchable run cards. Tool-call
settings and raw events use native disclosure controls. The decorative header
breathes subtly; entry and interaction transitions respect reduced-motion
preferences. The layout becomes one column on mobile. All existing run,
stop/resume, search, and export controls remain available.

The browser flow opens the event disclosure before filtering. All five Studio tests passed after the redesign, including isolated Chrome
interaction tests, real evidence download, stop/resume, and mobile overflow.
Desktop and mobile screenshots were inspected; the mobile decorative graphic
was subsequently moved above the headline to prevent visual overlap.

## Checked controls

| Area | Chrome checks |
| --- | --- |
| Run creation | Path validation rejects traversal without creating runs; successful creation completes; markup in file contents is shown as text, never executed |
| Run list | Filter, no-results message |
| Evidence | Three tool observations listed per file run |
| Ledger | Filter, no-results message |
| Export | Actual headless Chrome download; resulting JSON read from disk and checked against the selected completed run |
| Stop/resume | Browser stop-button error/success wiring via a controlled fixture; real persisted checkpoint resume; controls hidden after completion |
| Status | Error line empty on success; no console errors or unhandled rejections |
| Responsive | 1280px and 390px screenshots; no mobile horizontal overflow |

## Test boundaries

The GUI stop-button test uses a controlled active-state/stop-response fixture
because deterministic file tasks finish before a human can reliably click Stop.
A separate native HTTP test exercises the real stop endpoint, confirms the
runtime shutdown handle is signalled, checks the resulting durable checkpoint,
and resumes through the real endpoint to independently verified completion.

Creation, read-back, actual downloads, checkpoint resume, and server restart
persistence use real local resources. Browser automation uses isolated Chrome;
other browsers and physical touch interaction have not been verified.

Screenshots and final test log: `workspace/studio-qa/` (git-ignored).

## Usability continuation — 2026-09-10

Added editable note/checklist starters, clearer file-name guidance and a Create
file action with pending feedback. Run identifiers, provider information and
budgets now sit inside Run details. Result cards respond subtly to run state;
existing reduced-motion rules apply. Unchanged history markup is retained during
polling, and focused run buttons are restored when the list changes. Refresh
feedback now reports connection failures accurately, with a retry message while
opening a run.

JavaScript syntax passed. Initial verification passed three unit tests and three
HTTP/provider integration tests; both browser tests could not start Chrome's
debugger inside the sandbox. An outside-sandbox rerun was requested.

Outside-sandbox rerun: all eight Studio tests passed (three unit, five integration). Desktop and 390px mobile screenshots inspected; browser flow checks mobile overflow and console errors.

## Chat-first replacement — 2026-09-10

The default route now opens a chat workspace; the file form is retained only at
/files. See studio.md for the goal driver, actual tool grants and limitations.
All 14 Studio tests passed, plus a real Codex planner/worker/reviewer welcome
request. The new chat desktop/mobile screenshots were visually inspected.
