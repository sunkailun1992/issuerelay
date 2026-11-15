# Verification evidence retention implementation

1. Add an engine-level regression using the native process fixture; confirm both cleanup loss and false passing on evidence loss.
2. Implement bounded private evidence preparation and publication with host-generated report and integrity index; test filesystem boundaries.
3. Retain after process cleanup and before scratch cleanup. Include the durable index in the existing verification evidence view; downgrade retention errors to blocked.
4. Update user and chat documentation, code ownership, and lifecycle assertions. Preserve current confirmation, executor and delivery rules.
5. Run the required build, Rust, frontend and secret checks, review the exact diff, and integrate local main without publishing a package.

Design: [durable verification evidence](../specs/2026-11-16-issuerelay-verification-evidence-design.md).
