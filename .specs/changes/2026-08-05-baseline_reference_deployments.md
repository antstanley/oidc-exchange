# Change: A security baseline for reference deployments, enforced in CI

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** examples/, Dockerfile, crates/adapters (deployment templates)

Turn `examples/` from a set of hand-authored files nothing checks into a product surface with a
written security baseline and a CI job that holds every shipped template to it. Fix the seven
defects the baseline would currently reject — a reference relying party that decodes a JWT
instead of verifying it, a Fargate ALB that publishes `/token` over plaintext HTTP with no way
to turn it off, an unencrypted and unauthenticated Valkey session store whose obvious fix is
silently a no-op, a Google client secret passed to a Lambda as a plaintext environment
variable, a Kubernetes Deployment with no `securityContext` on `:latest`, an `init.sql` whose
unique index the adapter's migration cannot repair, and a SQLite database holding refresh-token
digests and email addresses created world-readable — then add the gate, so the next edit cannot
quietly undo any of it.

---

## Motivation

The threat model records the assumption that makes this cluster load-bearing: *"Operators read
`docs/` and copy from `examples/`. This is the documented onboarding path, so insecure examples
are production issues."* Two of the seven defects are things an operator can deploy today and
be worse off for having followed the documentation exactly — the Fargate template publishes the
token endpoint on cleartext HTTP and then `outputs.tf` hands the operator that `http://` URL,
which `examples/ecs-fargate/README.md:73-75` and `docs/deployment/ecs-fargate.md:116-118` pipe
straight into `curl`; and the CDK template writes the Google client secret into a
CloudFormation template, its change sets, `cdk.out/`, and every `GetFunctionConfiguration`
response. Neither is a subtle defect. Both survive because `.github/workflows/ci.yml` builds
and tests the Rust workspace, the two bindings and the web apps, and touches nothing under
`examples/`.

Fixing the seven is necessary and insufficient. Four of them do not have the fix a competent
contributor would naturally write: `rediss://` alone is silently downgraded to cleartext
because `crates/adapters/Cargo.toml:31` builds `fred` without a TLS feature; the Lambda secret
needs a runtime fetch rather than an injection-by-reference block, because
`resolve_placeholders_in_str` reads `${GOOGLE_CLIENT_SECRET}` from the process environment
only; the SQLite mode fix has to pre-create the file because `sqlx-sqlite` exposes no mode
setter; and the Postgres index fix has to name the *wrong* index explicitly, because the
adapter's existing idempotent repair drops a differently named one. Each is a place where the
plausible patch produces something that looks fixed and is not — which, in files nobody
re-executes, is the worst available outcome. A written baseline plus a mechanical check is the
only thing that converts "someone got this right once" into "this stays right".

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → *new section* Reference deployments | Adds the baseline: the seven properties every shipped deployable satisfies, and the conformance gate that asserts them |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Docker (`Dockerfile`) | Runtime stage runs as a non-root user; example Dockerfiles pin the base image by digest |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Release pipeline, CI paragraph | Adds the `reference-baseline` job to the CI job list |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Assumptions / Decisions / Open questions | Five Decisions, two Assumptions, three Open questions |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) → PostgreSQL | The migration drops the example's `idx_users_external_id` by name before recreating the partial index |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) → SQLite | Owner-only file mode asserted by the adapter; the migration script runs in one transaction |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) → Session-only stores, Valkey | `rediss://` selects TLS; the workspace enables a `fred` TLS feature and a test asserts it |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) → Assumptions | Adds the single-writer-at-bootstrap assumption the SQLite transaction now enforces |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) → `[session_repository]` | The Valkey URL may carry an auth token and a `rediss://` scheme, so it is a secret-bearing field |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) → Assumptions | Shipped TOML under `config/`, `examples/` and `docs/` is checked in CI |
| [`.specs/architecture-principles.md`](../architecture-principles.md) → Monorepo layout | `examples/` is a gated product surface, not documentation |

No new canonical page. `docs/security/reference-baseline.md` is added as the operator-facing
rendering of the `Reference deployments` section; the spec section is the normative statement
and the docs page carries the revision number a template cites.

**Out of scope — owned by sibling change specs, cross-referenced not restated.**

- The KMS `algorithm` strings in `examples/aws-web/config/oidc-exchange.toml:17`
  (`ECDSA_SHA_256`) and `examples/ecs-fargate/config/fargate.toml:11` (`ECDSA_SHA256`), and the
  tightened Postgres migration probe, belong to
  [`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md).
  That spec's probe does **not** cover this change's `init.sql` defect: the probe fires only on
  SQLSTATE `42501`, and `examples/linux-postgres/docker-compose.yml:7` makes the app's role
  (`oidc_exchange`) the database owner, so the DDL is never denied and the probe never runs.
  The two changes are complementary — that one makes a DDL-denied deployment prove its schema,
  this one repairs a schema the adapter provisioned over.
- The `oidc-exchange config check` subcommand this change's gate *invokes* is specified in
  [`2026-08-05-resolve_config_placeholders_all_channels.md`](2026-08-05-resolve_config_placeholders_all_channels.md).
  This change consumes the checker; it does not build it.
- Release provenance, the installer, the dependency-advisory gate, and the pattern-based
  `.gitignore` rules (including `cdk.out/`) belong to
  [`2026-08-05-harden_release_supply_chain.md`](2026-08-05-harden_release_supply_chain.md).
- The `audit.adapter = "noop"` default and `examples/container/config/production.toml` belong to
  [`2026-08-05-audit_and_throttle_authentication_failures.md`](2026-08-05-audit_and_throttle_authentication_failures.md).
  The baseline's durable-audit property asserts the result; it does not specify the fix.

This change implements
[`hardening/proposals/reference-deployment-baseline.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/reference-deployment-baseline.md)
— its recommended path, Option 2 (a named baseline with a CI conformance gate) with Option 1's
per-template fixes as its first phase, plus the one carve-out from Option 3: a single manual
end-to-end run of the two KMS examples.

