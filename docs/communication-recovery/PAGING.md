# Bounded history page work

Cursor-capable catch-up pages inspect at most 256 retained operations and traverse at most
2,048 dependency steps, including unknown and duplicate hashes. The closure is conservative:
work omitted from the optimization can cause duplicate transmission, never omitted history.
A provider cursor advances across already-held prefixes even when the resulting page is empty.

The requester grants at most eight advancing empty pages per document/provider without a stall
penalty, then applies the existing eight-round non-progress bound and cooldown. Changing provider
identities or repeating a position is not advancement. Cooldown cannot renew empty-page grace;
actual accepted history or signed completion clears it. The tracking map uses the existing bound.
No new wire tags, authority, operation identity or storage schema are introduced.

Compatibility limitation: clients without provider cursors retain the previous scan behavior.
Bounding their duplicate-only prefix without a continuation would strand them at that prefix.
This feature does not claim a universal CPU/time bound for legacy operation parsing, no-cursor
requests, or the older full-history exchange. Those require a separate negotiated boundary.

Verification on Windows: two replication regressions pass (duplicate-prefix continuation and
conservative/unknown-head closure bounds). All 278 sync library tests pass, including three new
signed requester regressions for honest empty pages, malicious unlimited advancement, and
repeated/rotating provider cursors. Raw outputs: `logs/cr-page-work-tests.log` and
`logs/cr-bounded-pages-sync-final.log`. Independent adversarial review found no remaining
blocker/high; its unnecessary honest-provider cooldown finding is corrected by the bounded grace.
