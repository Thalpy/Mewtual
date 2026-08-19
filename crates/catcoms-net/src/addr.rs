//! Multiaddr address classification: the one place that decides whether an address literal
//! means "this machine", "this LAN", "not an endpoint at all", or "somewhere on the public
//! internet".
//!
//! These predicates were duplicated three times (here, in the desktop bridge, and in
//! `catcoms-sync`'s peer-record filter) and a fourth copy would be its own bug: the whole
//! class of defect they exist to close is an address that one classifier calls hostile and
//! another calls fine. The desktop bridge now imports these; `catcoms-sync`'s copy stays where
//! it is (it walks the multiaddr *string*, before parsing) and is kept in agreement by hand.
//!
//! Three different questions get asked of an address, and they are genuinely different:
//!
//! * **May we advertise it?** ([`addr_is_private`]) A LAN or loopback address published to a
//!   rendezvous is undialable by the recipient and is a free map of our internal network.
//! * **May we dial it?** ([`addr_is_undialable`]) A LAN address is a perfectly good dial target
//!   (the most common first invite is someone in the same house); a multicast group or a
//!   link-local address is not an endpoint, and a name resolved at dial time is a target we
//!   never get to inspect.
//! * **Is it on the public internet?** ([`addr_is_globally_routable`]) The strictest of the
//!   three, and the one an *attacker-supplied* infrastructure address has to satisfy.

use std::net::{Ipv4Addr, Ipv6Addr};

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

/// Whether an IPv4 literal means nothing outside this machine or this LAN.
pub fn ipv4_is_local(ip: &Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        // RFC 6598 100.64.0.0/10, the carrier-grade-NAT block: routable in form, useless
        // in practice, and `Ipv4Addr::is_shared` is still unstable.
        || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
}

/// Whether an IPv4 literal cannot be a peer at all.
pub fn ipv4_is_undialable(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_multicast()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || o[0] == 0
        || o[0] >= 240
}

/// Whether an IPv4 literal is reserved for documentation or benchmarking, and therefore never
/// routed: `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24` (TEST-NET-1/2/3) and
/// `198.18.0.0/15`. `Ipv4Addr::is_documentation` covers the first three but not the fourth,
/// and `is_benchmarking` is still unstable, so both are spelled out.
fn ipv4_is_documentation(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
}

/// Whether an IPv4 literal could plausibly be reached from the public internet: not this
/// machine, not this LAN, not the carrier's NAT pool, not a multicast group or a reserved
/// block, and not a documentation range.
pub fn ipv4_is_globally_routable(ip: &Ipv4Addr) -> bool {
    !(ipv4_is_local(ip) || ipv4_is_undialable(ip) || ipv4_is_documentation(ip))
}

/// Whether an IPv6 literal points at this machine, **including** the IPv4-mapped spelling.
///
/// `Ipv6Addr::is_loopback()` is false for `::ffff:127.0.0.1`, which is how an earlier validator
/// was defeated: the mapped form sorted into the routable half and the joiner dialled its own
/// localhost. `to_ipv4_mapped`, not `to_ipv4`: the latter also matches the deprecated
/// `::a.b.c.d` form, which reads `::1` as `0.0.0.1` and misclassifies it.
pub fn ipv6_is_loopback(ip: &Ipv6Addr) -> bool {
    ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
}

