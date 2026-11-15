//! Closed dependency of native activation on its exact original network file.
//! It supplies no SDK/source/native permission and contains no owner backref.
use super::*;
use catcoms_wire::Decoder;
use std::sync::Arc;

const TAG: &[u8] = b"catcoms/native-founder-network/v1\0";
// Raw frame, ciphertext/unseal, bounded reread plus controls/handles/Arc. Each
// cold item pins the actual original handler invoice, which prepays all rows
// plus one temporary item; its fee-only tail cannot cycle to a cold cell.
pub(in crate::store) const COLD_FEE: usize = 4 * MAX_FRAME + 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct NetworkExpected {
    pub(in crate::store) row: u64,
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) root: [u8; 32],
    pub(in crate::store) identity: [u8; 32],
    pub(in crate::store) invocation: [u8; 32],
    pub(in crate::store) request: [u8; 32],
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) network: [u8; 32],
}
pub(in crate::store) struct NetworkDependency {
    expected: NetworkExpected,
    output: HeldNativeInput,
    servers: HeldDirectory,
    vault: HeldDirectory,
    // LAST: exact files, bounded bytes and controls precede their real payer.
    _payment: Arc<dyn Send + Sync>,
}
impl NetworkDependency {
    pub(super) fn from_original(
        expected: NetworkExpected,
        output: HeldNativeInput,
        servers: HeldDirectory,
        vault: HeldDirectory,
        payment: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            expected,
            output,
            servers,
            vault,
            _payment: payment,
        }
    }
    pub(in crate::store) fn matches(
        &self,
        expected: NetworkExpected,
        frame: Option<&[u8]>,
    ) -> bool {
        self.expected == expected && frame.is_none_or(|frame| frame == self.output.raw())
    }
    pub(in crate::store) fn capture_cold(
        store: &ServerStore,
        expected: NetworkExpected,
        frame: Option<&[u8]>,
        payment: Arc<dyn Send + Sync>,
    ) -> Result<Arc<Self>, AppError> {
        let file = path(
            store,
            false,
            &format!("servers/{}.native-net-v1", expected.row),
        )?;
        let value = Self::from_original(
            expected,
            HeldNativeInput::capture(&file, MAX_FRAME)?.ok_or_else(denied)?,
            HeldDirectory::capture(&path(store, false, "servers")?)?,
            HeldDirectory::capture(&path(store, false, "")?)?,
            payment,
        );
        if frame.is_some_and(|frame| frame != value.output.raw()) {
            return Err(denied());
        }
        value.require(store)?;
        Ok(Arc::new(value))
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        if self.expected.row == 0 {
            return Err(denied());
        }
        self.vault.require(&path(store, false, "")?)?;
        self.servers.require(&path(store, false, "servers")?)?;
        let file = path(
            store,
            false,
            &format!("servers/{}.native-net-v1", self.expected.row),
        )?;
        self.output.require(&file)?;
        validate_frame(store, self.expected, self.output.raw())?;
        self.output.file.sync_all().map_err(io)?;
        self.servers.file.sync_all().map_err(io)?;
        self.vault.file.sync_all().map_err(io)?;
        self.output.require(&file)?;
        self.servers.require(&path(store, false, "servers")?)?;
        self.vault.require(&path(store, false, "")?)
    }
}

pub(in crate::store) fn validate_frame(
    store: &ServerStore,
    expected: NetworkExpected,
    frame: &[u8],
) -> Result<(), AppError> {
    if !(40..=MAX_FRAME).contains(&frame.len()) {
        return Err(denied());
    }
    let key = Zeroizing::new(store.keys.db_key()?);
    let plain = Zeroizing::new(catcoms_crypto::unseal(
        &key,
        &crate::store::unframe(frame)?,
    )?);
    let mut input = Decoder::new(&plain);
    if input.get_bytes().map_err(|_| denied())? != TAG
        || input.get_u16().map_err(|_| denied())? != 1
    {
        return Err(denied());
    }
    for field in [
        expected.vault,
        expected.root,
        expected.invocation,
        expected.request,
        expected.signer,
        expected.identity,
        expected.network,
    ] {
        if input.get_bytes().map_err(|_| denied())? != field {
            return Err(denied());
        }
    }
    let public = input.get_bytes().map_err(|_| denied())?;
    if public.len() != 36 {
        return Err(denied());
    }
    let seed = Zeroizing::new(
        input
            .get_bytes()
            .map_err(|_| denied())?
            .try_into()
            .map_err(|_| denied())?,
    );
    let actual = catcoms_net::seeded_transport_identity_data(&seed).map_err(|_| denied())?;
    if actual.public_key_data().as_slice() != public
        || actual.commitment_data() != expected.network
        || input.get_u16().map_err(|_| denied())?
            != crate::store::ServerNet::derived_port_for_seed(&seed)
        || input.get_u64().map_err(|_| denied())? != 65_536
    {
        return Err(denied());
    }
    for _ in 0..3 {
        if input.get_str().map_err(|_| denied())?.len() > 4096 {
            return Err(denied());
        }
    }
    input.finish().map_err(|_| denied())
}
