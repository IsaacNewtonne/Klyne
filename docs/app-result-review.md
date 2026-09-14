# App result review

The completion guard no longer rejects a task merely because its goal mentions
sending a message. Desktop and browser operations use a shared destination-review
path, independent of app name. The original messaging guard rejected observed
success because it required a delivery receipt that no adapter produced.

The independent reviewer performs a read-only desktop observation, browser read
or screenshot, or a permitted URL fetch. It returns `observed_results`, binding
an effect, destination and description to an index in the current goal's evidence.
The host accepts only successful reviewer observations after the latest worker
operation. Worker prose, successful clicks, failed observations and stale indices
cannot satisfy this requirement. The citations are retained with the result.

The model interprets the destination, content and visible state; this is reviewed
evidence, not deterministic semantic verification or a delivery/read receipt.
The reviewer must distinguish an outgoing item from delivered or read status.
File receipts and caller-owned acceptance checks retain their independent checks.

If completion lacks evidence, the reviewer gets up to two further opportunities
to inspect or correct the result within the goal budget. It cannot create repair
tasks to repeat an attempted operation after this rejection, including after Resume.
If no tool operation was attempted, repair remains available: a worker merely
restating the request must not trap the goal in verification-only mode.
If verification remains unavailable, the task requests a destination check and
records `verification_needed`, rather than reporting an unclassified execution
failure. An actually uncertain dispatched action still requires reconciliation.

The UI distinguishes action-step progress from final review and labels completed
model-reviewed results Reviewed. Finishing the action steps alone no longer fills
the entire progress bar. Closing, switching or obscuring the destination may
prevent a fresh visual check; Klyne must report that limitation accurately.

Regression coverage includes a disposable browser chat: one send, a rejected
completion, a read-only destination check, then successful reviewed completion
without another send. Unit tests cover different destination labels and reject
worker-only, failed and stale observations. No real messages are sent by tests.

## Clarification discipline

Planner, workers and reviewer are instructed to use the latest user answers,
inspect display names in the app, and avoid asking about capitalization or
spacing alone. Exact content and identifiers retain their original semantics.
A proposed question contrasting quoted cosmetic variants of a group/contact/
profile label receives one bounded reconsideration before presentation, even
after a model-format retry. Questions about distinct observed matches, sign-in,
access, or missing content remain valid. This check does not resolve identity
itself or guarantee that a model will never ask another unnecessary question.

A conversation regression covers clarification, invalid JSON, a redundant
case question, and repair of unattempted work, while preserving a real sign-in
question. No real communication account is accessed by this test.

## Session selection and unsupported worker reports

A request naming an existing browser profile cannot use `browser_open`, whose
session is isolated. The host redirects that proposal to desktop observation
when Desktop is enabled, so the worker can locate the requested browser session.
Known authorized CDP endpoints remain available through browser_attach. Klyne
does not copy cookies or enable debugging on a personal profile.

A worker's send claim is rejected when its evidence contains only navigation,
launch, focus or observation. It receives bounded correction attempts and stays
unfinished if the claim persists. Navigation alone does not activate the
no-repeat restriction intended for potentially consequential operations.
Reviewers requesting internal tool output from the user receive a read-only
tool call instead, bounded to avoid repeating the same observation indefinitely.
Actual sign-in or access requirements may still need user input. These checks
do not establish that every model can reliably operate every app.
