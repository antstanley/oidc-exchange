# Task 05 — Container runtime hardening

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.4 Kubernetes](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Proposed changes — Docker](../../../changes/2026-08-05-baseline_reference_deployments.md#proposed-changes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests), [bindings distribution §Docker](../../../bindings/specs/05-distribution.md#docker-dockerfile)
**Depends on:** —
**Produces:** a digest-pinned container reference deployment whose image and Kubernetes workload satisfy the specified restricted runtime controls.
**Pointers:** `Dockerfile:1-12`; `examples/container/Dockerfile:1`; `examples/ecs-fargate/Dockerfile:1`; `examples/container/k8s/deployment.yml:15-58`; `examples/container/docker-compose.yml`; `docs/deployment/container.md:138`

## Steps

- [ ] Pin root and example image bases/deployments by digest and remove mutable image references from the scoped Docker, compose, manifest, and documentation surfaces.
- [ ] Create and select a non-root runtime user in the root image while retaining required runtime files and health tooling.
- [ ] Add pod and container security contexts, disable service-account token mounting, configure secret ownership/mode, and mount writable `emptyDir` storage for `/tmp` under a read-only root filesystem.
- [ ] Update compose and deployment documentation to use the immutable references and explain required volume ownership behavior.
- [ ] Add manifest/image assertions for the four restricted Pod Security conditions, signing-key controls, non-root image default, and digest pinning.

## Definition of done

- [ ] The published runtime image has a non-root `USER`; an explicit `runAsNonRoot` workload can start without relying on an unspecified UID.
- [ ] The Kubernetes deployment has all required restricted controls, writable `/tmp`, and secret mode/group settings that allow its non-root process to read signing keys.
- [ ] Every scoped shipped image reference is digest-pinned; no mutable tag remains in the covered template/documentation files.
- [ ] Policy/assertion coverage rejects a missing restricted control and a mutable image reference.
- [ ] Meets the repo definition of done (applicable Docker/Kubernetes/static checks and language checks; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: inspect the rendered manifest and image configuration, then run the policy assertions against compliant and intentionally weakened fixtures.
