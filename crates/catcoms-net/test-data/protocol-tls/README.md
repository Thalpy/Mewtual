# Public protocol test certificates

These certificates are test data, never production trust anchors. No private key
file is stored here. The tests use publicly known Ed25519 seeds: CA = [1;32],
localhost leaf = [2;32], wrong-name leaf = [3;32]. They are deliberately unsuitable
for any real identity. The leaf keys are reconstructed only in the cfg(test) code.

All validity intervals are fixed at 2020-01-01 through 2100-01-01 UTC. The CA
permits certificate signing; both leaves permit TLS server authentication.
localhost.pem names localhost and 127.0.0.1; wrong-name.pem names wrong.invalid
and 127.0.0.2. Both leaf signatures verify against ca.pem. The custom CA is added
only to the test transport, never to the production root store.

The three public PEM files are explicitly tracked despite the repository-wide
PEM ignore. Keep private keys ignored. This prevents clean CI checkouts losing
the include_bytes! inputs while preserving the ordinary secret-file protection.
