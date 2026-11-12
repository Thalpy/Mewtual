//! Original A locator and ciphertext chunk DATA. A Retain record selects exact original
//! Action/envelope pages; it does not accept/publish the action or admit a selector write.
use super::common::*;
use super::request::{TextRequestDigestData, TextRetainIdData};
use crate::AppError;
use catcoms_replication::chat::cold_actions::{
    OriginalActionDigest, ACTION_CHUNK_BYTES, MAX_ACTION_CHUNK_PAGE_BYTES,
};
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const RETAIN_TAG: &[u8] = b"catcoms/private-chat-text-retain/v1\0";
const CHUNK_TAG: &[u8] = b"catcoms/private-chat-text-cipher-chunk/v1\0";
const MAX_ACTION: usize = 66057;
pub(in crate::store) const MAX_ENCRYPTED_PAYLOAD_BYTES: usize = 131116;
pub(in crate::store) const MAX_RETAIN_PLAIN: usize = 520;
pub(in crate::store) const CIPHER_CHUNK_HEADER_BYTES: usize = 168;
pub(in crate::store) const MAX_CIPHER_CHUNK_PLAIN: usize =
    CIPHER_CHUNK_HEADER_BYTES + ACTION_CHUNK_BYTES;
const _: () = assert!(
    4 + RETAIN_TAG.len() + 2 + 7 * 36 + 8 + 16 + 1 + 2 * 40 + 1 + 3 * 40 == MAX_RETAIN_PLAIN
);
const _: () =
    assert!(4 + CHUNK_TAG.len() + 2 + 3 * 36 + 4 + 2 + 2 + 4 == CIPHER_CHUNK_HEADER_BYTES);
