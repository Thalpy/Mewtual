//! Bounded Head and exact admitted output framing. Full successor
//! validation rederives the COW delta; decoding supplies no authority.
use super::*;
const HEAD_TAG: &[u8] = b"catcoms/private-chat-indexed-history-head/v1\0";
const HEAD_KEY: &str = "catcoms/private-chat-indexed-history-head-key/v1";
const WAL_TAG: &[u8] = b"catcoms/private-chat-indexed-history-wal/v1\0";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Head {
    pub(super) scope: Option<[u8; 32]>,
    pub(super) generation: u64,
    pub(super) count: u64,
    /// Exact retained immutable bundle and currently selected index bytes.
    /// Replaced pages remain in the bounded transaction workspace until
    /// their exact retirement; they are never silently refunded on failure.
    pub(super) bytes: u64,
    pub(super) previous: [u8; 32],
    pub(super) transaction: [u8; 32],
    pub(super) root: Option<NodeRefData>,
}
impl Head {
    pub(super) fn empty() -> Self {
        Self {
            scope: None,
            generation: 0,
            count: 0,
            bytes: 0,
            previous: [0; 32],
            transaction: [0; 32],
            root: None,
        }
    }
    pub(super) fn require_scope(self, scope: SharedHistoryScope) -> Result<(), AppError> {
        if self.scope.is_some_and(|id| id != scope.id()) {
            return Err(refused());
        }
        self.shape()
    }
    fn shape(self) -> Result<(), AppError> {
        match self.root {
            None if self != Self::empty() => Err(refused()),
            Some(root)
                if self.scope.is_none()
                    || self.generation != self.count
                    || root.summary.keys != self.count
                    || self.count == 0
                    || self.bytes == 0
                    || self.previous == [0; 32]
                    || self.transaction == [0; 32] =>
            {
                Err(refused())
            }
            _ => Ok(()),
        }
    }
    pub(super) fn seal(
        self,
        key: &[u8; 32],
        layout: IndexedHistoryLayout,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.shape()?;
        let mut e = Encoder::with_capacity(INDEXED_HEAD_CAP);
        wire(e.put_bytes(HEAD_TAG))?.put_u16(1);
        wire(e.put_bytes(&layout.0))?;
        e.put_u8(u8::from(self.scope.is_some()));
        if let Some(scope) = self.scope {
            wire(e.put_bytes(&scope))?;
        }
        e.put_u64(self.generation)
            .put_u64(self.count)
            .put_u64(self.bytes);
        wire(e.put_bytes(&self.previous))?;
        wire(e.put_bytes(&self.transaction))?;
        self.root
            .map_or(TextRootData::Empty, TextRootData::Present)
            .encode(&mut e)?;
        let derived = head_key(key, layout);
        let bytes = Zeroizing::new(frame(&catcoms_crypto::seal(
            &derived,
            &Zeroizing::new(e.finish()),
            rng,
        )?));
        if bytes.len() > INDEXED_HEAD_CAP {
            return Err(refused());
        }
        Ok(bytes)
    }
    pub(super) fn open(
        key: &[u8; 32],
        layout: IndexedHistoryLayout,
        bytes: &[u8],
    ) -> Result<Self, AppError> {
        if bytes.len() > INDEXED_HEAD_CAP {
            return Err(refused());
        }
        let plain = Zeroizing::new(catcoms_crypto::unseal(
            &head_key(key, layout),
            &unframe(bytes)?,
        )?);
        let mut d = Decoder::new(&plain);
        if wire(d.get_bytes())? != HEAD_TAG
            || wire(d.get_u16())? != 1
            || digest(&mut d)? != layout.0
        {
            return Err(refused());
        }
        let scope = match wire(d.get_u8())? {
            0 => None,
            1 => Some(digest(&mut d)?),
            _ => return Err(refused()),
        };
        let generation = wire(d.get_u64())?;
        let count = wire(d.get_u64())?;
        let bytes = wire(d.get_u64())?;
        let previous = digest(&mut d)?;
        let transaction = digest(&mut d)?;
        let root = match TextRootData::decode(&mut d)? {
            TextRootData::Empty => None,
            TextRootData::Present(root) => Some(root),
        };
        wire(d.finish())?;
        let head = Self {
            scope,
            generation,
            count,
            bytes,
            previous,
            transaction,
            root,
        };
        head.shape()?;
        Ok(head)
    }
}
fn head_key(key: &[u8; 32], layout: IndexedHistoryLayout) -> Zeroizing<[u8; 32]> {
    let family = Zeroizing::new(blake3::derive_key(HEAD_KEY, key));
    Zeroizing::new(*blake3::keyed_hash(&family, &layout.0).as_bytes())
}

