# ADR-0007: No migration from the shell script

Status: accepted, 2026-09-30. Supersedes the migration parts of PLAN.md
(sections on the legacy script, its import and the cutover) and ADR-0002's
adoption of its topics.

## Context

The daemon was written to replace `publish_metrics.sh`, a shell script that
the owner had installed by hand on the two test routers. The script was a
model of the job, not a product with other users. The first versions still
carried a migration for it: detection of the script, its cron line and its
`rc.local` block (Diagnostics page), an import of its broker settings (Broker
page, `mqtt-metricsd --import-legacy`), the adoption of its retained topics
on the first pass (`identity.cleanup_legacy`) and the tools for the
switch-over (`tools/cutover.sh`, `tools/parity-check.py`,
`docs/MIGRATION.md`). Both routers were switched over on 2026-09-29.

## Decision

- The migration is removed from the packages, the documentation and the
  tools. Nobody else needs to know about the script.
- What the switch-over needed is kept where it still matters: the default
  set keeps the topics, `unique_id`s and names the script used, because Home
  Assistant keeps the history of an entity only while they stay the same.
  `tests/topic_compatibility.rs` checks them against the topics captured
  from the production broker (`tests/fixtures/primary/retained_topics.txt`).
- The set that is on by default is called the default set; the automatic
  interface selection (`wan*`, `wwan*`) is called automatic.
- The history (PLAN.md, GOAL.md, the ADRs and `docs/evidence/`) is kept as
  written. It refers to files that no longer exist; they are in the Git
  history before this change.

## Consequences

- `state.json` files that still carry `legacy_seeded` load as before; the
  field is dropped on the next write. A leftover `identity.cleanup_legacy`
  option is ignored.
- A new router that still runs the script gets no help from the packages:
  stop the script before starting the service, or both publish.
- The script and its marker file on the two test routers are removed by
  hand, with the owner's confirmation per router.
