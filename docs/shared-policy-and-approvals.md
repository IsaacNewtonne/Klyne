# Shared policy and approval lifecycle

Klyne derives a versioned `host_policy` from the user's workspace, access routes,
command policy and budget settings. It persists the projection under
`execution.policy_snapshot` and supplies it to every planner, worker and reviewer
request. The reviewer remains read-only. Model decisions and project files do
not update these settings; dispatch uses the same underlying host-owned values.
`AGENTS.md` is not treated as an executable permission configuration.

The accompanying instruction summary treats retrieved content as data, permits
routine implementation choices within the authorized scope, requires honest
partial results, and asks for essential missing information. These behavioral
instructions do not constitute a prompt-injection security boundary. Existing
tool gates enforce access; Terminal retains same-user host authority rather than
filesystem, network or account isolation.

## Task scheduling

The serial scheduler saves approval proposals with their task index and continues
other ready tasks. A task waiting for approval and its dependents cannot execute.
Independent tasks must explicitly have `depends_on: []`; omitted dependencies
retain the existing sequential default. Once no independent task is ready, the
conversation presents the next approval card. It cannot report completion while
tasks remain blocked or declined.

Approving requeues the blocked task. Declining leaves that task and its dependents
unfinished, requiring a revised instruction. The queue persists across restart.
This uses the existing serial scheduler; approval submission is available after
the active run pauses, not concurrently with an executing worker. An uncertain
dispatched effect still stops the run for reconciliation.

## Approval identity, expiry and context

New cards carry a unique `request_id`, creation time, a 15-minute expiry and a
context digest. Resolution must supply the displayed `request_id`; stale tabs
cannot approve a different queued request. Reading and resolving are serialized.
The browser submits this identity automatically and shows expiry and queue count.

The host checks expiry and context both when approving and before later dispatch.
The context binds the goal and policy. Shell context additionally binds the
resolved executable and existing file arguments using the tool qualification
fingerprint. DELETE dispatch reconstructs and compares the exact connection,
origin, method, concrete path/query and body digest. Changed inputs require a new
proposal. This does not cover every implicit shell dependency or assert that a
remote resource's state is unchanged; APIs without conditional writes retain that
limitation.

Approval-generated grants expire with their card or when its bound context
changes. Existing explicit user-supplied durable grants retain their separate
semantics. Secret card grants are stored separately so a temporary origin cannot
be merged into a durable secret grant. Identical permitted calls can reuse a
valid grant during its lifetime; this is not an exactly-once remote transaction.
An explicit user-supplied shell grant supersedes temporary cards for that exact
invocation; an explicit secret grant supersedes temporary cards for that secret
name. Previously temporary origins are not promoted to durable authority.

An expired or legacy card grants nothing. Resolution returns
`renewal_required: true`, and the browser resumes the task to obtain a fresh
proposal for review. Expiry never resolves a dispatched action's uncertain
outcome. Approval journal entries are labeled proposed, queued or resolved,
separately from actual tool dispatches and returns.

## Shared budgets

Workers and review rounds consume the same goal counters. Resume preserves step,
token and estimated-cost usage; a new goal resets them. Increasing an exhausted
limit is an explicit user setting change. Active elapsed time is carried between
normal resumptions, excluding approval wait time; an abrupt process death can
lose the current segment's elapsed accounting. Provider usage is metered after
responses, so token/cost limits can overshoot by an in-flight call and do not
guarantee exact billing caps. Per-worker allocations are not implemented.

## Verification

Regression coverage includes independent work while two approvals wait, blocked
dependencies, persisted queue recovery, stale card rejection, policy inheritance
across all roles, expiry without execution, changed script inputs, expiring
secret grants alongside durable grants, and budget preservation on Resume.
The workspace run is logged in `workspace/policy-workspace-tests.log`.

Result: **274 passed, zero failures, 10 ignored**, including **34 Studio chat
integration tests** against controlled providers and real Chrome. The final
temporary-to-durable grant adjustment also passed the targeted approval tests.
Workspace Clippy (all targets/features, warnings denied), formatting and diff
whitespace checks pass. Ignored subprocess/live fixtures are not counted as
passes. This validation does not measure live-provider prompt-injection resistance
or arbitrary application task success.