---

## Proposed changes

### `.specs/bindings/specs/05-distribution.md` → Reference deployments (Add)

Add as a new section after `Release pipeline`:

> ## Reference deployments
>
> `examples/` holds five deployable templates — `aws-web` (CDK), `ecs-fargate` (Terraform),
> `container` (compose and Kubernetes), `linux-postgres` and `linux-sqlite` (compose plus
> shell) — and the framework-integration samples under `examples/nodejs` and
> `examples/python`. `docs/deployment/` presents them as quick starts, so they are deployed
> verbatim and are treated as shipped artifacts, not illustrations.
>
> ### Baseline
>
> Every deployable artifact under `examples/`, and every image this repository builds,
> satisfies seven properties. The baseline is versioned; a template states the revision it
> conforms to. `docs/security/reference-baseline.md` is the operator-facing rendering.
>
> **B1 — Encrypted transport.** No configuration reachable through a template's documented
> variables produces a network path on which a credential, a token, or personal data travels
> in cleartext. TLS is the unconditional half of any listener pair and plaintext is the
> opt-in exception, gated behind a variable that names itself as testing-only and defaults
> to off. Where a plaintext listener exists at all it redirects; it never forwards. Outputs
> and documentation render the scheme actually provisioned.
>
> **B2 — Secrets by reference.** A secret reaches a runtime from a secret store, never by
> value through infrastructure configuration. No template, no synthesis output, and no
> committed example variables file contains a credential literal. Where the platform cannot
> inject by reference — AWS Lambda, whose environment the service reads through
> `resolve_placeholders_in_str` — the template fetches at runtime and exports into the
> process environment before the binary starts.
>
> **B3 — Least privilege at the runtime.** Every image this repository builds runs as a
> non-root user. Every Kubernetes workload satisfies the four conditions the `restricted`
> Pod Security Standard checks — `runAsNonRoot: true`, `allowPrivilegeEscalation: false`,
> `capabilities.drop: [ALL]`, `seccompProfile.type: RuntimeDefault` — and the pod holding
> signing-key material additionally sets `readOnlyRootFilesystem: true`, an explicit
> `runAsUser`, an `fsGroup` matching the mounted secret's `defaultMode`, and
> `automountServiceAccountToken: false`.
>
> **B4 — Immutable versions.** No shipped Dockerfile, compose file, manifest, or example
> variables file names a mutable tag. Base and deployed images are pinned by digest. Every
> Terraform root carries a committed `.terraform.lock.hcl`, so the provider version — and the
> resource defaults that come with it — is the reviewed one rather than whatever the
> operator's `init` resolved.
>
> **B5 — Loadable, durable configuration.** Every TOML this repository ships under `config/`,
> `examples/` and `docs/` loads under `oidc-exchange config check` and selects a durable
> audit sink. A configuration file that cannot produce a working service is a defect in the
> template, not in the operator's reading of it.
>
> **B6 — Restrictive modes on generated state.** A file created by this repository's own
> tooling or adapters that holds key material or authentication data is created owner-only.
> The adapter that owns the data asserts the mode itself; the ambient umask may make the
> result stricter and never looser.
>
> **B7 — A reference relying party verifies.** The demo relying parties demonstrate the
> verification a real relying party must perform: signature checked against the JWKS served
> at `GET /keys` under a pinned algorithm, then `exp`, `iss` and `aud`. Decoding is never
> substituted for verification in a file the documentation offers as a recipe.
>
> ### Conformance gate
>
> The `reference-baseline` job in `ci.yml` asserts the baseline on every push and pull
> request, over every shipped template rather than an enumerated list, so a sixth example
> inherits the check. It has four parts.
>
> **Infrastructure policy scan.** A ruleset derived from B1–B4 runs over the Terraform under
> `examples/*/infra/`, the CDK synthesis output, the Kubernetes manifests, and the compose
> files. The ruleset is written from the baseline rather than adopted from a vendor default,
> and the scanner is version-pinned. Every exception carries a written rationale in-tree; an
> exception without one fails the job.
>
> **Configuration check.** `oidc-exchange config check` runs over every TOML under `config/`,
> `examples/` and `docs/`, asserting B5.
>
> **Version pinning check.** A Terraform root without a committed `.terraform.lock.hcl`
> fails. A `:latest` or otherwise mutable image reference in any shipped Dockerfile, compose
> file, manifest, or variables example fails.
>
> **Cross-layer assertions.** A policy scan can assert that infrastructure *declares* a
> control; it cannot assert that the client honours it. Three properties therefore live as
> ordinary workspace tests in the `test` job: that the `fred` dependency has a TLS feature
> enabled and a `rediss://` URL yields a TLS connection; that a SQLite database and its
> `-wal`/`-shm` siblings are mode `0600` under umasks 022, 002 and 077; and that
> `examples/linux-postgres/init.sql` followed by the adapter's `MIGRATIONS` leaves exactly
> the partial unique index on `(external_id, provider)`. These are the checks that catch a
> control being silently removed from underneath a template that still looks correct.
>
> The gate lands in reporting mode, then becomes blocking once the templates conform.