pub(super) struct Wal<'a> {
    pub(super) old: &'a [u8],
    pub(super) next: &'a [u8],
    pub(super) record: SelectedHistoryRecord,
    pub(super) bundle: &'a [u8],
    pub(super) original: Vec<(NodeRefData, usize)>,
    pub(super) outputs: Vec<(NodeRefData, &'a [u8])>,
}
impl<'a> Wal<'a> {
    pub(super) fn encode(
        old: &[u8],
        next: &[u8],
        record: SelectedHistoryRecord,
        bundle: &[u8],
        original: &[(NodeRefData, usize)],
        outputs: &[(NodeRefData, PreparedPrivateChatPage)],
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if old.len() > INDEXED_HEAD_CAP
            || next.len() > INDEXED_HEAD_CAP
            || bundle.len() > MAX_SHARED_HISTORY_BUNDLE
            || !(1..=MAX_OUTPUTS).contains(&outputs.len())
        {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(WAL_CAP);
        wire(e.put_bytes(WAL_TAG))?.put_u16(1);
        for field in [
            old,
            next,
            record.action.as_bytes(),
            record.manifest.as_bytes(),
            bundle,
        ] {
            wire(e.put_bytes(field))?;
        }
        if original.len() > MAX_PATH {
            return Err(refused());
        }
        e.put_u8(original.len() as u8);
        for (node, bytes) in original {
            TextRootData::Present(*node).encode(&mut e)?;
            e.put_u64(*bytes as u64);
        }
        e.put_u8(outputs.len() as u8);
        for (node, page) in outputs {
            TextRootData::Present(*node).encode(&mut e)?;
            wire(e.put_bytes(page.as_bytes()))?;
        }
        if e.len() > WAL_CAP {
            return Err(refused());
        }
        Ok(Zeroizing::new(e.finish()))
    }
    pub(super) fn decode(bytes: &'a [u8]) -> Result<Self, AppError> {
        if bytes.len() > WAL_CAP {
            return Err(refused());
        }
        let mut d = Decoder::new(bytes);
        if wire(d.get_bytes())? != WAL_TAG || wire(d.get_u16())? != 1 {
            return Err(refused());
        }
        let old = wire(d.get_bytes())?;
        let next = wire(d.get_bytes())?;
        let record = SelectedHistoryRecord {
            action: ActionId::from_bytes(digest(&mut d)?),
            manifest: PrivateChatPageId::from_bytes(digest(&mut d)?),
        };
        let bundle = wire(d.get_bytes())?;
        if old.is_empty()
            || old.len() > INDEXED_HEAD_CAP
            || next.is_empty()
            || next.len() > INDEXED_HEAD_CAP
            || bundle.is_empty()
            || bundle.len() > MAX_SHARED_HISTORY_BUNDLE
        {
            return Err(refused());
        }
        let n = usize::from(wire(d.get_u8())?);
        if n > MAX_PATH {
            return Err(refused());
        }
        let mut original = Vec::with_capacity(n);
        for _ in 0..n {
            let TextRootData::Present(node) = TextRootData::decode(&mut d)? else {
                return Err(refused());
            };
            let size = usize::try_from(wire(d.get_u64())?).map_err(|_| refused())?;
            if size == 0
                || size > MAX_INDEX_FRAME_BYTES
                || original
                    .iter()
                    .any(|(old, _): &(NodeRefData, usize)| old.page == node.page)
            {
                return Err(refused());
            }
            original.push((node, size));
        }
        let n = usize::from(wire(d.get_u8())?);
        if !(1..=MAX_OUTPUTS).contains(&n) {
            return Err(refused());
        }
        let mut outputs = Vec::with_capacity(n);
        for _ in 0..n {
            let TextRootData::Present(node) = TextRootData::decode(&mut d)? else {
                return Err(refused());
            };
            let frame = wire(d.get_bytes())?;
            if frame.is_empty()
                || frame.len() > MAX_INDEX_FRAME_BYTES
                || original.iter().any(|(old, _)| old.page == node.page)
                || outputs
                    .iter()
                    .any(|(old, _): &(NodeRefData, &[u8])| old.page == node.page)
            {
                return Err(refused());
            }
            outputs.push((node, frame));
        }
        wire(d.finish())?;
        Ok(Self {
            old,
            next,
            record,
            bundle,
            original,
            outputs,
        })
    }

    /// Authenticated by the candidate Head. Excluding only that Head avoids
    /// a circular digest while binding every byte used for output and GC.
    pub(super) fn commitment(
        old: &[u8],
        record: SelectedHistoryRecord,
        bundle: &[u8],
        original: &[(NodeRefData, usize)],
        outputs: &[(NodeRefData, &[u8])],
    ) -> Result<[u8; 32], AppError> {
        let mut e = Encoder::with_capacity(WAL_CAP);
        for field in [
            WAL_TAG,
            old,
            record.action.as_bytes(),
            record.manifest.as_bytes(),
            bundle,
        ] {
            wire(e.put_bytes(field))?;
        }
        if original.len() > MAX_PATH || !(1..=MAX_OUTPUTS).contains(&outputs.len()) {
            return Err(refused());
        }
        e.put_u8(original.len() as u8);
        for (node, size) in original {
            TextRootData::Present(*node).encode(&mut e)?;
            e.put_u64(*size as u64);
        }
        e.put_u8(outputs.len() as u8);
        for (node, bytes) in outputs {
            TextRootData::Present(*node).encode(&mut e)?;
            wire(e.put_bytes(bytes))?;
        }
        if e.len() > WAL_CAP {
            return Err(refused());
        }
        Ok(*blake3::hash(&Zeroizing::new(e.finish())).as_bytes())
    }
}
