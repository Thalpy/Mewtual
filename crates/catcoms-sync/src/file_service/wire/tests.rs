use super::*;

fn devices() -> (MlsDevice, MlsDevice, MlsDevice) {
    (
        MlsDevice::generate().unwrap(),
        MlsDevice::generate().unwrap(),
        MlsDevice::generate().unwrap(),
    )
}
fn fields<'a>(helper: &MlsDevice, sources: &'a [DeviceId]) -> GrantFields<'a> {
    GrantFields {
        group: b"full server group",
        epoch: 7,
        helper: helper.device_id(),
        helper_peer: PeerId::from_u64(2),
        requester_peer: PeerId::from_u64(1),
        window: Window {
            cid: Cid::of(b"ciphertext"),
            max_cipher_bytes: 100,
            offset: 0,
            length: 10,
        },
        sources,
        created_ms: 1000,
        nonce: [8; 16],
    }
}

#[test]
fn original_delivery_and_both_signed_response_layers_bind_exact_frames() {
    let (c, b, a) = devices();
    let sources = [a.device_id()];
    let raw = fields(&b, &sources).issue(&c).unwrap();
    let grant = Grant::parse(&raw).unwrap();
    assert!(grant.verify_signature());
    assert_eq!(grant.requester(), c.device_id());
    assert_eq!(grant.group(), b"full server group");
    assert_eq!(grant.sources(), sources);
    let delivered = Delivery::issue(&grant, a.device_id(), PeerId::from_u64(3), &b).unwrap();
    let delivery = Delivery::parse(&delivered).unwrap();
    assert!(delivery.verify_signature() && delivery.grant().verify_signature());
    assert_eq!(delivery.grant().raw(), raw);
    assert_eq!(delivery.source(), a.device_id());
    let answer = SourcePage::issue(&grant, SourceStatus::Present, 20, b"0123456789", &a).unwrap();
    let answer = SourcePage::parse(&answer, &grant).unwrap();
    assert!(answer.verify_signature());
    let wrapped = HelperPage::issue(&grant, HelperStatus::Found, Some(&answer), &b).unwrap();
    let wrapped = HelperPage::parse(&wrapped, &grant).unwrap();
    assert!(wrapped.verify_signature());
    let source = wrapped.source().unwrap();
    assert!(source.verify_signature());
    assert_eq!(
        source.raw(),
        answer.raw(),
        "B preserves A's exact signed frame"
    );
    assert_eq!(source.source(), a.device_id());
    assert_eq!(source.page(), b"0123456789");
    assert_eq!(source.total(), 20);
}

#[test]
fn maximum_complete_frames_fit_the_derived_transport_ceiling() {
    let (c, b, a) = devices();
    let others = (0..3)
        .map(|_| MlsDevice::generate().unwrap())
        .collect::<Vec<_>>();
    let mut sources = others
        .iter()
        .map(MlsDevice::device_id)
        .chain([a.device_id()])
        .collect::<Vec<_>>();
    sources.sort();
    let group = [9; 256];
    let mut spec = fields(&b, &sources);
    spec.group = &group;
    spec.window.max_cipher_bytes = MAX_SOURCE_CHUNK as u32;
    spec.window.length = MAX_PAGE_BYTES as u32;
    let raw = spec.issue(&c).unwrap();
    assert_eq!(raw.len(), MAX_GRANT_BYTES);
    let grant = Grant::parse(&raw).unwrap();
    assert_eq!(
        Delivery::issue(&grant, a.device_id(), PeerId::from_u64(3), &b)
            .unwrap()
            .len(),
        MAX_DELIVERY_BYTES
    );
    let data = vec![3; MAX_PAGE_BYTES];
    let raw_page = SourcePage::issue(
        &grant,
        SourceStatus::Present,
        MAX_SOURCE_CHUNK as u32,
        &data,
        &a,
    )
    .unwrap();
    assert_eq!(raw_page.len(), MAX_SOURCE_REPLY_BYTES);
    let source = SourcePage::parse(&raw_page, &grant).unwrap();
    let raw_helper = HelperPage::issue(&grant, HelperStatus::Found, Some(&source), &b).unwrap();
    assert_eq!(raw_helper.len(), MAX_HELPER_REPLY_BYTES);
    let helper = HelperPage::parse(&raw_helper, &grant).unwrap();
    assert!(helper.verify_signature() && helper.source().unwrap().verify_signature());
}