### `.specs/bindings/specs/05-distribution.md` → Docker (`Dockerfile`) (Modify)

Replace the paragraph with:

> Multi-stage: `rust:1.96-slim` builder (`cargo build --release --bin oidc-exchange`, with
> `pkg-config`/`libssl-dev`) → `debian:bookworm-slim` runtime with `ca-certificates`/`curl`,
> exposing 8080 and running the binary. The runtime stage creates an unprivileged user and
> ends with a `USER` directive naming it, so the published image runs as non-root by default
> and a Kubernetes `runAsNonRoot: true` does not depend on the manifest also supplying a
> `runAsUser`. Both stages pin their base image by digest. The example Dockerfiles under
> `examples/` layer config onto the published image and pin it by digest likewise; none
> references a mutable tag.

### `.specs/bindings/specs/05-distribution.md` → Release pipeline, CI paragraph (Modify)

> **CI (`ci.yml`)** runs on push/PR: `lint` (`cargo fmt --check`, `cargo clippy -- -D
> warnings`), `test` (`cargo nextest run --workspace`, which carries the baseline's
> cross-layer assertions), `nodejs-test` (build the napi module, vitest, lint/fmt),
> `python-test` (maturin build, pytest via uv), `web-apps` (lint, format, typecheck), and
> `reference-baseline` (infrastructure policy scan, `config check` over every shipped TOML,
> and the version-pinning check — see `Reference deployments`).

### `.specs/bindings/specs/05-distribution.md` → Assumptions / Decisions / Open questions (Modify)

Add to **Assumptions**:

> - The templates under `examples/` are deployed verbatim by operators following
>   `docs/deployment/`. The baseline exists because of that assumption; if the project
>   ever relabels them as illustrative, the baseline's scope changes with the label.
> - CDK synthesis in CI needs only a Node toolchain and no AWS credentials; the gate reads
>   `cdk.out/` and never deploys.

Add to **Decisions**:

> - *Reference deployments are a product surface.* **`examples/` is gated in CI on the same
>   push/PR trigger as the workspace.** These files are wrong not because their authors were
>   careless but because nothing re-checks them; a gate is the only mechanism that keeps a
>   fix fixed.
> - *A ruleset derived from the baseline, not a vendor default.* **The policy scan encodes
>   B1–B4 and nothing else, and every exception needs a written rationale.** A gate that
>   blocks legitimate patterns trains contributors to add exceptions reflexively, at which
>   point the exception list becomes the real policy and nobody reads it.
> - *Cross-layer properties are workspace tests, not scanner rules.* **The `fred` TLS
>   feature, the SQLite file mode and the Postgres index shape are asserted by
>   `cargo nextest`.** Static infrastructure analysis cannot see a client silently degrading
>   a connection the infrastructure correctly declares.
> - *Reporting mode before blocking.* **The gate lands non-blocking, then flips once the
>   templates conform.** It fails on day one across most templates, so the per-template
>   fixes are a prerequisite rather than an alternative.
> - *Baseline is a floor.* **A template can conform and still be inappropriate for a
>   particular production context.** The properties are the minimum that makes a template
>   safe to copy, not an architecture review of the deployment it produces.

Add to **Open questions**:

> - Which policy scanner? The baseline is scanner-agnostic; the choice is a tooling
>   preference the maintainers may hold.
> - Should the compose-based examples (`linux-postgres`, `linux-sqlite`, `container`) also be
>   stood up and probed per PR? They are fast and cheap and cover three of the baseline's
>   properties end to end. The cloud templates are out of reach for a per-PR gate either way.
> - Have the AWS reference deployments ever been run end to end? The shipped KMS algorithm
>   strings suggest not. A single manual run is scoped as work package D below and the answer
>   changes how much the remaining AWS templates should be trusted.

### `.specs/service/specs/08-persistence.md` → PostgreSQL (Modify)

Extend the partial-index paragraph. The sentences describing the `DROP INDEX IF EXISTS
idx_users_external_id_provider` step are replaced by:

> Because `CREATE UNIQUE INDEX IF NOT EXISTS` cannot turn a pre-existing full index into a
> partial one, the inline DDL runs two explicit drops immediately before recreating the index
> with the `WHERE` predicate. The first, `DROP INDEX IF EXISTS idx_users_external_id`,
> removes a full unique index on `external_id` alone — the shape a database provisioned from
> an external script may carry. That index is strictly stronger than the invariant the domain
> requires: it collides the same `external_id` across two providers and permanently blocks
> re-registration of a soft-deleted identity, and because it is named differently from the
> adapter's own index, nothing else in the migration would ever touch it. The second,
> `DROP INDEX IF EXISTS idx_users_external_id_provider`, replaces the adapter's own index
> from a database that predates the partial form. Both are no-ops on a database the adapter
> provisioned itself, so the sequence is idempotent on every startup. A database whose live
> rows already violate the intended uniqueness fails the recreate loudly rather than
> silently proceeding without an index.

### `.specs/service/specs/08-persistence.md` → SQLite (Modify)

Extend the section with:

> `create_pool` asserts the database file's mode before opening it. SQLite creates a missing
> file at `SQLITE_DEFAULT_FILE_PERMISSIONS` (0644) masked by the ambient umask, and
> `sqlx-sqlite` exposes no mode setter, so the adapter creates the file itself at `0600`
> before connecting; an existing file from before this behaviour is tightened to `0600` on
> the next start. The `-wal` and `-shm` siblings inherit the database file's mode, so all
> three are owner-only. The check is Unix-only and a no-op elsewhere, and `:memory:`
> databases are skipped. This matters because the file holds refresh-token digests and user
> email addresses as recoverable bytes.
>
> The LMDB session store gets the same treatment, and needs its own statement because its
> on-disk shape differs: `[session_repository.lmdb] path` names a **directory** holding
> `data.mdb` and `lock.mdb`, so the adapter creates that directory at `0700` and both files at
> `0600`, tightening either on a later start if it finds them wider. A directory left at `0755`
> leaks the store's existence and size to every local account even when the files inside are
> owner-only, and `data.mdb` holds the same refresh-token digests the SQLite file does. This is
> B6 applied to the second store that generates its own state, not a restatement of the
> generic property.
>
> The migration script runs inside one explicit transaction rather than as a sequence of
> autocommitted statements, so the `DROP INDEX` and the `CREATE UNIQUE INDEX` that follows it
> are never separated by a window in which the uniqueness constraint is absent. SQLite's
> write lock is held for the whole script; a second process opening the same database file
> during bootstrap waits rather than writing into the gap. The same reasoning does not apply
> to Postgres, whose migration already runs as one implicit transaction holding an
> `ACCESS EXCLUSIVE` lock through sqlx's raw simple-query path.

### `.specs/service/specs/08-persistence.md` → Session-only stores, Valkey (Modify)

Extend the Valkey bullet with:

> The connection URL's scheme selects the transport: `rediss://` connects over TLS,
> `redis://` in cleartext. The URL may also carry an auth token as its password component,
> which the server presents on connect. TLS is only honoured because the workspace builds
> `fred` with a TLS feature enabled — `fred` gates its TLS configuration behind
> `enable-native-tls` / `enable-rustls` / `enable-rustls-ring`, and with none of them set it
> parses `rediss://` and discards the resulting TLS setting rather than rejecting the URL, so
> the connection would be cleartext while appearing encrypted. A workspace test asserts that
> a `rediss://` URL produces a TLS connection, so removing the feature fails the build rather
> than silently downgrading every deployment that configured one.

### `.specs/service/specs/08-persistence.md` → Assumptions (Modify)

Add:

> - Exactly one process runs the SQLite bootstrap against a given database file at a time.
>   The migration transaction makes a concurrent writer wait rather than observe a partially
>   applied schema, but it does not make two processes racing to create the same file a
>   supported topology.

### `.specs/service/specs/06-configuration.md` → `[session_repository]` (Modify)

Replace the section with:

> When present, overrides where sessions are stored: `adapter` (`valkey` | `lmdb`) with
> `[session_repository.valkey] { url, key_prefix? }` or `[session_repository.lmdb] { path,
> max_size_mb? }`. Absent → sessions live in the `[repository]` store.
>
> The Valkey `url` is a secret-bearing field: its scheme selects the transport (`rediss://`
> for TLS, `redis://` for cleartext) and its password component carries the store's auth
> token. It is supplied through the environment as a `${VAR}` placeholder resolved from a
> secret store, never written literally into a committed TOML file, and never carried in an
> infrastructure template's plaintext environment block.

### `.specs/service/specs/06-configuration.md` → Assumptions (Modify)

Add:

> - Every TOML this repository ships — under `config/`, `examples/` and `docs/` — loads under
>   `oidc-exchange config check` in CI. A shipped configuration that the service would refuse
>   at startup is a build failure, not a discovery an operator makes on first deploy.

### `.specs/architecture-principles.md` → Monorepo layout (Modify)

In the layout block, the `examples/` line reads:

> ```
> ├── examples/                  # reference deployments (gated) + framework integrations
> ```

and a paragraph is added after the block:

> `examples/` is a shipped surface, not documentation. `docs/deployment/` presents its
> templates as quick starts and operators deploy them verbatim, so they are held to the
> security baseline in
> [bindings/specs/05-distribution.md](bindings/specs/05-distribution.md) and checked by CI on
> the same trigger as the workspace.

---

## Type changes

None. This change touches infrastructure templates, Dockerfiles, a SQL script, a SvelteKit
loader, two adapter bootstrap functions, one Cargo dependency's feature set, and one CI job. No
domain entity, config field, or API shape changes. No `canonical-types.schema.json` fragment.

---

## Implementation notes

Four work packages. **A** is the per-template remediation and must land before **C**, since the
gate fails against the current templates. **B** is the baseline document and can be written in
parallel with A. **C** is the gate. **D** is independent of all three and answers a question
that may change the scope of the rest — run it early.

Within A, the items are independently shippable and are ordered by how deployable the defect is
today. A2 has a hard internal ordering that must not be inverted.

**A — Per-template fixes**