data_id!(
    TextCipherOriginalDigestData,
    b"catcoms/private-chat-text-cipher-original/v1\0"
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct TextChunkDescriptorData {
    pub(in crate::store) page: PrivateChatPageId,
    pub(in crate::store) plain_bytes: u32,
}

/// Canonical labels/descriptors only; real Root selects actual source/action originals.
pub(in crate::store) struct RetainedActionSpecData<'a> {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) request: TextRequestDigestData,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) original: OriginalActionDigest,
    pub(in crate::store) cipher: TextCipherOriginalDigestData,
    pub(in crate::store) action_bytes: u32,
    pub(in crate::store) cipher_bytes: u32,
    pub(in crate::store) channel: u128,
    pub(in crate::store) actions: &'a [TextChunkDescriptorData],
    pub(in crate::store) ciphers: &'a [TextChunkDescriptorData],
}
impl std::fmt::Debug for RetainedActionSpecData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedActionSpecData")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for TextChunkDescriptorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextChunkDescriptorData")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct RetainedActionData {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) request: TextRequestDigestData,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) original: OriginalActionDigest,
    pub(in crate::store) cipher: TextCipherOriginalDigestData,
    pub(in crate::store) action_bytes: u32,
    pub(in crate::store) cipher_bytes: u32,
    pub(in crate::store) channel: u128,
    actions: Vec<TextChunkDescriptorData>,
    ciphers: Vec<TextChunkDescriptorData>,
}
impl std::fmt::Debug for RetainedActionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedActionData").finish_non_exhaustive()
    }
}
impl RetainedActionData {
    pub(in crate::store) fn descriptors_data(
        &self,
    ) -> (&[TextChunkDescriptorData], &[TextChunkDescriptorData]) {
        (&self.actions, &self.ciphers)
    }
    fn validate(&self) -> Result<(), AppError> {
        validate_descriptors(
            self.action_bytes as usize,
            self.cipher_bytes as usize,
            &self.actions,
            &self.ciphers,
        )
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate()?;
        let mut e = Encoder::with_capacity(MAX_RETAIN_PLAIN);
        tag(&mut e, RETAIN_TAG)?;
        for b in [
            self.context.as_bytes(),
            self.transaction.as_bytes(),
            &self.token,
            self.request.as_bytes(),
            self.action.as_bytes(),
            self.original.as_bytes(),
            self.cipher.as_bytes(),
        ] {
            put(&mut e, b)?;
        }
        e.put_u32(self.action_bytes)
            .put_u32(self.cipher_bytes)
            .put_u128(self.channel);
        encode_descriptors(&mut e, &self.actions)?;
        encode_descriptors(&mut e, &self.ciphers)?;
        finish(e, MAX_RETAIN_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_RETAIN_PLAIN, RETAIN_TAG)?;
        let context = TextContextIdData::from_bytes(b32(&mut d)?);
        let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
        let token = b32(&mut d)?;
        let request = TextRequestDigestData::from_bytes(b32(&mut d)?);
        let action = ActionId::from_bytes(b32(&mut d)?);
        let original = OriginalActionDigest::from_bytes(b32(&mut d)?);
        let cipher = TextCipherOriginalDigestData::from_bytes(b32(&mut d)?);
        let action_bytes = d.get_u32().map_err(|_| refused())?;
        let cipher_bytes = d.get_u32().map_err(|_| refused())?;
        let channel = d.get_u128().map_err(|_| refused())?;
        let (actions, na) = decode_descriptors::<2>(&mut d)?;
        let (ciphers, nc) = decode_descriptors::<3>(&mut d)?;
        d.finish().map_err(|_| refused())?;
        validate_descriptors(
            action_bytes as usize,
            cipher_bytes as usize,
            &actions[..na],
            &ciphers[..nc],
        )?;
        // All framing/EOF/geometry precede these two bounded metadata copies.
        Ok(Self {
            context,
            transaction,
            token,
            request,
            action,
            original,
            cipher,
            action_bytes,
            cipher_bytes,
            channel,
            actions: actions[..na].to_vec(),
            ciphers: ciphers[..nc].to_vec(),
        })
    }
    pub(in crate::store) fn from_data(fixed: RetainedActionSpecData<'_>) -> Result<Self, AppError> {
        validate_descriptors(
            fixed.action_bytes as usize,
            fixed.cipher_bytes as usize,
            fixed.actions,
            fixed.ciphers,
        )?;
        Ok(Self {
            context: fixed.context,
            transaction: fixed.transaction,
            token: fixed.token,
            request: fixed.request,
            action: fixed.action,
            original: fixed.original,
            cipher: fixed.cipher,
            action_bytes: fixed.action_bytes,
            cipher_bytes: fixed.cipher_bytes,
            channel: fixed.channel,
            actions: fixed.actions.to_vec(),
            ciphers: fixed.ciphers.to_vec(),
        })
    }
    pub(in crate::store) fn frame_id_data(raw: &[u8]) -> Result<TextRetainIdData, AppError> {
        if raw.len() <= 40 || raw.len() > 1024 + 40 {
            return Err(refused());
        }
        Ok(TextRetainIdData::of_data(raw))
    }
}
fn validate_descriptors(
    action_bytes: usize,
    cipher_bytes: usize,
    actions: &[TextChunkDescriptorData],
    ciphers: &[TextChunkDescriptorData],
) -> Result<(), AppError> {
    if action_bytes == 0
        || action_bytes > MAX_ACTION
        || !(41..=MAX_ENCRYPTED_PAYLOAD_BYTES).contains(&cipher_bytes)
        || actions.len() != action_bytes.div_ceil(ACTION_CHUNK_BYTES)
        || ciphers.len() != cipher_bytes.div_ceil(ACTION_CHUNK_BYTES)
        || actions.len() > 2
        || ciphers.len() > 3
    {
        return Err(refused());
    }
    for descriptor in actions {
        if descriptor.plain_bytes == 0
            || descriptor.plain_bytes as usize > MAX_ACTION_CHUNK_PAGE_BYTES
        {
            return Err(refused());
        }
    }
    for (i, descriptor) in ciphers.iter().enumerate() {
        let payload = (cipher_bytes - i * ACTION_CHUNK_BYTES).min(ACTION_CHUNK_BYTES);
        if descriptor.plain_bytes as usize != CIPHER_CHUNK_HEADER_BYTES + payload {
            return Err(refused());
        }
    }
    for (i, descriptor) in actions.iter().chain(ciphers.iter()).enumerate() {
        if actions
            .iter()
            .chain(ciphers.iter())
            .take(i)
            .any(|old| old.page == descriptor.page)
        {
            return Err(refused());
        }
    }
    Ok(())
}
fn encode_descriptors(
    e: &mut Encoder,
    descriptors: &[TextChunkDescriptorData],
) -> Result<(), AppError> {
    e.put_u8(descriptors.len() as u8);
    for d in descriptors {
        put(e, d.page.as_bytes())?;
        e.put_u32(d.plain_bytes);
    }
    Ok(())
}
fn decode_descriptors<const N: usize>(
    d: &mut Decoder<'_>,
) -> Result<([TextChunkDescriptorData; N], usize), AppError> {
    let count = usize::from(d.get_u8().map_err(|_| refused())?);
    if count == 0 || count > N {
        return Err(refused());
    }
    let mut descriptors = [TextChunkDescriptorData {
        page: PrivateChatPageId::from_bytes([0; 32]),
        plain_bytes: 0,
    }; N];
    for value in &mut descriptors[..count] {
        *value = TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes(b32(d)?),
            plain_bytes: d.get_u32().map_err(|_| refused())?,
        };
    }
    Ok((descriptors, count))
}

