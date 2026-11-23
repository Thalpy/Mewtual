//! Immutable encrypted signed successors on the exact joined predecessor.
use super::*;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;

const FEE: usize = 8 * codec::MEMBER_HEADER_FRAME + 4096;

pub(super) struct HeaderWitness {
    path: PathBuf,
    raw: Zeroizing<Vec<u8>>,
    original: Option<CreatedStage>,
    header: OwnerHeaderId,
    covered: bool,
    _fee: CatalogReadReservation,
}

impl HeaderWitness {
    pub(super) fn require_current(&self) -> Result<(), AppError> {
        if !self.covered {
            return Err(denied());
        }
        exact_retained(
            &self.path,
            self.original.as_ref().ok_or_else(denied)?,
            &self.raw,
        )
    }

    fn cover(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if !self.covered {
            write_held(store, &self.path, &self.raw, &mut self.original)?;
            self.covered = true;
        }
        self.require_current()
    }
}

struct Scope<'a> {
    store: &'a ServerStore,
    key: &'a [u8; 32],
    context: V2PendingJoinContextData,
    epoch: u64,
    predecessor: &'a [u8],
}

impl Scope<'_> {
    fn path(&self, conflict: bool) -> Result<PathBuf, AppError> {
        let kind = if conflict { "conflict" } else { "first" };
        fixed(
            self.store,
            false,
            &format!(
                "chat-critical-namespaces.joined-member-header.{}.{}.{kind}.bin",
                hex::encode(self.context.original_pending),
                self.epoch
            ),
        )
    }

    fn fee(&self) -> Result<CatalogReadReservation, AppError> {
        self.store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .current
            .archive
            .pending
            .pool
            .try_reserve(FEE)
    }

    fn prepare(
        &self,
        path: PathBuf,
        header: &SignedOwnerHeader,
        conflict: bool,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<HeaderWitness, AppError> {
        let fee = self.fee()?;
        let raw = codec::seal_member_header(
            self.key,
            self.context,
            self.predecessor,
            (self.epoch, conflict),
            header,
            rng,
        )?;
        Ok(HeaderWitness {
            path,
            raw,
            original: None,
            header: header.id(),
            covered: false,
            _fee: fee,
        })
    }

    fn read(
        &self,
        path: PathBuf,
        conflict: bool,
    ) -> Result<(HeaderWitness, SignedOwnerHeader), AppError> {
        let fee = self.fee()?;
        let (file, raw) = read(&path, codec::MEMBER_HEADER_FRAME)?.ok_or_else(denied)?;
        let identity = session_id(&file).map_err(io)?;
        let header = codec::open_member_header(
            self.key,
            self.context,
            self.predecessor,
            (self.epoch, conflict),
            &raw,
        )?;
        let witness = HeaderWitness {
            path,
            raw,
            original: Some(CreatedStage {
                file,
                identity: Some(identity),
            }),
            header: header.id(),
            covered: true,
            _fee: fee,
        };
        witness.require_current()?;
        Ok((witness, header))
    }
}

pub(super) fn observe_live<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
    store: &ServerStore,
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    (epoch, predecessor): (u64, &[u8]),
    (first, fork): (&mut Option<HeaderWitness>, &mut Option<HeaderWitness>),
    (core, node): (&mut PreparedV2JoinedPrefix, &ClosedV2JoinedNode<T, R>),
    header: &SignedOwnerHeader,
    require_live: &impl Fn() -> Result<(), AppError>,
    rng: &mut impl catcoms_rt::CryptoRngCore,
) -> Result<(), AppError> {
    let scope = Scope {
        store,
        key,
        context,
        epoch,
        predecessor,
    };
    let conflict_path = scope.path(true)?;
    absent(&conflict_path)?;
    if let Some(original) = first {
        require_live()?;
        original.cover(store)?;
        if original.header != header.id() {
            node.verify_member_successor_header(core, header)?;
            let saved = (|| {
                require_live()?;
                let witness = scope.prepare(conflict_path, header, true, rng)?;
                *fork = Some(witness);
                fork.as_mut().ok_or_else(denied)?.cover(store)?;
                store.validate_mounted_governance_layout()
            })();
            // A failed write must still close the live view. A covered witness
            // also refuses cold activation if we stop before this call.
            let closed = node.observe_member_successor_header(core, header);
            saved?;
            let pair = core
                .member_successor_contradiction_data()
                .ok_or_else(denied)?;
            if pair.0.id() != original.header || pair.1.id() != header.id() {
                return Err(denied());
            }
            closed?;
            return Err(denied());
        }
        node.observe_member_successor_header(core, header)?;
    } else {
        let path = scope.path(false)?;
        absent(&path)?;
        node.verify_member_successor_header(core, header)?;
        let witness = scope.prepare(path, header, false, rng)?;
        node.observe_member_successor_header(core, header)?;
        *first = Some(witness);
        require_live()?;
        first.as_mut().ok_or_else(denied)?.cover(store)?;
    }
    require_live()
}

pub(super) fn recover<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
    store: &ServerStore,
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    (epoch, predecessor): (u64, &[u8]),
    (core, node): (&mut PreparedV2JoinedPrefix, &ClosedV2JoinedNode<T, R>),
    unavailable: bool,
) -> Result<Option<HeaderWitness>, AppError> {
    let scope = Scope {
        store,
        key,
        context,
        epoch,
        predecessor,
    };
    let first = scope.path(false)?;
    let conflict = scope.path(true)?;
    if absent(&first).is_ok() {
        absent(&conflict)?;
        return Ok(None);
    }
    if unavailable {
        return Err(denied());
    }
    let (witness, header) = scope.read(first, false)?;
    node.observe_member_successor_header(core, &header)?;
    if absent(&conflict).is_err() {
        let (_, other) = scope.read(conflict, true)?;
        node.observe_member_successor_header(core, &other)?;
        return Err(denied());
    }
    witness.require_current()?;
    Ok(Some(witness))
}
