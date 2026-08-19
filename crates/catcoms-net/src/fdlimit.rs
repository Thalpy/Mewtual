//! The file-descriptor headroom an internet-exposed infra node needs, checked at startup.
//!
//! Every admission decision in [`crate::admission`] is built on the node returning a clean
//! `ConnectionDenied` when it is full: a definite refusal is what sends a client to another rung
//! instead of leaving it on a hung socket. That story only holds while the *operating system* is
//! not the thing that runs out first. `RelayLimits::max_established_incoming` defaults to 8,192
//! connections; a stock Linux `RLIMIT_NOFILE` soft limit is **1,024**. So the shipped defaults hit
//! `EMFILE` in the accept loop at about an eighth of the configured capacity, and `EMFILE` is not
//! a refusal: `libp2p-tcp` logs it and keeps the listener, so callers see nothing at all and the
//! operator sees a node that is mysteriously deaf while reporting plenty of spare capacity.
//!
//! ## What this checks, and what it deliberately does not do
//!
//! It **reads** the soft limit and refuses to start when it is too low, naming the two fixes an
//! operator actually types (`LimitNOFILE=` in a systemd unit, `ulimit -n` in a shell). It does
//! **not raise** the soft limit, which would be the friendlier behaviour: `setrlimit` is only
//! reachable through `unsafe` FFI or a new dependency, and this workspace sets
//! `unsafe_code = "deny"`. Refusing to start with an actionable message is the sanctioned half of
//! "raise the soft limit or refuse to start naming `LimitNOFILE=`", so that is what happens.
//!
//! ## Portability
//!
//! The limit is read from `/proc/self/limits`, which is plain `std::fs` and compiles everywhere.
//! Where the file does not exist (Windows, macOS, a container without procfs) the check reports
//! "unknown" and passes: Windows has no per-process descriptor rlimit for sockets, and the
//! deployment target for these nodes is a Linux VPS. That is a real gap on macOS, where the
//! default soft limit is also low; it is named here rather than papered over.

use crate::NetError;

/// Descriptors a node needs beyond its connection cap: listeners, the identity file, log files,
/// resolver sockets, and the transient pair a connection briefly holds mid-upgrade. Small and
/// deliberately round; the number that matters is the connection cap next to it.
const HEADROOM: u64 = 128;

/// The process's soft `RLIMIT_NOFILE`, or `None` where it cannot be read on this platform.
///
/// Parsed out of `/proc/self/limits`, whose row is
/// `Max open files            1024                 4096                 files`.
/// `unlimited` maps to [`u64::MAX`].
pub fn soft_open_file_limit() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/limits").ok()?;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Max open files") else {
            continue;
        };
        let soft = rest.split_whitespace().next()?;
        if soft == "unlimited" {
            return Some(u64::MAX);
        }
        return soft.parse().ok();
    }
    None
}

/// Refuse to start when the soft descriptor limit cannot cover `max_established_incoming`
/// connections plus a little headroom.
///
/// `node` names the node in the error so an operator running both on one host knows which unit to
/// edit. Returns `Ok(())` when the limit is high enough **or** cannot be determined; a check that
/// cannot see the limit must not block a node that would have worked.
pub fn check_open_file_limit(node: &str, max_established_incoming: u32) -> Result<(), NetError> {
    let needed = u64::from(max_established_incoming).saturating_add(HEADROOM);
    let Some(soft) = soft_open_file_limit() else {
        tracing::debug!(
            node,
            needed,
            "open-file limit not readable on this platform; skipping the check"
        );
        return Ok(());
    };
    if soft >= needed {
        tracing::debug!(node, soft, needed, "open-file limit is sufficient");
        return Ok(());
    }
    Err(NetError::Build(format!(
        "the process may open only {soft} files but this {node} is configured for \
         {max_established_incoming} inbound connections (needing about {needed} descriptors). It \
         would hit EMFILE in the accept loop instead of refusing connections cleanly, which looks \
         to every caller like a node that is simply not answering. Raise the limit \
         (systemd: LimitNOFILE={needed}; shell: ulimit -n {needed}) or lower the connection cap."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_limit_never_blocks_start_up() {
        // On this developer platform there is usually no /proc, and a check that cannot see the
        // limit must pass rather than refuse a node that would have run fine.
        if soft_open_file_limit().is_none() {
            assert!(check_open_file_limit("relay", u32::MAX).is_ok());
        }
    }

    #[test]
    fn a_limit_that_is_high_enough_passes() {
        // Whatever this platform reports, a node configured for a single connection fits.
        assert!(check_open_file_limit("relay", 1).is_ok());
    }

    #[test]
    fn the_error_names_the_fix() {
        // Only meaningful where the limit is readable; elsewhere the check is a documented no-op.
        let Some(soft) = soft_open_file_limit() else {
            return;
        };
        if soft >= u64::from(u32::MAX) {
            return; // an unlimited process cannot be made to fail this check
        }
        let err = check_open_file_limit("relay", u32::MAX).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("LimitNOFILE="), "{msg}");
        assert!(msg.contains("ulimit -n"), "{msg}");
    }
}
