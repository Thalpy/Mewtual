# Current namespace DATA codec prerequisite (historical checkpoint)

The separate [physical opener](CHAT-V2-CURRENT-LAYOUT-OPENER.md) now consumes the descriptor,
marker and declaration-seed codec in production code, with fifteen further authored UNRUN
regressions. Only the unused group-name DATA wrapper remains cfg(test). It exposes a private
physical declaration/historical DATA reader, not a group task or Current issuer. Default,
Pending and ID2-only readers still reject ID3. The original narrower checkpoint below records
the seven-test prerequisite and its evidence at that time.

At that initial checkpoint the shared `critical_namespaces/archive_current_namespace_codec.rs` module was declared
**only under cfg(test)**. It supplies the sibling Current metadata tests with the reviewed
`CurrentLayoutData {vault,root}` and the exact ID3 descriptor, separately sealed marker,
group-wide deterministic directory-name data and independent declaration-seed framing.
It is not a production handler. Default/Pending/bootstrap readers still reject ID3, and no
new physical opener, supported Current policy, group reservation or Current issuer is enabled.

The exact grammar/caps and distinct KDF domains remain those in
`CHAT-OWNER-V2-GENESIS-ACTIVATION-FORMAT.md`. Declaration payloads retain the complete marker
and opaque catalog frame, plus the expected complete original catalog-frame identifier.
The DATA parser checks complete outer framing before opening its nested marker. It does not
verify catalog namespace/parent transitions, actual file/session/root barriers or saved-source
custody; those belong to the later real physical consumer. In particular, a different caller
parent label remains DATA rather than an authenticated original-parent observation.

Seven focused tests are authored, all **UNRUN** under the compilation/test/build pause:

* exact tags/version/role/length and full prefixes/EOF/context refusal;
* group-wide hash geometry and cross-vault/root/group separation;
* real AEAD marker/key/domain/ciphertext rejection;
* exact borrowed whole candidate/parent-label retention;
* malformed late outer ranges refusing before the actual nested marker opener;
* maximum catalog frame and oversize-before-RNG boundaries;
* authenticated malformed DATA and sanitized Debug.

Exact-format design and final actual-diff STATIC reviews are CLEAR, including the fixed
physical g- prefix comment. Direct formatting, whitespace, ambient-dependency and Store raw-fs
checks passed. Compilation, tests, builds and clippy were not run under the pause; source-only
checks are not execution evidence. The physical opener and complete across-signer fences still require their own actual design,
implementation and review under the activation-format contract; this prerequisite implements none
of those consumers.
Removing cfg(test) requires the real constructor/physical transaction, exact file/candidate
custody, paid namespace/index/metadata consumers and all cross-signer group fences together.
No test-only codec, parsed descriptor, scalar limit or declaration label may substitute for
those gates or a real SourceSaved/Current capture.
