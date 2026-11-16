# Security prerequisite for the PR backlog

This branch starts at main88f36129. It extracts the dependency repairs from
PRs93/94/103/113 without their intervening Chat V2 implementation. PR84 remains
draft while its own validation and this prerequisite are incomplete.

## Scope and compatibility

Both shipping workspaces use libp2p0.57.1 and the same local gossipsub0.50.1
correction, plus Hickory proto/net/resolver0.26.3. The compatible QUIC/Quinn/UDP
and rustix/xattr fixes are retained. No advisory exception, protocol feature
removal, new native command, Chat V2 storage namespace or activation is added.
The frontend sanitizer and its real DOM regression are carried from PR93.

Gossip keeps the existing finite2000-topic subscription limits and65536-byte
payload/control limits. The vendor patch splits initial subscriptions into
bounded frames rather than increasing a packet cap. Complete archive checksums,
the exact modified-file inventory and both runtime lock resolutions are checked
by `.github/scripts/check-network-security.py`.

The AutoNAT RNG adapter consumes an injected seed and advances forks; it does
not clone a random stream or introduce an ambient application RNG. The current
upstream QUIC/TLS stack explicitly uses AWS-LC; WebSocket uses ring. Net installs
no process-global provider, and peer-certificate verification never substitutes
for WebSocket public-PKI verification. Relay/rendezvous constructors keep their
existing transport choices. Protocol/key/signature identities remain unchanged.

Incoming subscriptions retry held gossip only for their exact canonical topic.
Malformed or unrelated topics cannot repeatedly sign the whole retained queue.
Existing periodic retry and bounded queue ownership are preserved.

## Independent extraction

The compatibility example derives expected peer identities from public APIs
available on both main and the pinned predecessor. It imports no later native
identity helper. The two security runners share a standalone bounded subprocess
and exact-result guard, rather than importing the namespace5/Chat V2 runner.
Its controls reject empty/ignored/duplicate/failed cohorts and exercise actual
child exit, timeout and bounded marker collection.

Old/new transport evidence builds the unchanged pinned predecessor944115c2,
adding only the identical public-API example. The gate checks source/lock/binary
hashes, authenticated QUIC, bidirectional requests, large signed gossip with2000
topics and actual withdrawal. The separate current/current pressure scenario
does not relabel a phased old/current exchange as load acceptance.

## Validation and limits

All171 extracted Net unit tests, all3388 frontend tests and the four bounded
runner regressions pass locally. Strict Root all-target/all-feature Clippy and
both dependency audits pass. Native/full-suite and transport compatibility
checks remain pending on this new source; earlier descendant passes are not its sign-off.
The exact Net cohort count comes from this compiled tree, not PR113.
Physical audio/network acceptance and production Chat V2 are outside this fix.

Ordinary Root CI uses the user's existing deferral for the single named
`cold_provider_transfers_only_one_missing_change_after_ten_thousand_operations`
cost case. There is no positive filter and no other skip. Final Chat V2
acceptance must run that case; an intermediate pass does not claim otherwise.

The lifecycle-mutation job receives90 outer minutes instead of60. All68
mutations, the three-way assignment, restored controls, source-preservation
guard and individual command deadlines remain unchanged. The previous shard1
run detected23 mutations but completed only12 restored controls before timeout.

## Foundation validation follow-up

The combined audit and strict checks passed, but both full Root jobs aborted
in the same Studio page fixture on the default test stack. The reviewed heap
fixture correction from the later stack is backported without changing its
assertions or limits. All six affected page tests pass before optimization.
The later stack's exact reviewed six-package development/test crypto profiles
are reused to shorten the next full validation; release settings and crypto
parameters remain unchanged. Fresh complete validation is still required.
