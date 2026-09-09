# Interrupted file-action recovery

Status: **IMPLEMENTED** for exact-content file-write postconditions and fresh file reads. **PARTIAL** for general tool recovery; shell and other effects are refused.

## Protocol

`RunState::next_action_id()` derives a run-scoped ID from persisted objective ID and history position. New `ActionPrepared` and `ActionObserved` events carry versioned JSON with action ID, action, and optional observation. Legacy informational tool events remain for compatibility. Pending state is committed before a tool is invoked. Version-1 checkpoints remain compatible because IDs are derived from existing fields.

`--resume` preserves conservative refusal of pending actions. `--reconcile` opts into observation-based recovery:

1. Validate checkpoint version and canonical workspace identity.
2. Recheck both the original action permission and read permission.
3. Observe the target through the registered filesystem tool.
4. For writes, compare exact contents with the persisted action. Never invoke a replacement write. For reads, retain the new observation.
5. Record reconciliation evidence with the original action ID, checkpoint the updated history, clear pending state, then continue the normal loop and independent completion verification.

Missing files, mismatches, permission rejection, and unsupported actions leave the pending checkpoint intact. Persistence errors propagate. Retrying after a reconciliation-checkpoint failure can repeat observation/audit records but cannot duplicate the write.

## Evidence and limits

Tests kill a real worker after `ActionObserved` but before the completion checkpoint. Ordinary resume refuses it; reconciliation completes with one original write invocation and the same ID across prepared, observed, and reconciled records. Additional tests cover matching-file modification timestamps, missing/conflicting/forbidden paths, pending reads, shell refusal, and injected reconciliation-checkpoint failure.

An existing matching file establishes a postcondition, not historical causation. Advisory locks and lexical/reparse-point checks retain the limitations documented in README. Reconciliation reads currently have no independent persisted call budget or size limit. A terminal run still returns its historical outcome without revalidation. IDs are unique only within a run/database; event consumers must retain database/run identity when aggregating records.

The file-core-v1 benchmark after this change: 10/10 positive tasks verified, 10/10 negative cases denied, 30 normal tool calls, 40 deterministic model decisions, 0.5117077 seconds in one Windows debug sample. This benchmark does not measure reconciliation throughput or broader coding intelligence.