#[test]
fn source_list_roles_and_window_bounds_are_closed_before_signing() {
    let (c, b, a) = devices();
    let d = MlsDevice::generate().unwrap();
    let mut pair = [a.device_id(), d.device_id()];
    pair.sort();
    let reversed = [pair[1], pair[0]];
    let duplicate = [a.device_id(); 2];
    let too_many = [a.device_id(); 5];
    for sources in [
        &[][..],
        &[b.device_id()][..],
        &[c.device_id()][..],
        &reversed,
        &duplicate,
        &too_many,
    ] {
        assert!(fields(&b, sources).issue(&c).is_err());
    }
    let sources = [a.device_id()];
    let mut spec = fields(&b, &sources);
    spec.helper = c.device_id();
    assert!(spec.issue(&c).is_err());
    spec.helper = b.device_id();
    spec.helper_peer = spec.requester_peer;
    assert!(spec.issue(&c).is_err());
    spec.helper_peer = PeerId::from_u64(2);
    for group in [&[][..], &[0; 257][..]] {
        spec.group = group;
        assert!(spec.issue(&c).is_err());
    }
    spec.group = b"full server group";
    let original = spec.window;
    for window in [
        Window {
            max_cipher_bytes: 0,
            ..original
        },
        Window {
            max_cipher_bytes: (MAX_SOURCE_CHUNK + 1) as u32,
            ..original
        },
        Window {
            offset: 100,
            ..original
        },
        Window {
            length: 0,
            ..original
        },
        Window {
            length: (MAX_PAGE_BYTES + 1) as u32,
            ..original
        },
    ] {
        spec.window = window;
        assert!(spec.issue(&c).is_err());
    }
}

#[test]
fn malformed_and_oversized_frames_are_rejected_without_crypto_or_trailing_bytes() {
    let (c, b, a) = devices();
    let sources = [a.device_id()];
    let raw = fields(&b, &sources).issue(&c).unwrap();
    for end in 0..raw.len() {
        assert!(Grant::parse(&raw[..end]).is_err());
    }
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(Grant::parse(&trailing).is_err());
    assert!(Grant::parse(&vec![FILE_BRIDGE_KIND; MAX_GRANT_BYTES + 1]).is_err());
    let grant = Grant::parse(&raw).unwrap();
    let delivery = Delivery::issue(&grant, a.device_id(), PeerId::from_u64(3), &b).unwrap();
    for end in 0..delivery.len() {
        assert!(Delivery::parse(&delivery[..end]).is_err());
    }
    assert!(Delivery::parse(&vec![FILE_SOURCE_KIND; MAX_DELIVERY_BYTES + 1]).is_err());
    assert!(SourcePage::parse(&vec![SOURCE_REPLY; MAX_SOURCE_REPLY_BYTES + 1], &grant).is_err());
    assert!(HelperPage::parse(&vec![HELPER_REPLY; MAX_HELPER_REPLY_BYTES + 1], &grant).is_err());
    let mut wrong_key = Encoder::new();
    wrong_key.put_u8(FILE_BRIDGE_KIND);
    field(&mut wrong_key, grant.signed.body);
    field(&mut wrong_key, &[0; 33]);
    field(&mut wrong_key, &[0; 64]);
    assert!(Grant::parse(&wrong_key.finish()).is_err());
}

#[test]
fn delivery_cannot_change_roles_targets_or_original_signature_domain() {
    let (c, b, a) = devices();
    let other = MlsDevice::generate().unwrap();
    let sources = [a.device_id()];
    let raw = fields(&b, &sources).issue(&c).unwrap();
    let grant = Grant::parse(&raw).unwrap();
    assert!(Delivery::issue(&grant, other.device_id(), PeerId::from_u64(3), &b).is_err());
    for peer in [grant.helper_peer(), grant.requester_peer()] {
        assert!(Delivery::issue(&grant, a.device_id(), peer, &b).is_err());
    }
    assert!(Delivery::issue(&grant, a.device_id(), PeerId::from_u64(3), &c).is_err());
    let wrong_domain = Signed::issue(
        FILE_BRIDGE_KIND,
        grant.signed.body,
        DELIVERY_DOMAIN,
        &c,
        MAX_GRANT_BYTES,
    )
    .unwrap();
    assert!(!Grant::parse(&wrong_domain).unwrap().verify_signature());
    let mut tampered = raw.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(
        !Grant::parse(&tampered).unwrap().verify_signature(),
        "structural parsing never grants authentication"
    );
}

