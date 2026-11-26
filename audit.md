# Tessel Audit Guide

How to audit Tessel (Pure Rust: `tessel-core`, `tessel-io`, `tessel-ai`, `tessel-render`, `tessel-app`)
using the project audit skills plus the visual/delivery skills. This file is the router:
it tells you **which skill to load, what to inspect, and what a passing audit looks like**.

## 0. Universal rules (apply to every audit)

1. **Evidence first.** Classify every material claim as `VERIFIED_IN_SCOPE`,
   `CONTRADICTED_IN_SCOPE`, `PARTIAL`, `NOT_FOUND_IN_SCOPE`, or `UNKNOWN`.
   Never upgrade `UNKNOWN` to verified because code "looks plausible".
   Keep verified behavior, proposed fixes, test observations, and inference
   visibly separate.
2. **Trace before edit, sync after change.** Before touching a file, inspect
   its blast radius (file lineage and caller hierarchy) per the project
   graph workflow in `AGENTS.md`; after all edits and test runs, sync the
   source graph. Never use blind `grep`/`find` for architecture discovery;
   use them only for literal strings (exact error text, UI labels, config
   values).
3. **Tessel Rust reliability rules (non-negotiable).**
   - Zero `unwrap()` / `expect()` in production code; return `Result<T, E>`
     via `thiserror` (`unwrap()` only inside `#[cfg(test)]`).
   - `tessel-core` stays headless: no `wgpu`, `winit`, `egui`, `tokio`, `reqwest`.
   - All scene mutations go through the `Command` bus (deterministic + undoable).
   - Single Responsibility + Anchoring Object Type (`nano.rs` -> sub-unit ->
     unit anchor -> `ops.rs` pure algorithms -> `commands.rs` mutations).
   - Right-handed `+Y` up, `+Z` front, CCW winding, `1 voxel = 0.1 m`
     (see `.knowledges/docs/conventions.md`).
4. **Quality gates.** `cargo fmt --check`, `cargo clippy --workspace
   --all-targets -- -D warnings`, and `cargo test --workspace` must all pass.
5. **One skill owns each finding.** Fix at the authoritative boundary the
   owning skill names; do not mix unrelated refactors into an audit fix.

## 1. How to run an audit

1. **Scope it.** Name the area, crates, entry points, and what is explicitly
   out of scope.
2. **Map the surface.** List handlers, commands, schemas, configs, assets,
   probes, or scenes in scope. Use the project graph workflow for the
   blast radius.
3. **Build evidence and assumption ledgers.** One table for claims
   (`Claim | Evidence | Status | Validation`), one for assumptions and
   uninspected layers. Every skill has an `evidence-and-assumption-control`
   reference: follow it.
4. **Run the owning skill(s).** Use the router table in section 2 to pick
   skills. Read only the references each skill names for the task.
5. **Validate with the repo's own tests.** Reproduce against real code, add a
   committed test where the touched area has tests, and run the gates in
   rule 4. Throwaway probes stay in `/tmp`, never in the deliverable.
6. **Report.** Use the template in section 4. Mark non-applicable sections
   `OUT_OF_SCOPE` with a reason; list `UNKNOWN` items and remaining risks.

## 2. Skill router: symptom -> skill

| Symptom / question | Owner skill |
|---|---|
| API shape, versioning, schema compat, serialization, webhooks | `api-contracts-and-integrations` |
| Auth, RBAC/IDOR, tenant isolation, injection, secrets, CVEs | `application-security-and-isolation` |
| Coupling, cycles, dead code, duplication, startup config | `architecture-and-config-hygiene` |
| Domain rules, boundaries, state machines, math/rounding, input validation | `business-logic-and-input-validation` |
| Schema constraints, transactions, races, idempotency, retention/lifecycle | `data-integrity-and-lifecycle` |
| Async tasks, cancellation, leaks, shutdown, backpressure, durability | `evidence-led-concurrency-and-delivery` |
| Logs, metrics, tracing, alerts, health probes, audit trails | `observability-and-audit-trail` |
| Latency, memory, complexity, I/O, caching, benchmarks | `performance-and-resource-efficiency` |
| Error paths, retries/timeouts, degradation, isolation, recovery | `resilience-and-error-handling` |
| Blender model/scene/animation/render, `.blend` work | `blender` |
| Local browser app handoff (start command + URL) | `browser-app-delivery` |
| Playable browser game (2D/2.5D/3D) | `browser-game` |
| Polished/art-directed 3D look, reference matching | `game-3d-visuals` |
| Single-file 3D web page, viewer, WebGL/WebGPU/WGSL | `web3d` |
| Which visual skill owns this deliverable | `visual-coding` (router, read first) |
| Ambiguous prompt, unsettled product decision before building | `prompt-confidence-gate` |

## 3. How to audit with each skill

### 3.1 `api-contracts-and-integrations`

