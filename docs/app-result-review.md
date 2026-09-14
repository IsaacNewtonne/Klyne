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
tasks to repeat the operation after this rejection, including after Resume.
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
