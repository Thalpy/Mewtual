# Normal3 Legacy invite refresh

Status: private implementation under validation. Chat V2 remains disabled.
This stores an already-minted Legacy invite; it does not mint one, authorize a
join, supply a native command credential or construct an SDK/Source owner.

`refresh_legacy_invite_data` uses the same original physical allocation, poisoned
mutex cell, inventory, claim, exact ciphertext and outgoing retirement debt as
Rename. A retained tagged request includes the operation kind, stable row identity,
numeric ID, expected metadata revision and complete input. Another kind or changed
field cannot take over an uncertain attempt. Exact retained retry draws no RNG.

Only active Legacy accepts refresh. Identity and revision checks precede equal
invite handling. Equal input at the current revision is a no-change observation;
equal input at an old revision is a refusal, not a recovered invocation receipt.
Rename and refresh share that revision, so delayed metadata cannot rebase over
either operation. Birth/origin, group/DM classification, name, other rows and ID
highwater remain unchanged. The existing canonical successor check also preserves
every row's mandatory future completion/retirement space.

The invite bound is checked before file access, equality scanning or input copy,
including retained retries. Exactly one maximum 64KiB frozen request payload is
explicitly counted in the existing independently prepaid 2MiB control assertion.
It is separate from selected/candidate payloads and frame/AEAD terms. No fee or
pool ceiling increases, second preparation cell or payment-lifetime substitute
is introduced. Refusal diagnostics contain no invite payload.

Warm verification derives the operation from the authenticated sealed successor
and requires it to match the actual request. Both IO constructors and continuous
cold inspection use one closed Rename-or-Refresh predicate. Cleanup still permits
only exact duplicate, attributable short Stage or actual AEAD failure. Other
authenticated changes, malformed/future records and simultaneous JOIN/native WAL
are preserved and refused. Existing format3 already describes this exact
transition; version1/2 grammars and key domains remain unchanged.

The pinned `4f45d586f6e23f6acc0914fa83d93792544c5bba` predecessor understands
selected format3 and Rename recovery. The extended gate requires its actual
three openers to examine eleven fixtures. Positive controls include a clean
refreshed index and its existing supported Rename/torn/duplicate/bad-MAC recovery.
An authenticated Refresh Stage must refuse without changing any file. The head
then verifies all eleven round trips. These are pending executions, not claims
that a modern unit decoder proves old-binary compatibility.

Focused semantic and mounted-IO tests cover maximum input, terminal capacity,
stale revisions, all four warm/cold write cuts, exact ciphertext retry, cross-kind
conflicts, both close/admission orderings and original physical-cell poison.
The initial nine cases pass (50.59s), and all 98 combined lifecycle cases pass
(442.83s), including two additional actual-boundary regression controls from
review. Design/diff review and re-review are clear. The compiled registry has
207 ordinary cases plus two explicit compatibility cases. All 3,441 frontend
tests, Svelte checking and production building pass. Actual predecessor/native
checks, full exact-source suites and strict lint/dependency gates remain required
before sign-off. Other normal3 operation
producers, scoped Legacy/native consumers, joined-member provenance/transport and
combined text Chat V2 acceptance remain unfinished.
