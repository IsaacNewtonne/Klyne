# Working workspace reference

`working-ui-reference.png` is the user's supplied visual direction, preserved
unchanged. The working view uses its dark panels, orange central ring, tool
cards, right status column and bottom live activity log.

The ring is drawn in canvas; no embedded screenshot or fabricated dashboard
values drive the interface. Runtime status drives its motion, pending actions
light their tool card, recorded evidence fills the log, and the task graph fills
the step list. Studio CPU uses the existing telemetry endpoint and is explicitly
process-scoped. Missing telemetry displays a dash. Stop uses the existing Stop
action. Conversation and Details remain available.

The canvas is capped at 30 frames per second and a 1.5 device pixel ratio. It
stops animating when idle, hidden, or reduced motion is requested. Narrow layouts
place the ring above a two-column tool grid. Starting a goal opens the working
view, and the user can switch back to conversation.

The reference's memory/disk figures, completion-time estimate and simultaneous
tool activity are not claimed by the current runtime and are not reproduced as
fake metrics. The local preview identifies its simulated data explicitly.