#[test]
fn source_page_is_exact_for_requested_window_and_negative_markers_carry_no_bytes() {
    let (c, b, a) = devices();
    let sources = [a.device_id()];
    let mut spec = fields(&b, &sources);
    spec.window.offset = 15;
    let raw = spec.issue(&c).unwrap();
    let grant = Grant::parse(&raw).unwrap();
    assert!(SourcePage::issue(&grant, SourceStatus::Present, 20, b"12345", &a).is_ok());
    for (total, page) in [
        (20, b"1234".as_slice()),
        (20, b"123456".as_slice()),
        (15, b"".as_slice()),
        (101, b"1234567890".as_slice()),
    ] {
        assert!(SourcePage::issue(&grant, SourceStatus::Present, total, page, &a).is_err());
    }
    for status in [SourceStatus::Absent, SourceStatus::Unavailable] {
        let raw = SourcePage::issue(&grant, status, 0, &[], &a).unwrap();
        let parsed = SourcePage::parse(&raw, &grant).unwrap();
        assert!(parsed.verify_signature());
        assert!(SourcePage::issue(&grant, status, 1, &[], &a).is_err());
        assert!(SourcePage::issue(&grant, status, 0, b"hidden data", &a).is_err());
    }
    for wrong in [&b, &c] {
        assert!(SourcePage::issue(&grant, SourceStatus::Absent, 0, &[], wrong).is_err());
    }
}

#[test]
fn complete_grant_hash_rejects_cross_group_epoch_window_helper_and_nonce_responses() {
    let (c, b, a) = devices();
    let sources = [a.device_id()];
    let spec = fields(&b, &sources);
    let raw = spec.issue(&c).unwrap();
    let grant = Grant::parse(&raw).unwrap();
    let page = SourcePage::issue(&grant, SourceStatus::Present, 20, b"0123456789", &a).unwrap();
    let source = SourcePage::parse(&page, &grant).unwrap();
    let helper = HelperPage::issue(&grant, HelperStatus::Found, Some(&source), &b).unwrap();
    for change in 0..6 {
        let mut spec = fields(&b, &sources);
        match change {
            0 => spec.group = b"another group",
            1 => spec.epoch += 1,
            2 => spec.window.offset += 1,
            3 => spec.helper_peer = PeerId::from_u64(4),
            4 => spec.nonce[0] ^= 1,
            _ => spec.window.cid = Cid::of(b"another ciphertext"),
        }
        let raw = spec.issue(&c).unwrap();
        let other = Grant::parse(&raw).unwrap();
        assert_ne!(other.hash(), grant.hash());
        assert!(SourcePage::parse(&page, &other).is_err());
        assert!(HelperPage::parse(&helper, &other).is_err());
        assert!(HelperPage::issue(&other, HelperStatus::Found, Some(&source), &b).is_err());
    }
}

#[test]
fn helper_found_requires_exact_present_signature_and_negatives_cannot_smuggle_a_source() {
    let (c, b, a) = devices();
    let sources = [a.device_id()];
    let raw = fields(&b, &sources).issue(&c).unwrap();
    let grant = Grant::parse(&raw).unwrap();
    let page = SourcePage::issue(&grant, SourceStatus::Present, 20, b"0123456789", &a).unwrap();
    let source = SourcePage::parse(&page, &grant).unwrap();
    assert!(HelperPage::issue(&grant, HelperStatus::Found, None, &b).is_err());
    assert!(HelperPage::issue(&grant, HelperStatus::Absent, Some(&source), &b).is_err());
    assert!(HelperPage::issue(&grant, HelperStatus::Found, Some(&source), &c).is_err());
    let absent = SourcePage::issue(&grant, SourceStatus::Absent, 0, &[], &a).unwrap();
    let absent = SourcePage::parse(&absent, &grant).unwrap();
    assert!(HelperPage::issue(&grant, HelperStatus::Found, Some(&absent), &b).is_err());
    let wrong_domain = Signed::issue(
        SOURCE_REPLY,
        source.signed.body,
        HELPER_DOMAIN,
        &a,
        MAX_SOURCE_REPLY_BYTES,
    )
    .unwrap();
    let wrong = SourcePage::parse(&wrong_domain, &grant).unwrap();
    assert!(!wrong.verify_signature());
    assert!(HelperPage::issue(&grant, HelperStatus::Found, Some(&wrong), &b).is_err());
    for status in [HelperStatus::Absent, HelperStatus::Unavailable] {
        let raw = HelperPage::issue(&grant, status, None, &b).unwrap();
        let parsed = HelperPage::parse(&raw, &grant).unwrap();
        assert!(parsed.verify_signature());
        assert!(parsed.source().is_none());
        let mut body = Encoder::new();
        body.put_u8(1);
        field(&mut body, &grant.hash());
        body.put_u8(status as u8);
        field(&mut body, source.raw());
        let raw = Signed::issue(
            HELPER_REPLY,
            &body.finish(),
            HELPER_DOMAIN,
            &b,
            MAX_HELPER_REPLY_BYTES,
        )
        .unwrap();
        assert!(HelperPage::parse(&raw, &grant).is_err());
    }
}
