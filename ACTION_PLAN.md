# ActionPlan and transaction contract

## Move plan

The UI submits selected SQLite file IDs and one Desktop-relative folder name.
Core creates a UUID plan and snapshots each source's size and modification
time. Source and target paths are derived by Core, not supplied by UI or AI.
A plan item is immutable once created. Executed or failed plans cannot be
executed again.

## Validation and policy

Validation verifies status, source boundary/type/snapshot, target boundary,
normal destination directory, absence of target collisions, and duplicate
targets. Preview returns checks, issues, and proposed paths. Execute reloads
the stored plan and repeats validation. Any issue blocks the entire plan.
A rule or LLM suggestion must be converted into this same plan format.

## Execute

Core writes a transaction journal first. It creates one destination folder
when needed, then moves items using Windows no-replace semantics. On failure
it moves completed items back in reverse order. It marks failed after
successful compensation or recovery_needed if compensation is incomplete.
A successful transaction becomes executed and enters history.

## Undo

Undo uses a transaction ID, not UI-provided paths. Before mutation it checks
every destination file still matches its snapshot and all original paths are
vacant. It moves items back in reverse order. If Undo itself fails, Core
attempts to compensate already restored items. It removes a transaction
created destination folder only if empty. History remains after Undo.
