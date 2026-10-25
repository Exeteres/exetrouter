# Automatic routing: schema 11 to 12

Migration 012 adds per-account routing metadata, durable quota-window absence fences, per-account redemption observations and a reset-attempt ledger containing credit/cycle digests, local account IDs, timestamps, numeric quota snapshots and fixed outcomes. At most two absence fences and one metadata/observation row exist per account. The attempt ledger retains recovery and final-day cohort guards. Upstream credit IDs remain transient; neither prompts nor credentials enter these tables. Existing user activation, operator locks, OAuth ciphertext, keys, usage, cooldowns, context proofs and retired routing settings are preserved. Retired priorities/thresholds no longer influence routing.

## Upgrade

This is a reviewed schema/behavior change, not an unattended packaging update. Obtain operator approval, inspect the actual deployment, stop the old runtime and take/verify a consistent private backup with both keys. Keep the exact old binary/image and configuration. Never run two copied OAuth pools as a migration technique.

Deploy matching server, native SSH gateway and client binaries. The new server/standalone runtime automatically refreshes metadata and may spend expiring reset credits under the [new contract](account-pool.md#ranking-and-switching); no dashboard is needed. This is intentional operational behavior, not an installation check. Validate health and read-only Doctor/model discovery without inference or deliberate credit use. Check schema 12, foreign keys, integrity, preserved accounts/users/availability and diagnostic capabilities. Routine Docker updating must still refuse an unreviewed schema change.

## Rollback

Stop the new runtime. Back up its current database and keys first. Reset redemption is irreversible; restoring an old snapshot does not restore a spent credit and may discard usage or rotated OAuth credentials.

The four new tables and their indexes are additive. After reviewing that they have exactly the migration-012 structure and that no later schema migration exists, keep the tables and their rows intact and set `PRAGMA user_version=11` in a reviewed offline transaction. Install the previous server and gateway together. Older code ignores these tables and resumes its saved manual routing settings. There are no pending confirmation handles to recover.

On re-upgrade, migration 012 retains the ledger and observations (`CREATE TABLE IF NOT EXISTS`). Never delete them to retry a reset: all potentially successful outcomes remain fenced until fresh explicitly allowed limits prove recovery and a new weekly cycle. The account/weekly-cycle upstream key is stable across upgrades and hosts. Finish with integrity/foreign-key checks and read-only diagnostics. A backup restore is disaster recovery, not a routine rollback strategy.