Use when reviewing MCP schemas, export formats (`.glb`, `.obj`, `.tessel`),
or any wire/serialization boundary.
Inspect: specs, handlers, DTOs, serializers, webhook endpoints/dispatchers.
Must check: spec-to-handler parity, consistent error envelopes, deterministic
pagination/ordering, additive backward/forward compatibility (no breaking
renames, type changes, or enum renumbering under an existing version),
`int64` precision, `null` vs empty collections, UTC timestamps, unknown
field/enum handling, inbound webhook auth over the raw body (HMAC/signature/mTLS),
enqueue-before-work, idempotent dedupe keys, out-of-order protection.
Tessel mapping: export writers in `tessel-io` must be byte-for-byte
deterministic (sorted keys, e.g. `BTreeMap`); coordinate/export conventions
follow `.knowledges/docs/conventions.md` and `file-format-spec.md`.

### 3.2 `application-security-and-isolation`

Use when reviewing trust boundaries: file dialogs, import/export parsers,
AI/MCP inputs, keyring secrets, multi-profile data.
Inspect: entry points, auth middleware, principal/tenant extraction, sinks
(SQL, shell, templates, filesystem, outbound HTTP), secrets, logs, responses,
`Cargo.toml`/`Cargo.lock`.
Must check: default-deny auth, resource-level authorization (never trust
client-supplied IDs), tenant scoping of every query/cache/file/queue payload,
parameterization and allowlists, canonical path containment, SSRF allowlists,
no hardcoded fallback secrets, no credentials/PII in logs, no stack traces or
SQL in external errors, and a real scanner run before claiming "no known CVEs".
Treat UI checks and happy-path tests as hypotheses, not proof.

### 3.3 `architecture-and-config-hygiene`

Use when reviewing crate/module boundaries, coupling, or startup config.
Inspect: `crates/*` layout, `mod.rs` exports, import direction, DI/startup
wiring, all config sources (env, files, flags).
Must check: layering (core never depends on render/app/net), no import/DI
cycles (break by inverting dependencies, not by merging packages), no leaky
boundary types, dead code verified across reflection/route tables/exports
(never from text search alone), no drifting duplicated logic, fail-fast
startup validation with no silent fallbacks (`TIMEOUT=0` or dev secrets must
not become defaults). Preserve behavior; verify with build + tests.

### 3.4 `business-logic-and-input-validation`

Use when reviewing voxel/pixel/mesh/rig rules, converters, and validators.
Inspect: authoritative rule sources, state machines, calculations, every entry
point (UI, CLI, MCP, queue/webhook).
Must check: rule parity across entry points, cross-field invariants on merged
partial updates, min/max/empty/overflow, date/DST edges, exact decimal or
minor-unit math (never binary float for exact values), explicit
`(from_state -> to_state)` guards with terminal states locked, canonicalize
before validate, allowlists, and payload bounds. Record unstated rules as
`UNKNOWN`, never invent thresholds.

### 3.5 `data-integrity-and-lifecycle`

Use when reviewing scene stores, undo deltas, caches, export pipelines, and
create/update/delete/archive/expire flows.
Inspect: DDL/migrations, queries, transaction wrappers, cache hooks, mutation
endpoints, lifecycle jobs; note engine, isolation level, deployment topology.
Must check: constraints in the persistence layer (`NOT NULL`, `UNIQUE`,
`CHECK`, FKs, atomic conditional writes), no slow network I/O inside a
transaction, transaction propagation + rollback on every error path, no
dual-write presented as atomic without outbox/saga, idempotency contracts
(key binding, payload/actor mismatch rejection, concurrent-duplicate safety),
soft-delete filtering, cascade behavior, bounded purge/expiration. Never
extrapolate single-threaded tests to concurrent safety.

### 3.6 `evidence-led-concurrency-and-delivery`

Use only when the task concerns lifecycle/delivery guarantees: cancellation,
failure isolation, ownership, durability, backpressure, fairness, shutdown.
Do not invoke it just because code uses threads/tasks/async syntax.
Define the contract first: admission, storage/durability, ordering, ownership
period, ack/retry, idempotency, cancellation propagation, join/wait, shutdown
and recovery. Then trace one work item end to end and bound every resource
(batch, memory, queue depth, retries, timers, concurrency, shutdown time).
Tessel mapping: `tokio` lives outside `tessel-core`; render-loop work must not
allocate per frame for static data (buffer reuse + `DirtyFlags`); shutdown
timeout proves only the deadline expired, not that work stopped.

### 3.7 `observability-and-audit-trail`

Use when reviewing logs, metrics, traces, alerts, `/livez` vs `/readyz`, and
append-only audit trails.
Must check: structured correlated logs with secret/PII redaction, no log
storms in loops, bounded metric cardinality (never raw IDs/URLs/errors as
labels), trace propagation across sync/async boundaries, liveness isolated
from soft dependencies with short bounded readiness timeouts, and durable
audit events (actor, action, target, outcome, UTC time, correlation ID).
Tessel mapping: follow `.knowledges/docs/error-and-logging.md` for severity
and redaction discipline.

### 3.8 `performance-and-resource-efficiency`

