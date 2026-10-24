//! Fixture-only wire observations. Parsing proves neither authentication nor import. Keep
//! fixed metadata, never request/reply payloads, keys, addresses or arbitrary error strings.
use catcoms_wire::{Decoder, DocType};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CatchupMetadata {
    doc_type: DocType,
    doc_id: u128,
    heads: u32,
    // Exact ordered wire frontier, not semantic equality or proof these changes exist.
    frontier_digest: [u8; 32],
    paging: bool,
    resume: Option<u32>,
}

/// Borrow the complete released kind19 framing. Bounds precede every loop; Decoder lends
/// slices and the hash streams into fixed state. Malformed frames produce no observation.
pub(super) fn request(bytes: &[u8]) -> Option<CatchupMetadata> {
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let mut outer = Decoder::new(bytes);
    if outer.get_u8().ok()? != 19 {
        return None;
    }
    let inner = outer.get_bytes().ok()?;
    if outer.get_bytes().ok()?.len() != 32 {
        return None;
    }
    outer.get_u64().ok()?;
    if outer.get_bytes().ok()?.len() != 16 {
        return None;
    }
    outer.get_u64().ok()?;
    if outer.get_bytes().ok()?.len() != 64 {
        return None;
    }
    outer.finish().ok()?;

    let mut body = Decoder::new(inner);
    let doc_type = DocType::from_tag(body.get_u16().ok()?)?;
    let doc_id = body.get_u128().ok()?;
    let heads = body.get_u32().ok()?;
    if heads > 512 {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"Mewtual six-client wire frontier v1\0");
    hash.update(heads.to_be_bytes());
    for _ in 0..heads {
        let head = body.get_bytes().ok()?;
        if head.len() != 32 {
            return None;
        }
        hash.update(head);
    }
    let paging = !body.is_empty();
    let resume = if paging {
        if body.get_u8().ok()? != 1 {
            return None;
        }
        match body.get_bytes().ok()? {
            [] => None,
            cursor if cursor.len() == 20 => Some(u32::from_be_bytes(cursor[16..].try_into().ok()?)),
            _ => return None,
        }
    } else {
        None
    };
    body.finish().ok()?;
    Some(CatchupMetadata {
        doc_type,
        doc_id,
        heads,
        frontier_digest: hash.finalize().into(),
        paging,
        resume,
    })
}

/// Raw marker inside well-framed signed-response fields. This parser does NOT verify the
/// signature, recognized marker, bundle or request binding; only production ingress does.
pub(super) fn raw_reply_marker(bytes: &[u8]) -> Option<u8> {
    let mut reply = Decoder::new(bytes);
    if reply.get_bytes().ok()?.len() != 32 || reply.get_bytes().ok()?.len() != 64 {
        return None;
    }
    let body = reply.get_bytes().ok()?;
    reply.finish().ok()?;
    body.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_wire::Encoder;

    fn frame(count: u32, head_bytes: usize, cursor: Option<&[u8]>) -> Vec<u8> {
        let mut body = Encoder::new();
        body.put_u16(DocType::Channel.tag())
            .put_u128(923)
            .put_u32(count);
        for _ in 0..count.min(513) {
            body.put_bytes(&vec![7; head_bytes]).unwrap();
        }
        if let Some(cursor) = cursor {
            body.put_u8(1).put_bytes(cursor).unwrap();
        }
        let mut out = Encoder::new();
        out.put_u8(19).put_bytes(&body.finish()).unwrap();
        out.put_bytes(&[1; 32]).unwrap();
        out.put_u64(1000).put_bytes(&[2; 16]).unwrap();
        out.put_u64(5).put_bytes(&[3; 64]).unwrap();
        out.finish()
    }

    #[test]
    fn exact_fixed_document_frontier_and_cursor_observation_never_implies_authentication() {
        let mut cursor = [9; 20];
        cursor[16..].copy_from_slice(&42u32.to_be_bytes());
        let wire = frame(2, 32, Some(&cursor));
        let observed = request(&wire).unwrap();
        assert_eq!(
            (observed.doc_type, observed.doc_id, observed.heads),
            (DocType::Channel, 923, 2)
        );
        assert!(observed.paging);
        assert_eq!(observed.resume, Some(42));
        assert_ne!(
            observed.frontier_digest,
            request(&frame(1, 32, None)).unwrap().frontier_digest
        );
        // These dummy signature bytes are intentionally unverified test data.
        let mut reply = Encoder::new();
        reply.put_bytes(&[1; 32]).unwrap();
        reply.put_bytes(&[0; 64]).unwrap();
        reply.put_bytes(&[1, 0, 0, 0, 0]).unwrap();
        assert_eq!(raw_reply_marker(&reply.finish()), Some(1));
    }

    #[test]
    fn malformed_truncated_trailing_or_oversized_frontiers_never_create_metadata() {
        let wire = frame(2, 32, Some(&[]));
        for end in 0..wire.len() {
            assert!(request(&wire[..end]).is_none(), "truncated at {end}");
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert!(request(&trailing).is_none());
        assert!(request(&frame(513, 32, None)).is_none());
        assert!(request(&frame(u32::MAX, 32, None)).is_none());
        assert!(request(&frame(1, 31, None)).is_none());
        assert!(request(&frame(0, 32, Some(&[0; 19]))).is_none());
        assert!(request(&[0; 64 * 1024 + 1]).is_none());
        assert!(raw_reply_marker(&[0; 4]).is_none());
    }
}
