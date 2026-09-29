# ADR-0006: Clear the retained state when a value goes away

Status: accepted, 2026-09-29. Supersedes the rule in PLAN.md section 5
("empty or unavailable values are never sent as empty retained payloads").

## Context

The plan treated the legacy script's empty retained payloads as a defect
(they delete the retained state). The daemon therefore published nothing for
a value it could not read. The audit found the consequence: after a WAN goes
down, its address, gateway, device, latency or rate topics keep the last
retained value, and Home Assistant keeps showing it as current while the
router is online. WAN failover is the main thing this integration is used to
watch. The legacy script cleared such values on its next run.

## Decision

A reading that is unavailable is one of three things:

| Kind | Meaning | State topic |
| --- | --- | --- |
| absent | the value is not there now (interface down, target silent, instance gone) | a retained value published earlier is cleared once |
| failed | the source could not be read (timeout, garbled answer) | the last value is kept |
| permanent | the entity cannot exist on this router | not announced; leftovers cleared on a full pass |

- The engine remembers per entity what it left on the broker (`Held`). After
  a restart it does not know, so an entity that is owned according to
  `state.json` and currently absent is cleared once.
- An instance that disappears from a group that was read successfully is
  treated as absent. A group whose source failed says nothing about its
  instances.
- Nothing else changes: a placeholder value is never published, and
  discovery configs stay in place when a value goes away (removing them
  would delete the entity from Home Assistant's registry and lose its
  customizations every time an interface flaps).
- "Permanent" is data-driven: an address that exists is always published,
  and a protocol list only decides whether a *missing* address is permanent.

## Consequences

- Home Assistant shows unknown, not a stale value, when a WAN is down.
- A transient `ubus` failure does not blank anything.
- Home Assistant ignores an empty payload for binary sensors and keeps their
  previous state. The binary sensors that can lack a value (mwan3 online,
  cluster peer/service state, carrier) are opt-in and rarely absent.
- After every daemon restart, sensors that need two samples (CPU, rates)
  show unknown for one interval instead of a value from before the restart.