/// The IPv6 counterpart of [`ipv4_is_globally_routable`].
///
/// `is_unique_local` / `is_unicast_link_local` / `is_documentation` are all still unstable, so
/// those are bit tests. Two families need spelling out beyond the obvious ranges:
///
/// * **IPv4 in IPv6 clothing.** `::ffff:192.168.1.5` is a private address written the other
///   way, and every rule below would be dodged by choosing the other spelling. Both the mapped
///   (`::ffff:a.b.c.d`) and the deprecated compatible (`::a.b.c.d`) forms are unwrapped and
///   judged by the IPv4 rules.
/// * **The transitional ranges**, which are the same dodge by a longer route: each embeds an
///   IPv4 address that the host stack unwraps, so `2002:c0a8:0101::` reaches `192.168.1.1` and
///   none of the checks above would have seen it. Nothing in this product ever publishes one,
///   so they are refused outright rather than unwrapped and re-judged.
pub fn ipv6_is_globally_routable(ip: &Ipv6Addr) -> bool {
    let s = ip.segments();
    if ipv6_is_loopback(ip) || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // fc00::/7 unique-local, fe80::/10 link-local, 2001:db8::/32 documentation.
    if (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || (s[0] == 0x2001 && s[1] == 0x0db8)
    {
        return false;
    }
    // 2002::/16 6to4, 2001:0::/32 Teredo, 64:ff9b::/96 and 64:ff9b:1::/48 NAT64.
    if s[0] == 0x2002 || (s[0] == 0x2001 && s[1] == 0x0000) || (s[0] == 0x0064 && s[1] == 0xff9b) {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_is_globally_routable(&v4);
    }
    // The deprecated `::a.b.c.d` compatible form. `::` and `::1` are already handled above, so
    // `to_ipv4`'s well-known over-match cannot misfire here.
    if s[..6] == [0, 0, 0, 0, 0, 0] {
        if let Some(v4) = ip.to_ipv4() {
            return ipv4_is_globally_routable(&v4);
        }
    }
    true
}

/// Whether a multiaddr names an address that means nothing outside this machine or this LAN:
/// loopback, RFC1918 private space, the RFC6598 CGNAT block, IPv4 link-local, IPv6 unique-local
/// (`fc00::/7`) or IPv6 link-local (`fe80::/10`), plus the unspecified addresses.
///
/// Such an address must never be **advertised**. Publishing it to a rendezvous or handing it to
/// a remote peer over identify discloses this machine's internal topology to anyone who asks,
/// and buys nothing: the recipient cannot route to it. (Loopback stays perfectly usable inside
/// an invite's bootstrap list, which is the deliberately same-machine case; this predicate
/// governs what leaves the box, not what an invite may carry.)
pub fn addr_is_private(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| match p {
        Protocol::Ip4(ip) => ipv4_is_local(&ip),
        Protocol::Ip6(ip) => {
            ipv6_is_loopback(&ip)
                || ip.is_unspecified()
                // fc00::/7 unique-local and fe80::/10 link-local; both `is_unique_local` and
                // `is_unicast_link_local` are still unstable.
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                // An IPv4 address written in v6 form is still that address.
                || ip.to_ipv4_mapped().is_some_and(|v4| ipv4_is_local(&v4))
        }
        _ => false,
    })
}

/// Whether a multiaddr names something that cannot be a peer at all: a multicast group, an
/// IPv4/IPv6 link-local address, the unspecified addresses, the IPv4 broadcast address, the
/// reserved `0.0.0.0/8` and `240.0.0.0/4` blocks, or a **name resolved at dial time**.
///
/// Distinct from [`addr_is_private`], which asks a different question ("may we *advertise*
/// this?") and deliberately includes LAN addresses. A LAN address is a perfectly good thing to
/// *dial* (the most common first invite is someone in the same house); a multicast group is not
/// an endpoint, and a link-local address means a different machine on every network the invite
/// is opened on, which is what turns an invite into a scanner aimed at the reader's own segment.
///
/// DNS components are refused for the reason a peer record refuses them:
/// `/dns4/scan.attacker.tld/tcp/22` passes every check that can be made on the string and then
/// resolves, at dial time, to whatever the address's author currently points it at. Nothing this
/// app mints ever contains one.
pub fn addr_is_undialable(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| match p {
        Protocol::Ip4(ip) => ipv4_is_undialable(&ip),
        Protocol::Ip6(ip) => {
            ip.is_multicast()
                || ip.is_unspecified()
                // fe80::/10 link-local (`is_unicast_link_local` is still unstable).
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                // ...and the same address written as IPv4-in-IPv6 (mapped form only).
                || ip.to_ipv4_mapped().is_some_and(|v4| ipv4_is_undialable(&v4))
        }
        Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => true,
        _ => false,
    })
}

