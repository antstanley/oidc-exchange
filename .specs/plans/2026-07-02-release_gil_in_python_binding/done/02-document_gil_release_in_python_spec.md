# Task 02 — document GIL release in python spec

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-document_gil_release_in_python_spec-certificate.md](02-document_gil_release_in_python_spec-certificate.md)

**Implements:** [.specs/bindings/specs/03-python.md](../../../bindings/specs/03-python.md) §Implementation (the "Rust (`src/lib.rs`)" bullet)
**Depends on:** 01
**Produces:** the canonical `03-python.md` Implementation bullet describes the GIL release delivered by task 01, so the page and the code agree
**Pointers:** `.specs/bindings/specs/03-python.md:32-34` (the Implementation "Rust (`src/lib.rs`)" bullet to replace) and `:3` (the `**Date:**` field to bump); `.specs/changes/merged/2026-07-01-release_gil_in_python_binding.md:39-46` (the Proposed-changes wording to apply)

## Steps

- [x] Replace the `03-python.md` Implementation "Rust (`src/lib.rs`)" bullet with the change spec's Proposed-changes wording: note that `handle_request_sync` extracts method/path/headers/body from the `PyDict`, **releases the GIL (`py.allow_threads`) around the blocking FFI `handle_request` call**, and re-acquires it to build the result dict, so other Python threads — including an asyncio event loop — keep running while a request is in flight; `shutdown` stays a no-op.
- [x] Bump the page's `**Date:**` header field.
- [x] Confirm the new wording matches the behaviour shipped in task 01's `lib.rs` (no claim the code does not deliver) and that no other section of `03-python.md` (API, Decisions) contradicts it.

## Definition of done

- [x] The `03-python.md` Implementation "Rust (`src/lib.rs`)" bullet states the GIL release via `py.allow_threads` and the re-acquisition to build the result dict, matching the change spec's Proposed-changes block.
- [x] The bullet matches the behaviour shipped in task 01 (`bindings/python/src/lib.rs`) with no drift, and no other section of the page contradicts it; the `**Date:**` field is bumped.
- [x] Meets the repo definition of done as it applies to a canonical prose page (the change description states the why; every internal link on the page still resolves — no code gates apply).
- [x] Reviewable: a reviewer diffs `03-python.md` and confirms the Implementation bullet reads the `py.allow_threads` behaviour and matches both the change spec's Proposed-changes block and the shipped `lib.rs`.
