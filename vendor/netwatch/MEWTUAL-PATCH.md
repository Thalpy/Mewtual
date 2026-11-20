# Netwatch Windows dependency feature patch

Source: crates.io `netwatch` 0.19.3, verified against the existing shipping
lockfile SHA256 `39da9cad5f23aa43401f09497d3b280e51b5feba115d9ffbf38ca35d6ff95e96`.
The published source files and licenses are retained unchanged.

Only the published and original manifests change: the Windows `wmi` dependency
disables default features. Netwatch's sole WMI query deserializes the `Name`
string from `Win32_IP4RouteTable`; it uses no WMI date/time types. Disabling
WMI's optional default Chrono integration preserves this query, COM bindings,
route selection and event handling while removing the unmaintained dependency
reported in RUSTSEC-2026-0543. No advisory is ignored.

Both Mewtual workspaces patch the same exact Netwatch version and source.
Root/native dependency audits, complete hosted suites and the Windows WMI
compilation gate remain required; no local runtime validation is claimed.
