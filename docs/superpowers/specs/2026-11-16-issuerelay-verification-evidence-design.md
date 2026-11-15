# Durable independent verification evidence

## Problem and behavior

The verification launcher deletes its disposable source copy before recording the result. Evidence strings can therefore point to reports and screenshots that no longer exist. Keep the source copy disposable, and retain only an explicitly reserved evidence directory before cleanup.

The host creates `.issuerelay-evidence` exclusively after staging the reviewed delivery. The verifier writes its report and attachments there. After the runner has ended the entire process tree, the host copies regular files into a private temporary directory next to `<data>/verification-evidence/<job-id>`, adds `result.json` with the bounded native final output and run identity, generates `index.json`, and atomically publishes the directory. Worker attachments live under `files/`, separate from the host files.

The index lists every payload's relative path, size and SHA-256. It includes the host report, but does not hash itself. Existing run details, repair references and MCP `get_run` receive the durable index path through the existing verification evidence field. No new command, write permission, confirmation route or database schema is required.

## Boundaries

- Evidence text is descriptive, never a filesystem read request. The host does not follow absolute paths named by the model.
- Retain at most 256 worker files, 16 directory levels, 16 MiB per attachment and 64 MiB total. The bounded host report may use at most 128 KiB of that budget.
- Reject symbolic links, reparse points, unsupported file types, invalid portable path names and multiply linked Unix files. Bound reads and check the opened file's identity and size. Normalize trusted operating-system roots before validating the reserved descendants.
- Do not retain the whole source copy, private configuration or credentials. Files and directories are private to the current user; Windows uses the existing current-user application-data access boundary.
- An empty attachment directory still produces the host report and index. A missing or invalid directory, a conflicting destination or any copy/index failure blocks acceptance. Never record a passing result without a published index.
- Existing delivery, task revision, session, cancellation and completion checks remain in force. Retention is local storage, not independent proof that a model's conclusion is correct.
- Existing evidence is never overwritten or automatically deleted by this change. Source cleanup and log retention do not remove the evidence store. A process crash during publication may leave an unpublished temporary directory; it does not produce a valid index receipt.

## Verification

Reproduce the old failure through the real engine and native process fixture: files disappear after scratch cleanup, and removing the attachment directory incorrectly permits a passing result. After repair, check passed, failed and blocked conclusions, hashes, nested files, engine reload and persistent MCP-visible references. Exercise path, link, file type, byte/count/depth limits, target conflicts and partial-publication failures separately. Run all required repository checks before integrating local main. Real provider and platform results remain separate from these synthetic checks.
