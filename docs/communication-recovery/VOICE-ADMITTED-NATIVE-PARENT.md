# Admitted native parent capacity (29 October; inactive)

`Ledger::reserve_admitted_parent` accepts only the original opaque admitted
published-key wrapper, actual physical Session identity and nonzero generation.
It returns a distinct affine `AdmittedParentClaim`, with no conversion into an
Existing claim, selected owner, selector, Row or Action. Its identity predicate
is historical provenance, not NativeActive/current-selection authority.

The claim reserves one real position and monotone serial in the SAME32-slot
allocator used by Existing parent, confirmation and SDP rows. The index retains
only a scalar stamp. Ordinary allocators cannot consume that position; copied
RowId bytes cannot resolve a reserved claim as a Row. Future admitted consumption
must transfer the SAME stamp and prepaid phase/metadata contributions under
Index, without refunding/recharging or falling back to ordinary Begin.

The original2,304-byte metadata fee and independent2,048-byte phase contribution
are reserved outside native guards. The complete production inventory includes
the claim/preparation wrappers, guard and WHOLE retained SessionIdentity Arc
allocation. Owners are declared before Index and destroyed before the shared
private final-return marker can make the position reusable, including refusal
and unwinding destruction. Group/process/position limits are unchanged.

Four native regressions pass against actual admitted activation, key
publication and physical setup. They cover exact identity/fees, mixed16+16 claim
pressure and Existing parent/child allocator exclusion, partial-payment refusal
without serial debit, and End/unwind custody. The Existing pressure fixture tests
allocator exclusion only; it is not live SDP execution. The complete Windows
claim/preparation inventory measures1,160/2,304B. The integrated admitted filter
passes54/0 (39.97s), including these four cases and the separate bootstrap tests.
A fifth focused case also passes: the original physical Row actually returns
with no external identity alias, while the SAME claim/stamp/fees remain until
its own final drop. Design/actual-diff review and the coverage re-review are clear.
Full suites and strict gates remain pending; these are separate source-bound
focused checks, not a complete backend result.

The admitted native Slot/operation31 consumer, bilateral agreement/RoundGate and
admitted parent publication are not implemented by this primitive. It grants no
Ready, KDF, SDP/ICE/RTC or media permission. V2 remains OFF.
