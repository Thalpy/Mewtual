//! Domain-separated derivation contexts.
//!
//! Per-document keys are derived from the MLS exporter secret. To keep the
//! derivation collision-free we feed the exporter a **fixed-width** context
//! (`u16` document-type tag followed by a `u128` document id), which is trivially
//! injective. We also keep *content* derivation ([`CHANNEL_EXPORTER_LABEL`])
//! domain-separated from *network-visible* derivation ([`METADATA_EXPORTER_LABEL`])
//! so that learning a content key never reveals network identifiers and vice
//! versa.

/// MLS exporter label for per-channel / per-document **content** keys.
pub const CHANNEL_EXPORTER_LABEL: &str = "catcoms channel v1";

/// MLS exporter label for **network-visible** identifiers (gossipsub topics,
/// rendezvous namespaces). Deliberately distinct from [`CHANNEL_EXPORTER_LABEL`].
pub const METADATA_EXPORTER_LABEL: &str = "catcoms metadata v1";

/// MLS exporter label for **real-time media** keys (E2E voice/video frame encryption).
/// Domain-separated from content + metadata keys, so a media key reveals neither.
pub const MEDIA_EXPORTER_LABEL: &str = "catcoms media v1";

/// Logical document types within a server. Encoded as a fixed-width `u16` tag so
/// the derivation context is injective. Values are stable across versions — only
/// ever append new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum DocType {
    /// A chat channel's append-only message log.
    Channel = 1,
    /// A wiki page document.
    Wiki = 2,
    /// The status / events feed.
    Status = 3,
    /// The calendar document.
    Calendar = 4,
    /// The single-use-invite ledger.
    InviteLedger = 5,
    /// The member roles / permissions document.
    MemberRoles = 6,
    /// The file index (fileshare browser metadata).
    FileIndex = 7,
    /// Network-routing identifiers (blinded gossipsub topics + rendezvous
    /// namespaces). Has no content document of its own — it only ever feeds the
    /// **metadata** exporter label to derive the per-removal routing secret
    /// (`ns_secret_L`). Kept a distinct `DocType` so that derivation is
    /// domain-separated from every content document by the injective context.
    Routing = 8,
    /// Per-member profiles (display name, color, font, text effect) — a single
    /// shared CRDT document per server keyed by member device fingerprint.
    Profile = 9,
}

impl DocType {
    /// The stable fixed-width tag for this document type.
    pub fn tag(self) -> u16 {
        self as u16
    }

    /// Recover a document type from its stable tag.
    pub fn from_tag(tag: u16) -> Option<Self> {
        Some(match tag {
            1 => DocType::Channel,
            2 => DocType::Wiki,
            3 => DocType::Status,
            4 => DocType::Calendar,
            5 => DocType::InviteLedger,
            6 => DocType::MemberRoles,
            7 => DocType::FileIndex,
            8 => DocType::Routing,
            9 => DocType::Profile,
            _ => return None,
        })
    }
}

/// Low-level, injective context encoding: `u16` tag ‖ `u128` id (18 bytes,
/// big-endian, fixed width). Exposed so the injectivity property can be tested
/// over arbitrary tags.
pub fn context_bytes(doc_type_tag: u16, doc_id: u128) -> [u8; 18] {
    let mut out = [0u8; 18];
    out[0..2].copy_from_slice(&doc_type_tag.to_be_bytes());
    out[2..18].copy_from_slice(&doc_id.to_be_bytes());
    out
}

/// The canonical MLS-exporter **context** for a document's key derivation.
///
/// Because the encoding is fixed-width, two distinct `(doc_type, doc_id)` pairs
/// can never produce the same context bytes — which is exactly what makes
/// per-channel key separation sound.
pub fn exporter_context(doc_type: DocType, doc_id: u128) -> [u8; 18] {
    context_bytes(doc_type.tag(), doc_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_vectors() {
        // Channel, id = 1.
        assert_eq!(
            exporter_context(DocType::Channel, 1),
            [0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01]
        );
        // FileIndex (tag 7), id = 0.
        assert_eq!(
            exporter_context(DocType::FileIndex, 0),
            [0x00, 0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00]
        );
        // Routing (tag 8), id = 0 — the network-routing derivation context.
        assert_eq!(
            exporter_context(DocType::Routing, 0),
            [0x00, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00]
        );
        // Profile (tag 9), id = 0 — the per-member profile document.
        assert_eq!(
            exporter_context(DocType::Profile, 0),
            [0x00, 0x09, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00]
        );
    }

    #[test]
    fn from_tag_roundtrips_every_variant() {
        for dt in [
            DocType::Channel,
            DocType::Wiki,
            DocType::Status,
            DocType::Calendar,
            DocType::InviteLedger,
            DocType::MemberRoles,
            DocType::FileIndex,
            DocType::Routing,
            DocType::Profile,
        ] {
            assert_eq!(DocType::from_tag(dt.tag()), Some(dt));
        }
        // Unknown tags decode to None (stable: 0 and the first unused value).
        assert_eq!(DocType::from_tag(0), None);
        assert_eq!(DocType::from_tag(10), None);
    }

    #[test]
    fn labels_are_distinct() {
        assert_ne!(CHANNEL_EXPORTER_LABEL, METADATA_EXPORTER_LABEL);
    }

    #[test]
    fn same_id_different_type_differs() {
        assert_ne!(
            exporter_context(DocType::Channel, 42),
            exporter_context(DocType::Wiki, 42)
        );
    }

    #[test]
    fn same_type_different_id_differs() {
        assert_ne!(
            exporter_context(DocType::Channel, 1),
            exporter_context(DocType::Channel, 2)
        );
    }
}