Use when asked to make something faster or leaner. Baseline first: target
metric (latency/tail, throughput, memory, CPU, I/O, startup, cost) plus
workload shape (sizes, frequency, hot/cold paths).
Inspect the full path (loops, data structures, allocations, serialization,
queries, concurrency, caches) before changing it. Prefer better complexity,
fewer allocations, fewer round trips, and bounded concurrency over
micro-optimizations. Caching never substitutes for fixing the algorithm; when
used, define capacity, TTL, invalidation, and the exact fallback path that
protects the origin from a miss storm. Report complexity before/after plus
benchmark/profile/test evidence; never claim "faster" without it.

### 3.9 `resilience-and-error-handling`

Use when hardening error paths and dependency boundaries (HTTP/gRPC clients,
DBs, caches, queues, third-party APIs, export/import I/O).
Classify each dependency as hard (fail closed) or soft (safe degradation that
preserves auth and correctness bounds). Must check: explicit
skip/retry/fallback/propagate per failure class, no swallowed errors or dummy
fallbacks without observability, bounded retries with jitter that are gated on
retryable + idempotent operations, consistent connect/read/overall timeouts,
no cross-layer retry amplification breaching outer deadlines, liveness
separated from soft dependencies, and both failure entry and post-outage
recovery (refill/herd protection, stale-state clearing) verified by
fault injection, not by reading timeout constants.

### 3.10 Visual and delivery skills

Read `visual-coding` first: it names the single owner for the deliverable and
forbids running two owners' processes at once.

- `blender`: one idempotent `build.py` (`reset_scene()` first, tunables on
  top), preview then `--final` modes, blocking before detail, real camera +
  lighting + materials, every render inspected. Finish only when every output
  exists at the requested path and the script reruns cleanly.
- `browser-app-delivery`: derive the real start command, run it, hand off one
  start command plus one concrete URL with explicit open-in-browser wording
  (or the exact artifact path + smoke result for standalone). Never invent a
  URL or a server for a standalone artifact.
- `browser-game`: size the work (Compact / Standard / Full), build in its own
  folder with separate entry/style/code files, play with real input, run the
  required critic round(s), fix blockers and majors, final-play the exact
  delivered files.
- `game-3d-visuals`: run only for polished, art-directed results with
  references or explicit fidelity asks. References in `visual-plan/refs/`,
  visual PRD, blockout gate, render-vs-reference acceptance loop.
- `web3d`: plan forms/materials/light before effects; headless
  `check_page.py` at desktop + phone, zero console errors or shader warnings,
  test-list inputs driven through the page's own controls, player QA (gate P)
  on detailed briefs, final run on the exact delivered bytes.

### 3.11 Other skills you may need

- `prompt-confidence-gate`: gate an ambiguous prompt before executing; read
  the threshold, score candidate intents, ask when confidence is low.
- `bundled:git`: never amend/push/rebase/reset without an explicit request;
  filter "my commits" by configured identity; never bulldoze lock files.
- `bundled:plan`: on an explicit planning request, research first, return one
  concise inline plan, do not implement in the planning turn.
- `bundled:taste`: preflight before creating or visually styling human-facing
  web UI (not for non-visual logic or machine-only HTML).
- Tessel `testing-strategy`, `release-engineering`, `config-and-mcp-setup`,
  `blueprint`, and `file-format-spec` under `.knowledges/docs/`: load when the
  audit touches tests, releases, MCP setup, scope, or export formats.

## 4. Audit report template

```markdown
# Audit: <area> (<date>)

## 1. Scope
Inspected: <crates/files/endpoints/scenes, skills used>.
Out of scope: <items> (each non-applicable section below says OUT_OF_SCOPE + reason).

## 2. Evidence ledger
| Claim | Evidence | Status | Validation |
|---|---|---|---|
| ... | <file:line, config, command output> | VERIFIED_IN_SCOPE / ... | <how to recheck> |

## 3. Findings (one section per skill used; delete the rest)
- `CONTRADICTED_IN_SCOPE`: <exact failing path + counterexample>.
- `PARTIAL`: <which paths/modes fail>.
- `NOT_FOUND_IN_SCOPE`: <searched boundary, no implementation found>.
- `UNKNOWN`: <what is missing and why it matters>.

## 4. Validation performed
<Exact commands + observed results: cargo fmt/clippy/test, fault injection,
boundary tests, benchmark/profile numbers, check_page runs, scanner output.>

## 5. Assumptions and remaining risks
<Unverified consumers/providers, uninspected layers, env vars, runtime DI,
external dashboards; each with impact and how to validate.>
```

## 5. Tessel pre-merge checklist

- [ ] Blast radius inspected before editing; source graph synced after the
  last edit (per the graph workflow in `AGENTS.md`).
- [ ] No `unwrap()`/`expect()` outside `#[cfg(test)]`; errors use
  `thiserror` (`anyhow` only at binary startup).
- [ ] `tessel-core` imports no `wgpu`/`winit`/`egui`/`tokio`/`reqwest`.
- [ ] Scene mutations go through `Command` (undoable, deterministic).
- [ ] SRP + anchor layout kept (`nano.rs`, sub-unit, unit, `ops.rs`,
  `commands.rs`); cross-crate behavior added via traits/adapters.
- [ ] Coordinate/export rules kept (`+Y` up, `+Z` front, CCW, voxel scale).
- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` all pass; findings carry evidence statuses.

