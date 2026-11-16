# Durable independent verification evidence

## Problem and behavior

Verification evidence is retained after the runner returns. While it is still running, however, its only raw reports and screenshots live in a system-temporary source copy. A machine restart can remove that copy before normal retention runs. Move activated attempts into host-owned durable staging, then recover only their reserved evidence as partial, unverified material after interruption.

The host creates a private, per-attempt workspace under `<data>/verification-work/`, stages the reviewed delivery into its fixed `work` child, then creates `.issuerelay-evidence` exclusively. Only `work` is granted the existing verification sandbox's write permission. Before spawning, the host durably writes an exclusive ownership marker outside that write root, binding the run, started attempt and approved source fingerprint. Prepared but unstarted directories remain disposable; activated attempts never disappear merely because a host task is dropped.

The verifier writes its report and attachments there. After the runner has ended the entire process tree, the host copies regular files into a private temporary directory next to `<data>/verification-evidence/<job-id>`, adds `result.json` with the bounded native final output and workspace identity, generates `index.json`, and atomically publishes the directory. Worker attachments live under `files/`, separate from the host files. Normal workspace cleanup occurs only after both publication and the result transaction succeed. Retention or result-recording errors preserve staging and emit a warning.

The index lists every payload's relative path, size and SHA-256. It includes the host report, but does not hash itself. Existing run details, repair references and MCP `get_run` receive the durable index path through the existing verification evidence field. No new command, write permission, confirmation route or database schema is required.

## Boundaries

- Evidence text is descriptive, never a filesystem read request. The host does not follow absolute paths named by the model.
- Retain at most 256 worker files, 16 directory levels, 16 MiB per attachment and 64 MiB total. The bounded host report may use at most 128 KiB of that budget.
- Reject symbolic links, reparse points, unsupported file types, invalid portable path names and multiply linked Unix files. Bound reads and check the opened file's identity and size. Normalize trusted operating-system roots before validating the reserved descendants.
- Do not retain the whole source copy, private configuration or credentials. Files and directories are private to the current user; Windows uses the existing current-user application-data access boundary.
- An empty attachment directory still produces the host report and index. A missing or invalid directory, a conflicting destination or any copy/index failure blocks acceptance. Never record a passing result without a published index.
- Existing delivery, task revision, session, cancellation and completion checks remain in force. Retention is local storage, not independent proof that a model's conclusion is correct.
- Existing evidence is never overwritten or automatically deleted by this change. Source cleanup and log retention do not remove the evidence store. A process crash during publication may leave an unpublished temporary directory; it does not produce a valid index receipt.

## Interrupted host recovery

Before the first scheduling cycle, after the existing executor-leftover sweep, inspect only the fixed private workspace root. A bounded plain ownership marker must match the directory, verification run, exact started attempt and current acceptance source fingerprint. Unknown markers, stale or multiple matching attempts, pending process records and mismatched approvals are preserved with warnings; they never authorize reads of model-provided paths.

For a matched interrupted run, publish only its fixed evidence directory through the same path, file-type and budget checks. The host report explicitly marks interruption, partial/unverified evidence and no complete worker final. Recovery records `verification_blocked`, attaches the validated index through existing run details, and never interprets a stored final as a passing verdict or resumes business actions. If publication already completed before the result transaction was lost, validate the exact workspace identity and every indexed payload before reusing that immutable index. Conflicts and corruption are never overwritten.

Recovery preserves the run's original interrupted `finished_at`, native session and reported usage. The new partial host report carries its separate `recovered_at`; that evidence timestamp is not a run duration. Identical recovered or unavailable conclusions cause no record update or job event on later initialization, while already published payloads are still checked for corruption.

Interrupted raw staging remains available for inspection, including after partial publication: recovery has no fresh runner-completion proof for every potentially unrecorded old writer. Missing or over-budget evidence is reported unavailable and staging is preserved. Existing pre-feature system-temporary runs have no trusted descriptor; this change cannot reconstruct their lost files. No new database schema, permission, network setting or MCP write route is introduced. The panel distinguishes interrupted/partial recovery from a business verification failure in all five locales and keeps old generic interruption reasons readable.

## Verification

Reproduce the old failure through the real engine and native process fixture: files disappear after scratch cleanup, and removing the attachment directory incorrectly permits a passing result. Also exercise an active verifier that wrote raw but returned no final, then abort only the private synthetic host runtime and restore its persisted pre-crash running row before reloading the engine. This models the distinction between Rust task destructors and an abrupt machine stop without killing an application or rebooting a host. Check partial recovery, forced Blocked status, stale/multiple attempts, wrong markers/fingerprints, budget and link rejection, immutable-index idempotence/conflicts and publication followed by a failed result transaction. Run all required repository checks before integrating local main. Real provider, physical reboot and platform results remain separate from these synthetic checks.