1. **Fargate listener chain, as one change.** `examples/ecs-fargate/infra/main.tf:541-550`
   creates `aws_lb_listener.http` unconditionally on port 80 with a `forward` action;
   `:552-565` creates the HTTPS listener under `count = var.certificate_arn != "" ? 1 : 0`.
   Plaintext is the unconditional case and TLS the optional one — invert it. Move the `count`
   onto the plaintext listener, change its action to a 301 redirect to 443, and add a second,
   mutually exclusive `aws_lb_listener` gated on `var.certificate_arn == "" &&
   var.allow_insecure_http` for the forwarding case. Add `variable "allow_insecure_http"`
   (bool, default `false`) to `variables.tf`, naming itself testing-only; the existing
   `certificate_arn` description at `variables.tf:35-39` ("Leave empty for HTTP-only
   (testing)") is what currently reads as an endorsement and must change with it. Both
   listeners bind port 80 under complementary `count` conditions, so exactly one exists in any
   plan. Then: `outputs.tf:1-4` renders `alb_url` as `"http://${dns_name}"` unconditionally —
   make the scheme follow what was provisioned. Then `examples/ecs-fargate/README.md:73-75`
   and `docs/deployment/ecs-fargate.md:116-118`, which pipe that output into `curl`. Finally
   `main.tf:677-679`: the ECS service's `depends_on = [aws_lb_listener.http]` names a resource
   that becomes conditional, so it must move to the target group or become a list over both
   listeners — miss this and `terraform apply` fails on an index into an empty list.

2. **CDK client secret.** `examples/aws-web/infra/lib/stack.ts:63` passes
   `GOOGLE_CLIENT_SECRET: props.googleClientSecret` into the Lambda's `environment` block, and
   `bin/app.ts:7-12` sources it from CDK context with a placeholder default. Two steps, both
   required:
   - Template side: replace the value with a `GOOGLE_CLIENT_SECRET_ARN`, construct
     `secretsmanager.Secret.fromSecretCompleteArn`, and `grantRead(authFunction)`. Have
     `bin/app.ts` read `googleClientSecretArn` from context and fail fast when absent rather
     than defaulting to a placeholder.
   - Runtime side, and this is the part the obvious fix misses: an ECS-style `secrets` block
     does not exist for Lambda, and `resolve_placeholders_in_str`
     (`crates/server/src/bootstrap.rs:164-197`) resolves `${GOOGLE_CLIENT_SECRET}` through
     `std::env::var` at `:186` — the process environment only. Something must put the value
     there before the binary starts. `provided.al2023` honours `AWS_LAMBDA_EXEC_WRAPPER`, and
     this stack already uses that mechanism for the demo-app function
     (`stack.ts:89`), so a short wrapper that fetches the secret, exports it, and `exec`s
     `bootstrap` satisfies the existing config contract with no change to the service. Pair it
     with the AWS Parameters and Secrets Lambda Extension if the per-cold-start call matters.
   The alternative — teaching config loading to resolve a secret-store reference directly — is
   larger, benefits every deployment shape, and is recorded as an Open question rather than
   proposed here.

3. **Valkey, in this order.** The order is the whole point; reversing it ships a template that
   looks encrypted and is not.
   1. Add a TLS feature to `fred` at `crates/adapters/Cargo.toml:31`, which today reads
      `fred = { version = "10", features = ["serde-json"] }`. `enable-rustls-ring` is the
      choice consistent with the workspace's existing `sqlx` `tls-rustls`. `fred` gates its
      TLS configuration behind `enable-native-tls` / `enable-rustls` / `enable-rustls-ring`
      and with none set discards the scheme parsed by `Config::from_url`
      (`crates/adapters/src/valkey/mod.rs:31`).
   2. Add the regression test that a `rediss://` URL produces a TLS connection. It fails
      before step 1 and passes after; that is the assertion that keeps the feature from being
      dropped later.
   3. Only then `examples/ecs-fargate/infra/main.tf:268-282`: add
      `transit_encryption_enabled = true`, `at_rest_encryption_enabled = true`, and an
      `auth_token` from a generated `random_password` stored in Secrets Manager. Set the
      at-rest flag explicitly even though the provider default may already cover it —
      uncertainty about a default is the argument for pinning it.
   4. `main.tf:613` builds `VALKEY_URL` as `redis://…` in the task definition's plaintext
      `environment` block. With an auth token the URL is a credential: it moves to the
      `secrets` block alongside the Google secret (`:619-628`), and its scheme becomes
      `rediss://`. `examples/linux-postgres/config/postgres-valkey.toml:25` keeps `redis://`
      for the local compose topology, which is loopback and out of B1's scope; note that
      explicitly rather than leaving it to look like an oversight.
   5. Commit `.terraform.lock.hcl` under `examples/ecs-fargate/infra/` — there is none today,
      so `~> 5.0` resolves a different AWS provider, and different resource defaults, per
      operator.

4. **Kubernetes.** `examples/container/k8s/deployment.yml:15-58` declares no `securityContext`
   at pod or container level and pulls `your-registry/oidc-exchange:latest` at `:18`. Add the
   pod-level `securityContext` (`runAsNonRoot`, explicit `runAsUser`/`runAsGroup`/`fsGroup`
   `65534`, `seccompProfile: RuntimeDefault`), the container-level context
   (`allowPrivilegeEscalation: false`, `readOnlyRootFilesystem: true`,
   `capabilities.drop: [ALL]`), and `automountServiceAccountToken: false`. Three traps: with
   the current image's default user being root, `runAsNonRoot: true` without an explicit
   `runAsUser` produces `CreateContainerConfigError` rather than a chosen uid — which is why
   the `Dockerfile` `USER` change below is the more durable half; `readOnlyRootFilesystem`
   needs an `emptyDir` at `/tmp`; and tightening the signing-key Secret's `defaultMode` to
   `0440` only works with a matching pod `fsGroup`. Pin the image by digest. Then the root
   `Dockerfile:7-12`, whose runtime stage has no `USER` directive at all, so every image this
   repository publishes runs as root. Then `docs/deployment/container.md:138`, which
   reproduces the manifest inline with `:latest`, and `examples/container/Dockerfile:1` /
   `examples/ecs-fargate/Dockerfile:1`, which both `FROM
   ghcr.io/antstanley/oidc-exchange:latest`.

5. **Postgres schema drift, plus the adapter self-repair.**
   `examples/linux-postgres/init.sql:14` creates `idx_users_external_id`, unique on
   `external_id` alone, where the adapter's `MIGRATIONS`
   (`crates/adapters/src/postgres/mod.rs:39-40`) creates a partial unique index on
   `(external_id, provider) WHERE status != 'deleted'`. The example's index is strictly
   stronger: it collides one `external_id` across two providers and permanently blocks
   re-registration of a soft-deleted identity. The adapter's existing repair drops
   `idx_users_external_id_provider`, a different name, so the drift is permanent. Two edits:
   - `init.sql:14` → the partial index on `(external_id, provider)`, and add
     `version BIGINT NOT NULL DEFAULT 1` to the `users` DDL at `:1-12` so the script matches
     the adapter without relying on repair. (The missing `version` column is *not* the
     unrepairable half: the migration's `ALTER TABLE users ADD COLUMN IF NOT EXISTS version`
     at `postgres/mod.rs:32` heals it, and the example's role is the database owner so that
     DDL succeeds. Fix it because a template should be correct on its own terms, not because
     the drift survives.)
   - `crates/adapters/src/postgres/mod.rs`, immediately before the existing drop at `:39`: add
     `DROP INDEX IF EXISTS idx_users_external_id;` so a database already provisioned from the
     old `init.sql` heals on its next start. Mirror the same line into
     `crates/adapters/src/sqlite/mod.rs:37`; SQLite ships no comparable `init.sql`, but the
     two migration blocks are maintained as mirrors and should not diverge.

   The stronger option is to delete `init.sql` and its compose mount
   (`examples/linux-postgres/docker-compose.yml:12`) outright, letting the adapter's
   migrations own the schema — which is what `docs/deployment/linux-postgres.md:57` already
   tells operators happens. That removes the class of defect rather than this instance. It is
   recorded as an Open question because it depends on whether any deployment mode genuinely
   needs DDL before the app connects. The adapter self-repair is required either way, for
   databases already in the drifted state.

6. **SQLite file mode, and the bootstrap transaction.**
   `crates/adapters/src/sqlite/mod.rs:65-78` calls `create_if_missing(true)` and lets SQLite
   create the file at 0644 under a default umask; the `-wal` and `-shm` siblings inherit that
   mode. Add a `#[cfg(unix)]` helper called at the top of `create_pool`, before
   `connect_with`, that opens the path with `create_new(true).mode(0o600)` and, on
   `AlreadyExists`, tightens the existing file with `set_permissions` — a database from before
   this fix keeps its loose mode otherwise. SQLite treats a zero-length file as a valid empty
   database, so pre-creating is safe. Skip `:memory:`. Map failures into `Error::StoreError`
   like the neighbouring branches.

   While in this function, close the bootstrap window that `coverage.json` carries as
   `deferred_sqlite_index_recreate_window`: `:81-86` executes the multi-statement `MIGRATIONS`
   through `sqlx::query`, which on SQLite autocommits each statement, so the `DROP INDEX` at
   `:37` and the `CREATE UNIQUE INDEX` at `:38` are separate transactions with no uniqueness
   constraint in force between them. Run the script inside one `sqlx` transaction (a
   `BEGIN IMMEDIATE`, so the write lock is taken up front rather than upgraded mid-script).
   The Postgres half of that deferred item was refuted — `sqlx::raw_sql` already runs the
   script in one implicit transaction holding `ACCESS EXCLUSIVE` — so no Postgres change
   follows from it. This is folded in here rather than left deferred because it is a
   three-line change in a function this work package already opens, and because the local
   compose probes the gate adds are exactly the context in which a second concurrent writer
   during bootstrap stops being hypothetical.

   `examples/linux-sqlite/setup.sh:9-16` generates the signing key under the ambient umask;
   set a restrictive umask in the script and `chmod 600` the generated key. The documented
   backup command does not propagate the source mode — fix it in
   `docs/deployment/linux-sqlite.md` alongside.

7. **The demo relying party.**
   `examples/aws-web/demo-app/src/routes/authenticated/+page.server.ts:11-17` splits the
   `access_token` cookie, checks it has three segments, base64url-decodes the middle one, and
   returns the result as the authenticated user at `:19-32`. It verifies no signature and
   checks no `exp`.

   **Verify. Do not relabel.** The de-escalating path — keep the decode and mark the file
   display-only — is available and this change rejects it, for three reasons. First, the file
   has the exact shape of an auth gate: it redirects to `/` when the cookie is missing
   (`:7-9`) and when parsing throws (`:33-35`), so a reader parses it as an authorization
   check regardless of a comment saying otherwise, and a comment is the first thing lost to
   copy-paste. Second, `GET /keys` exists so that relying parties can verify; a reference
   integration that skips verification leaves the service's central product claim without a
   single demonstrated consumer anywhere in the repository. Third, the `httpOnly` flag set at
   `api/login/+server.ts:35-41` is not a substitute for verification — anything able to write
   a cookie for the origin (a sibling application on a shared parent domain, cookie-tossing
   from a subdomain) supplies a token this loader accepts, and with no `exp` check it accepts
   an expired one indefinitely.

   The fix: fetch the JWKS from `${AUTH_ENDPOINT}/keys`, reject any header `alg` other than
   the expected one (which also rejects `alg: "none"`), verify the signature with
   `webcrypto.subtle` against the JWK whose `kid` matches, then check `exp`, `iss` and `aud`
   against `ISSUER_URL` / `AUDIENCE_URL` — both of which `stack.ts:160-166` already sets on
   the demo-app function. Cache the JWKS and re-fetch on an unknown `kid`, mirroring the
   service's own JWKS handling. A maintained library is the better production answer and
   should be what the README points at; the dependency-free form is worth shipping so the
   sample stays copy-pasteable. Ship it as a reusable module the framework samples under
   `examples/nodejs/*` can import, not only as an inline loader.

**B — The baseline document.** Write `docs/security/reference-baseline.md` as the
operator-facing rendering of the `Reference deployments` section: one property per line, each
traceable to a finding or to a threat-model invariant, with a revision number a template can
cite. Short. It is the source the policy ruleset is derived from, so a rule with no line in it
is a rule that should not exist.

**C — The gate.** Add the `reference-baseline` job to `.github/workflows/ci.yml` with the four
parts described in the spec block above, plus a `cdk synth` step (Node toolchain, no AWS
credentials) so the CDK output is scannable and greppable for secret literals. Move the three
cross-layer assertions into the workspace test suite so they run in `test`. Land the job
non-blocking; expect failures across most templates until A completes; triage, then flip to
blocking with the exception mechanism in place. Derive the template list from the filesystem,
not from a hardcoded array, so a sixth example is covered on the day it lands.

**D — The manual KMS run.** Deploy `examples/aws-web` and `examples/ecs-fargate` once, by hand,
with the shipped `key_manager.kms.algorithm` values, and record whether `/token` and `/keys`
fail on first request. `coverage.json` carries this as an open question and the shipped strings
(`ECDSA_SHA_256`, `ECDSA_SHA256` — AWS `SigningAlgorithmSpec` names, not JWS `alg` names)
predict they do. An afternoon's work. If they cannot have been run, that reframes how much
confidence every other AWS template deserves, and reducing their scope becomes a reasonable
alternative to maintaining them. The algorithm fix itself belongs to
[`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md);
what this package produces is the recorded answer, not the patch.

**Regression tests.**

- `terraform plan` with and without a certificate ARN, and with `allow_insecure_http` both
  ways: assert no configuration produces a listener that forwards plaintext except the
  explicitly opted-in one, and that `alb_url` renders the scheme actually provisioned in every
  case.
- A `rediss://` URL produces a TLS connection — failing before the `fred` feature is added,
  passing after. Plus a Terraform assertion that transit encryption and an auth token are
  declared.
- `cdk synth` output contains no secret literal, and the function resolves the secret at
  runtime.
- The Kubernetes manifest satisfies the four `restricted`-profile conditions and names a
  digest.
- `init.sql` into a scratch schema, then `MIGRATIONS`, then the existing
  `delete_user_frees_external_id_for_recreation` body — the test that would have caught the
  drift, and a small extension of one that already exists. Plus: the same `external_id` under
  two providers produces two accounts.
- The LMDB environment under umasks 022, 002 and 077: the `path` directory is `0700` and
  `data.mdb`/`lock.mdb` are `0600`; a pre-existing `0755` directory or `0644` file is tightened
  on open.
- `create_pool` under umasks 022, 002 and 077: database, `-wal` and `-shm` all `0600`. Plus a
  pre-existing 0644 database tightened on open.
- The demo relying party rejects a token with a valid payload and a garbage signature, an
  `alg: "none"` token, an expired token, and a token whose `iss` or `aud` does not match.

---

## Compatibility

| Deployment | What breaks | Migration |
|---|---|---|
| A Fargate stack applied without a certificate ARN | Port 80 stops serving; the template now requires either a certificate or an explicit `allow_insecure_http = true` | Supply a certificate, or set the opt-in variable and understand what it means |
| A Fargate stack whose Valkey is already provisioned | Enabling transit encryption and an auth token on an existing replication group is a replacement, not an in-place update | Plan the change; sessions in the old cluster are lost, and the session store is a cache of revocable state, not a system of record |
| A CDK stack deployed from the old template | The Google client secret is already in CloudFormation change sets and `GetFunctionConfiguration` history | Rotate the secret at Google after moving to the ARN. The template fix does not un-disclose the old value |
| A Postgres database provisioned from the old `init.sql` | Nothing, on the next start: the adapter drops `idx_users_external_id` and creates the partial index | If duplicate live `(external_id, provider)` rows already exist, the recreate fails loudly and they must be reconciled first |
| An existing SQLite database at 0644 | Nothing; the mode is tightened on next open | None. Verify no other process depended on group or world read |
| A deployment pinning `ghcr.io/antstanley/oidc-exchange:latest` | The published image starts running as non-root; a bind-mounted path owned by root becomes unwritable | Adjust volume ownership, or set an explicit `runAsUser` matching the previous behaviour and record why |

The demo relying party's change is not a compatibility concern — it is a sample application —
but it is worth stating that a deployment which copied the old loader is not fixed by this
change. Its own copy still trusts an unverified token.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to
   the merge date.
2. Insert the `Reference deployments` section into
   [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) after
   `Release pipeline`.
3. No schema change to fold in.
4. Work packages ship independently: **A1** (Fargate listener chain), **A2** (CDK secret),
   **A3** (Valkey, internally ordered), **A4** (Kubernetes + `Dockerfile`), **A5** (Postgres
   schema), **A6** (SQLite), **A7** (demo relying party), **B**, **C** and **D** are each
   separately mergeable to the codebase. Merge *this spec* only when A, B and C have all
   landed and the gate is blocking; until then leave it `Accepted` and record which packages
   have shipped in its header. D's outcome is recorded in the header regardless of what it
   finds.
5. If A5's Open question resolves toward deleting `init.sql`, amend the `Proposed changes`
   block for `08-persistence.md` before merging — the adapter self-repair line stays either
   way.
6. Flip `**Status:**` to `Merged`, stamp `**Merged:** YYYY-MM-DD`, move to
   `.specs/changes/merged/`.
7. Update [`.specs/README.md`](../README.md)'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- `oidc-exchange config check` exists by the time work package C lands. It is specified in
  [`2026-08-05-resolve_config_placeholders_all_channels.md`](2026-08-05-resolve_config_placeholders_all_channels.md)
  and this change consumes it. If that spec has not shipped, C lands without its configuration
  half and gains it later; the other three parts do not depend on it.
- `fred`'s TLS feature can be enabled for every build without an unacceptable cost. It adds a
  TLS stack to builds that do not use Valkey, which is the price of making the guarantee
  unconditional.
- The examples' AWS resources are provisioned by the templates themselves, so changing a
  Terraform resource is a change to what an operator gets on their next apply, not a change to
  something already running under someone else's control.
- Nothing currently committed contains real key material. The mode and ignore work is a
  guardrail against a future commit.

### Decisions

- *Option 2 with Option 1 as its first phase.* **The per-template fixes land first and the
  gate second, because the gate fails against the current templates.** They are not
  alternatives: Option 1 removes today's hazards and Option 2 stops them returning. The
  proposal's own reasoning is that four of the eight fixes are not the fix a competent
  contributor would naturally write, and nothing in the repository would notice any of them
  being got wrong.
- *The demo relying party verifies; it is not relabelled.* **`+page.server.ts` fetches the
  JWKS and verifies, and no decode-only variant ships alongside it.** The de-escalating path
  the finding allows is available and rejected: the file has the shape of an auth gate, a
  comment does not survive copy-paste, and `GET /keys` otherwise has no demonstrated consumer
  in the repository at all.
- *The `fred` TLS feature ships before the Valkey template.* **Feature, then regression test,
  then `rediss://`.** The reverse order produces a template that reports encryption and
  connects in cleartext, which is worse than today's honest cleartext because nobody
  re-checks it.
- *The Lambda secret is fetched at runtime, not injected by reference.* **An exec wrapper
  exports the value into the process environment before `bootstrap` runs.** Lambda has no
  ECS-style `secrets` block, and `resolve_placeholders_in_str` reads only
  `std::env::var` — an ARN in the environment resolves to nothing without something to
  dereference it.
- *The adapter self-repairs the example's index rather than trusting the corrected
  `init.sql`.* **`DROP INDEX IF EXISTS idx_users_external_id` joins the migration.** Fixing
  the script helps only new deployments; every database already provisioned from it carries
  permanent drift that nothing else in the migration would ever touch.
- *The SQLite bootstrap window is closed here rather than left deferred.* **The migration
  script runs in one transaction.** `coverage.json` defers it for want of evidence of a
  second concurrent writer, which is a fair reason not to prioritise it and not a reason to
  leave it open while already editing the same function — and the local compose probes this
  change contemplates are precisely where a second writer appears.
- *Cross-layer checks are workspace tests, not scanner rules.* **Three properties move into
  `cargo nextest`.** A Terraform scan can assert that ElastiCache declares transit encryption;
  it cannot assert that the client honours `rediss://`, which is the exact failure this
  cluster contains.

### Open questions

- Should `examples/linux-postgres/init.sql` be deleted outright rather than corrected? Deleting
  it and its compose mount removes the class of defect and matches what
  `docs/deployment/linux-postgres.md:57` already claims happens. Keeping it is defensible only
  if some deployment mode needs DDL before the app connects — and if it is kept, it should be
  generated from `MIGRATIONS` rather than maintained by hand. The adapter self-repair is
  required under either answer.
- Should config loading learn to resolve a secret-store reference directly, rather than the
  Lambda template shipping an exec wrapper? It is the more invasive change and it benefits
  every deployment shape, removing the environment variable as a carrier entirely. The wrapper
  is proposed because it needs no service change; the broader design is a better answer if
  someone is willing to own it.
- Should the compose-based examples be stood up and probed per PR? They are fast, need no
  credentials, and would cover B1, B5 and B6 end to end on three templates. Per-PR is stronger
  and slower.
- Which policy scanner, and does its ruleset stay small? The baseline is scanner-agnostic. The
  risk being managed is a false-positive rate that trains contributors to add exceptions
  reflexively, at which point the exception list is the real policy.
- Have the AWS reference deployments ever been run end to end? Work package D answers it. If
  they have not, reducing their scope to something the project can maintain is a legitimate
  response and would change what B and C need to cover.
- Do the framework-integration samples under `examples/nodejs` and `examples/python` need
  their own baseline properties? They are relying-party samples rather than deployables, so
  B1–B6 mostly do not apply, but the threat model records `DEBUG = True`, `ALLOWED_HOSTS =
  ["*"]`, a hardcoded `SECRET_KEY` and `app.run(debug=True)` in the Django and Flask samples.
  B7 covers their verification behaviour; whether the gate should also hold them to
  framework-level hardening is undecided.
