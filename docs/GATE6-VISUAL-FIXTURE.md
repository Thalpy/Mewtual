# Gate 6 visual Studio fixture

Use `http://localhost:1420/?fixture=chat&studio=1` with the development frontend at 900×640,
wait for `html[data-visual-ready="chat"]`, then open the normal Studio tab and its entries.
The existing DEV + localhost fixture entry point and readiness marker are unchanged. Ordinary
`?fixture=chat` data is unchanged; production builds never install either fixture variation.

The explicit Studio opt-in supplies canonical decimal channel `1` (displayed as general),
real native-shaped Index/Flipnote/Score DTOs, a stable mocked group/full device identity,
three real 192×144 PIX1 cat frames, authored SFX durations and linked Score. Separate entries
expose retained sound alternatives and remove-wins Score/header/extent conflicts. Every DTO
passes through the same native readers as production. CIDs are stable mocked lookup keys,
not renderer-derived publication or encrypted-content claims.

This is a **read-only visual fixture**. It implements only the explicit list/read/blob and
empty draft/recovery reads needed to inspect those states. Save/Create/publication/GIF/repair
and historical mutation commands fail loudly; they never return fake Saved results or fake
files. Unknown dependencies still fail loudly. It is not a simulation of native admission,
durability, CRDT mutations, recovery authority, file reveal or actual audio-device behavior.
The primary agent owns composed App visual validation and full-suite acceptance.

Bounded verification: 16/16 existing/new fixture tests pass, Svelte checking reports zero
errors/warnings, and staged diff checking passes. Independent actual-diff review and a tiny
re-review close the one LOW readiness-fidelity finding; the reviewer independently reran all
16 focused tests. No BLOCKER/HIGH/MEDIUM/LOW remains for this read-only fixture checkpoint.
This is not acceptance of the composed application or Gate 6.