/// Borrowed exact cipher page. Chunk equality/digest alone is not MLS/key/acceptance evidence.
pub(in crate::store) struct CipherChunkData<'a> {
    context: TextContextIdData,
    transaction: TextTransactionIdData,
    original: TextCipherOriginalDigestData,
    total: usize,
    index: usize,
    count: usize,
    payload: &'a [u8],
}
impl std::fmt::Debug for CipherChunkData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CipherChunkData").finish_non_exhaustive()
    }
}
impl<'a> CipherChunkData<'a> {
    pub(in crate::store) fn encode_data(
        context: TextContextIdData,
        transaction: TextTransactionIdData,
        nonce: &[u8; 24],
        cipher: &[u8],
        index: usize,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let total = 24usize.checked_add(cipher.len()).ok_or_else(refused)?;
        if cipher.len() < 17 || total > MAX_ENCRYPTED_PAYLOAD_BYTES {
            return Err(refused());
        }
        let count = total.div_ceil(ACTION_CHUNK_BYTES);
        if index >= count {
            return Err(refused());
        }
        let original = cipher_digest(nonce, cipher)?;
        let offset = index * ACTION_CHUNK_BYTES;
        let payload = (total - offset).min(ACTION_CHUNK_BYTES);
        let mut e = Encoder::with_capacity(MAX_CIPHER_CHUNK_PLAIN);
        tag(&mut e, CHUNK_TAG)?;
        put(&mut e, context.as_bytes())?;
        put(&mut e, transaction.as_bytes())?;
        put(&mut e, original.as_bytes())?;
        e.put_u32(total as u32)
            .put_u16(index as u16)
            .put_u16(count as u16)
            .put_u32(payload as u32);
        // Encode the nonce/cipher join without allocating another whole131116B envelope.
        for at in offset..offset + payload {
            e.put_u8(if at < 24 { nonce[at] } else { cipher[at - 24] });
        }
        finish(e, MAX_CIPHER_CHUNK_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &'a [u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_CIPHER_CHUNK_PLAIN, CHUNK_TAG)?;
        let context = TextContextIdData::from_bytes(b32(&mut d)?);
        let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
        let original = TextCipherOriginalDigestData::from_bytes(b32(&mut d)?);
        let total = d.get_u32().map_err(|_| refused())? as usize;
        let index = usize::from(d.get_u16().map_err(|_| refused())?);
        let count = usize::from(d.get_u16().map_err(|_| refused())?);
        let payload = field(&mut d, ACTION_CHUNK_BYTES)?;
        d.finish().map_err(|_| refused())?;
        if !(41..=MAX_ENCRYPTED_PAYLOAD_BYTES).contains(&total)
            || count != total.div_ceil(ACTION_CHUNK_BYTES)
            || index >= count
            || payload.len() != (total - index * ACTION_CHUNK_BYTES).min(ACTION_CHUNK_BYTES)
        {
            return Err(refused());
        }
        Ok(Self {
            context,
            transaction,
            original,
            total,
            index,
            count,
            payload,
        })
    }
    pub(in crate::store) fn require_descriptor_data(
        &self,
        retained: &RetainedActionData,
        index: usize,
    ) -> Result<(), AppError> {
        if self.context != retained.context
            || self.transaction != retained.transaction
            || self.original != retained.cipher
            || self.total != retained.cipher_bytes as usize
            || self.index != index
            || self.count != retained.ciphers.len()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn payload_data(&self) -> &'a [u8] {
        self.payload
    }
}
pub(in crate::store) fn cipher_digest(
    nonce: &[u8; 24],
    cipher: &[u8],
) -> Result<TextCipherOriginalDigestData, AppError> {
    if cipher.len() < 17 || cipher.len() > MAX_ENCRYPTED_PAYLOAD_BYTES - 24 {
        return Err(refused());
    }
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/private-chat-text-cipher-original/v1\0");
    h.update(&((24 + cipher.len()) as u32).to_be_bytes());
    h.update(nonce);
    h.update(cipher);
    Ok(TextCipherOriginalDigestData::from_bytes(
        *h.finalize().as_bytes(),
    ))
}

#[cfg(test)]
mod tests;
