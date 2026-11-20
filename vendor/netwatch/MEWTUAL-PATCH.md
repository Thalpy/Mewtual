# Netwatch Windows dependency feature patch

Source: crates.io `netwatch` 0.19.3, verified against the existing shipping
lockfile SHA256 `39da9cad5f23aa43401f09497d3b280e51b5feba115d9ffbf38ca35d6ff95e96`.
The published package and licenses are retained, with the changes below.

The published and original manifests change: the Windows `wmi` dependency
disables default features. Netwatch's sole WMI query deserializes the `Name`
string from `Win32_IP4RouteTable`; it uses no WMI date/time types. Disabling
WMI's optional default Chrono integration preserves this query, COM bindings,
route selection and event handling while removing the unmaintained dependency
reported in RUSTSEC-2026-0543. No advisory is ignored.

Strict hosted compilation also exposed five deprecated `noq-udp` 1.4.0
`UdpSocketState::send` calls in the published Netwatch source. Registry
dependencies normally cap their own warnings; a local path patch does not.
`src/udp.rs` adds five explicit `expect(deprecated)` annotations: four send
statements and one short method, each containing one deprecated call. All
executable Rust statements remain unchanged. No job-wide or crate-wide warning
allowance is added.

The new `try_send` API returns errors the original API intentionally silences,
including message-size errors from MTU probes. Blindly replacing it would
change Iroh's transport behavior just to fix compilation. This patch retains
the locked upstream behavior. Any Netwatch/NoQ change must revisit these five
expectations and remove them when upstream migrates its UDP error policy.
Every other Rust source remains byte-identical to the published crate.

Reference: https://docs.rs/noq-udp/1.4.0/noq_udp/struct.UdpSocketState.html#method.send

Both Mewtual workspaces patch the same exact Netwatch version and source.
Root/native dependency audits, complete hosted suites and the Windows WMI
compilation gate remain required; no local runtime validation is claimed.
