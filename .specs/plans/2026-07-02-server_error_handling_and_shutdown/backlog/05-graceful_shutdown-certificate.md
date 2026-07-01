# Done Certificate — Task 05: graceful shutdown on SIGTERM

**Task:** [05-graceful_shutdown.md](05-graceful_shutdown.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> Verification protocol for Task 05. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** The non-Lambda server drains in-flight requests on SIGTERM or ctrl-c and exits
  deterministically within a 10 s hard deadline instead of aborting connections.
- **P2 — Obligations.** Done iff O1…O6 all hold, in DoD order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the Lambda-detection branch or introduce business logic
  into `main`.

## Obligations

- **O1 — SIGTERM/ctrl-c drains in-flight requests then exits.**
  - *Claim:* `axum::serve(...).with_graceful_shutdown(shutdown_signal())` stops accepting and
    drains on a signal.
  - *Evidence to collect:* read `crates/server/src/main.rs:33-38`; confirm `with_graceful_shutdown`
    is wired to a `shutdown_signal()` awaiting SIGTERM (`tokio::signal::unix`) and ctrl-c.
  - *Checks:* resolve `SignalKind::terminate()` and `tokio::signal::ctrl_c` — confirm both are
    awaited (whichever fires first), not just ctrl-c.
  - *Status:* ☐ unverified

- **O2 — Drain bounded by a 10 s named constant; stragglers aborted on expiry.**
  - *Claim:* the post-signal drain is wrapped in `tokio::time::timeout` with a named constant
    (e.g. `SHUTDOWN_DRAIN_DEADLINE_SECS = 10`); on expiry the process exits rather than hangs.
  - *Evidence to collect:* read the shutdown path; confirm the named constant and the `timeout`
    wrapper. Run the drain-deadline unit test (injected short deadline against a non-completing
    drain) — expect it returns within the deadline.
  - *Status:* ☐ unverified

- **O3 — Negative-space: deadline fires and the process exits even if a request never completes.**
  - *Claim:* a non-completing in-flight request does not cause an indefinite hang — the deadline
    forces exit.
  - *Evidence to collect:* run the unit test where the drain future never resolves and confirm the
    `timeout` elapses and the code proceeds to exit (logging the straggler abort).
  - *Status:* ☐ unverified

- **O4 — Two meaningful assertions on the shutdown path; deadline is a named constant.**
  - *Claim:* the shutdown path carries two or more non-trivial assertions and the deadline is a
    named constant, not a literal.
  - *Evidence to collect:* read the shutdown path; confirm the assertions (e.g. non-zero deadline,
    non-empty bind address) and the named constant.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: SIGTERM drains in-flight then exits within 10 s.**
  - *Claim:* a reviewer starting the server, holding a slow request, and sending SIGTERM observes
    the in-flight request drain and the process exit within the deadline.
  - *Evidence to collect:* start the binary, open a slow request, `kill -TERM <pid>`, and observe
    the in-flight request finish and the process exit within 10 s (unit test covers the deadline
    helper; this drive confirms the signal wiring).
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/main.rs` `main`: trace the non-Lambda branch and confirm the server still
  binds `host:port` and serves normally when no signal is pending (startup path unchanged) : ☐
  (PRESERVED / REGRESSION)

## Residue

- The full SIGTERM drive is a manual reviewable step; the unit test covers only the drain-deadline
  helper (see plan.md Open questions on whether to add a binary-spawning integration test). Not an
  obligation beyond O5.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
