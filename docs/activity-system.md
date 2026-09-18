# Activity-driven workspace

The existing orange workspace is the default saved-task view. The compact header, centered model ring, tool positions, and composer stay in place.
Details, steps, and observations open on demand; Conversation remains an explicit toggle. The ring and tool cards can be
clicked or reached by keyboard to inspect their current state and observations.

## Source of truth

`chat_store.rs` appends visual events in the same SQLite transaction as task
metadata. The snapshot carries the latest 256 sequenced events. Each tool dispatch
has an operation ID shared by its return event. Calls that finish between UI polls
therefore retain a result transition. First load reconstructs state without
animating historical completions. Approval proposals do not become active tools.
Evidence is an observation, not proof that the overall objective succeeded.

`activity.js` reduces these events into a map of operations keyed by ID. Separate
operations can share a category, and ending one does not end the others. The current
host executes assignments sequentially; the presentation supports parallel events
without manufacturing parallel execution.

Events include `tool:start`, `tool:waiting`, `tool:result`, `tool:complete`,
`tool:error`, `workflow:stage_changed`, `observation:new`, `klyne:waiting`,
`klyne:thinking`, and measured system CPU/connection updates. Tool data contains
`id`, `action` (or `tool` category plus `operation`), and `direction`. Integrations
must report waiting explicitly; elapsed time never changes execution into waiting.
The existing synchronous tool adapters report dispatch and return, not internal
streaming progress. Model requests and the desktop access queue report waiting.

## Motion rules

- Working: energetic reactor waves; only dispatched tool icons and cards animate.
- Waiting: slow breathing and a circumference pulse, with the actual wait reason.
- Return: one inbound connector packet and a brief completion or failure effect.
- Idle, stopped, needs input: calm reactor; input attention belongs to the composer.
- Failure: brief interruption followed by a steady error indication.
- CPU: actual Studio process measurement only; external model CPU is excluded.
- GPU: whole-device NVIDIA utilization when available, including other apps;
  this is not a per-model measurement.
- Progress: completed action steps when known; no invented percentage without a plan.

Connector nodes persist across renders, so unrelated tool updates cannot restart
another request packet. Geometry is measured on layout updates, not every drawing
frame. Reactor motion is capped at 30 FPS and stops when idle, hidden, outside the
viewport, or in reduced-motion mode. Timers only settle finite visual effects;
they never declare that a tool completed.

## Verification

`node scripts/test-activity-ui.cjs` drives the real UI with isolated event fixtures:
parallel categories, waiting, independent completion, persistent failure, ring
inspection, mobile overflow, and reduced motion. The Rust
`visual_events_survive_short_calls_without_replaying_history` test covers durable
dispatch/result correlation and excludes approval proposals from execution.

## Lifecycle and event contract

Every simultaneous operation must provide a distinct stable ID, including calls
to the same tool. A start sends one outbound packet, an explicit result sends one
inbound packet, and completion removes only that operation. The UI does not loop
request packets to pretend a service is streaming.

The reducer owns the 800 ms feedback expiry, independent of the visible view.
Disconnect clears live operations; reconnect reconstructs from sequenced records
without replaying past success or failure effects. Thinking and reviewing events
normalize to the working ring; a model request in flight means waiting, including
reviewer requests. Backend dispatch IDs use the durable action row ID.

Ambient workspace particles are removed. The reactor emits a small bounded set
only in working state, directed using the actual active card's angle. The ASCII
mark is static outside working. CPU is measured independently and does not imply
a tool call. Registered CSS CPU angle transitions smooth actual readings.

The pipeline uses the existing metadata row. Unknown progress is indeterminate,
completed step counts remain real, and task completion has one finite highlight.
Observation highlights are batched to at most one per 300 ms and stay local to
the observations control. No saved conversation replays a failure flash.

Additional checks: scripts/test-activity-state.cjs covers concurrent IDs
within one category, direction, expiry, aliases, CPU, reconnect and history.
The browser suite checks waiting/working canvas changes, static completion,
reduced motion, model labels, details, responsive fit, and input attention.

## Connected cards and layout transitions

The existing two tool columns retain their order. Saved API connections from
GET /api/apps add one card per connection; saving or forgetting a connection in
Settings updates immediately, and background refresh discovers backend changes.
Installed but unconnected programs are not presented as API connections.

Named app calls carry app:<connection-name> as their visual category, so only
that app card and route light up. Icons are local deterministic SVG identicons
derived from each connection name; no remote image or credential request is made.

Card identity and focus survive resize/reflow. A bounded 380 ms FLIP transition
interpolates from the last visible rectangle to the new layout; interrupted
transitions begin from their current on-screen position. Removed cards fade out
for 180 ms. Connector routes fade while layout is in flight and return at their
new endpoints. Reduced motion applies the final layout immediately. Extra rows
can scroll once more than six rows are connected.

## ASCII sun reactor

The central canvas renders a monospace corona with curved prominences. Working
intensity combines the working state, active operation count, and measured Studio
CPU; waiting remains a patient low-energy state. Mouse/touch proximity pulls the
nearby corona outward without moving the provider label. Energy changes ease to
their targets, then idle stops requesting frames. Reduced motion draws one static
state and disables pointer distortion. The glyph grid and flare count are bounded.

Welcome-screen exception: the welcome flame and interactive background embers
animate whenever the welcome screen is visible. This is intentional ambience,
independent of the task telemetry rules. In task view the background canvas hides
and its loop stops; the ASCII sun continues to follow task state. The release
suite includes test-welcome-effects.cjs to check animation, view switching, and
reduced motion so telemetry changes cannot silently disable welcome effects again.