/// Whether a multiaddr points at this machine. Folds IPv4-in-IPv6, so `/ip6/::ffff:127.0.0.1`
/// is recognised as loopback rather than sorted into the routable half of a dial plan and
/// dialled at the reader's own localhost.
pub fn addr_is_loopback(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| match p {
        Protocol::Ip4(ip) => ip.is_loopback(),
        Protocol::Ip6(ip) => ipv6_is_loopback(&ip),
        _ => false,
    })
}

/// Whether a multiaddr carries a name that is resolved at dial time.
pub fn addr_has_dns(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| {
        matches!(
            p,
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
        )
    })
}

/// Whether every host component of a multiaddr is an IP literal on the public internet.
///
/// The strictest of the three questions in this module, and the one asked of an address chosen
/// by a party we do not trust. It requires an IP literal to be *present*: an address with no
/// host component at all (the memory transport) and an address that only names a host
/// indirectly (DNS) both fail, because neither can be judged now, and "judged later, by the
/// resolver, against whatever the author points the name at" is precisely the hole.
pub fn addr_is_globally_routable(addr: &Multiaddr) -> bool {
    let mut saw_literal = false;
    for p in addr.iter() {
        match p {
            Protocol::Ip4(ip) => {
                saw_literal = true;
                if !ipv4_is_globally_routable(&ip) {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                saw_literal = true;
                if !ipv6_is_globally_routable(&ip) {
                    return false;
                }
            }
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => {
                return false
            }
            _ => {}
        }
    }
    saw_literal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Multiaddr {
        s.parse().expect("test multiaddr parses")
    }

    #[test]
    fn the_three_questions_are_deliberately_different() {
        // A LAN address must never be advertised, but is a perfectly good dial target (the most
        // common first invite is someone in the same house) and is not on the public internet.
        let lan = a("/ip4/192.168.1.5/tcp/9");
        assert!(addr_is_private(&lan));
        assert!(!addr_is_undialable(&lan));
        assert!(!addr_is_globally_routable(&lan));

        // A documentation range is not "private" (it names nobody's LAN) and is not obviously
        // undialable, but it is reserved so that it never routes.
        let doc = a("/ip4/203.0.113.7/tcp/9");
        assert!(!addr_is_private(&doc));
        assert!(!addr_is_undialable(&doc));
        assert!(!addr_is_globally_routable(&doc));

        // A public literal answers all three the same way.
        let public = a("/ip4/45.79.12.34/tcp/9");
        assert!(!addr_is_private(&public));
        assert!(!addr_is_undialable(&public));
        assert!(addr_is_globally_routable(&public));

        // A name is judged by the resolver, not by us: undialable, and never routable.
        let name = a("/dns4/rz.example.org/tcp/443/tls/ws");
        assert!(addr_has_dns(&name));
        assert!(addr_is_undialable(&name));
        assert!(!addr_is_globally_routable(&name));
        // ...but it is not "private": nothing here discloses internal topology, which is why
        // the operator variant of the rendezvous validator can accept one.
        assert!(!addr_is_private(&name));
    }

    #[test]
    fn loopback_folds_the_ipv4_mapped_spelling() {
        // `Ipv6Addr::is_loopback()` is false for `::ffff:127.0.0.1`; testing it directly is the
        // bug this exists to prevent.
        assert!(addr_is_loopback(&a("/ip4/127.0.0.1/tcp/9")));
        assert!(addr_is_loopback(&a("/ip6/::1/tcp/9")));
        assert!(addr_is_loopback(&a("/ip6/::ffff:127.0.0.1/tcp/9")));
        assert!(!addr_is_loopback(&a("/ip4/45.79.12.34/tcp/9")));
        // `::1` must not be read through `to_ipv4` as "0.0.0.1", which is neither loopback nor
        // routable and would have been sorted into the wrong half.
        assert!(!ipv6_is_globally_routable(&"::1".parse().unwrap()));
        assert!(ipv6_is_loopback(&"::1".parse().unwrap()));
    }

    #[test]
    fn an_address_with_no_host_component_is_never_globally_routable() {
        assert!(!addr_is_globally_routable(&a("/memory/1234")));
        assert!(!addr_is_globally_routable(
            &a("/ip4/45.79.12.34/tcp/9").with(Protocol::Dns4("rz.example.org".into()))
        ));
    }
}
