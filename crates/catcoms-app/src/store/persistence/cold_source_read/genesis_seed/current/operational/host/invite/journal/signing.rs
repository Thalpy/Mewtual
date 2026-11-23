//! Immutable KeyAttempt and Raw siblings of the covered unsigned intent.
//! A fresh warm owner never adopts a signing file, even if its bytes match.
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;

pub(super) const ATTEMPT: &str = "invite.attempt.v1";
pub(super) const RAW: &str = "invite.raw.v1";
const SIGNING_TAG: &[u8] = b"mewtual/private-chat/invite-signing/v1\0";
const SIGNING_KEY: &str = "mewtual/private-chat/invite-signing-key/v1";
const OBJECTS: u32 = 2;
pub(super) const FRAME_BYTES: usize = 512;
const BYTES: u32 = (OBJECTS as usize * FRAME_BYTES) as u32;
pub(in super::super) const SIGNING_JOURNAL_BYTES: usize = 8 * FRAME_BYTES + 4 * MAX_PATH + 4096;

#[derive(Default)]
pub(in super::super) struct InviteSigningJournal {
    attempt: JournalFile,
    raw: JournalFile,
    attempt_covered: bool,
    raw_covered: bool,
    unexpected_raw: bool,
}

fn key(store: &crate::store::ServerStore) -> Result<Zeroizing<[u8; 32]>, AppError> {
    let db_key = Zeroizing::new(store.keys.db_key()?);
    Ok(Zeroizing::new(blake3::derive_key(
        SIGNING_KEY,
        db_key.as_ref(),
    )))
}

/// Authenticate one exact linked stage. An absent attempt commitment selects
/// the KeyAttempt grammar; a present commitment selects only the Raw grammar.
pub(super) fn open_stage(
    store: &crate::store::ServerStore,
    frame: &[u8],
    context: [u8; 32],
    intent: [u8; 32],
    attempt: Option<[u8; 32]>,
) -> Result<Option<[u8; 64]>, AppError> {
    if frame.len() > FRAME_BYTES {
        return Err(refused());
    }
    let key = key(store)?;
    let plain = Zeroizing::new(unseal(&key, &crate::store::unframe(frame)?)?);
    let mut d = Decoder::new(&plain);
    if field(&mut d, SIGNING_TAG.len())? != SIGNING_TAG
        || d.get_u16().map_err(|_| refused())? != 1
        || d.get_u32().map_err(|_| refused())? != OBJECTS
        || d.get_u32().map_err(|_| refused())? != BYTES
        || field(&mut d, 32)? != context
        || field(&mut d, 32)? != intent
    {
        return Err(refused());
    }
    let kind = d.get_u8().map_err(|_| refused())?;
    let raw = match attempt {
        None if kind == 1 => None,
        Some(parent) if kind == 2 && field(&mut d, 32)? == parent => {
            Some(field(&mut d, 64)?.try_into().map_err(|_| refused())?)
        }
        _ => return Err(refused()),
    };
    d.finish().map_err(|_| refused())?;
    Ok(raw)
}

fn seal_stage(
    store: &crate::store::ServerStore,
    context: [u8; 32],
    intent: [u8; 32],
    raw: Option<([u8; 32], &[u8; 64])>,
    rng: &mut impl catcoms_rt::CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut e = Encoder::with_capacity(FRAME_BYTES);
    e.put_bytes(SIGNING_TAG).map_err(|_| refused())?;
    e.put_u16(1);
    e.put_u32(OBJECTS);
    e.put_u32(BYTES);
    e.put_bytes(&context).map_err(|_| refused())?;
    e.put_bytes(&intent).map_err(|_| refused())?;
    if let Some((attempt, signature)) = raw {
        e.put_u8(2);
        e.put_bytes(&attempt).map_err(|_| refused())?;
        e.put_bytes(signature).map_err(|_| refused())?;
    } else {
        e.put_u8(1);
    }
    let plain = Zeroizing::new(e.finish());
    let key = key(store)?;
    let frame = Zeroizing::new(crate::store::frame(&seal(&key, &plain, rng)?));
    if frame.len() > FRAME_BYTES {
        return Err(refused());
    }
    Ok(frame)
}

impl InviteSigningJournal {
    pub(in super::super) fn prepare_attempt(
        &mut self,
        intent: &InviteIntentJournal,
        store: &crate::store::ServerStore,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.attempt.frame.is_none() {
            self.attempt.frame = Some(seal_stage(
                store,
                intent.context.ok_or_else(refused)?,
                intent.object.commitment()?,
                None,
                rng,
            )?);
        }
        Ok(())
    }

    fn require_raw_absent(&mut self, pages: &Pages, name: &str) -> Result<(), AppError> {
        if self.unexpected_raw || self.raw.create_entered {
            return Err(refused());
        }
        let error = match fs::symlink_metadata(pages.0.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => io(e),
            Ok(_) => refused(),
        };
        self.unexpected_raw = true;
        Err(error)
    }

    /// Called under the real selected Current before entering the key. An
    /// uncertain create or a preexisting attempt never grants signing. The
    /// same covered marker can service fresh cuts after known pre-effect refusals.
    pub(in super::super) fn cover_attempt(
        &mut self,
        intent: &mut InviteIntentJournal,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        intent.bind(guard)?;
        intent.require_original()?;
        let pages = intent.pages()?;
        self.require_raw_absent(pages, &intent.names.raw)?;
        if self.attempt_covered {
            self.attempt.require_exact(pages, &intent.names.attempt)?;
        } else {
            self.attempt.cover(pages, &intent.names.attempt)?;
            self.attempt_covered = true;
        }
        self.require_raw_absent(pages, &intent.names.raw)?;
        require_pages(pages)
    }

    pub(in super::super) fn require_attempt(
        &mut self,
        intent: &mut InviteIntentJournal,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if !self.attempt_covered {
            return Err(refused());
        }
        intent.bind(guard)?;
        intent.require_original()?;
        let pages = intent.pages()?;
        self.attempt.require_exact(pages, &intent.names.attempt)?;
        self.require_raw_absent(pages, &intent.names.raw)?;
        require_pages(pages)
    }

    pub(in super::super) fn prepare_raw(
        &mut self,
        intent: &InviteIntentJournal,
        store: &crate::store::ServerStore,
        signature: &[u8; 64],
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        if !self.attempt_covered || self.unexpected_raw {
            return Err(refused());
        }
        if self.raw.frame.is_none() {
            self.raw.frame = Some(seal_stage(
                store,
                intent.context.ok_or_else(refused)?,
                intent.object.commitment()?,
                Some((self.attempt.commitment()?, signature)),
                rng,
            )?);
        }
        Ok(())
    }

    pub(in super::super) fn cover_raw(
        &mut self,
        intent: &mut InviteIntentJournal,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if !self.attempt_covered || self.unexpected_raw {
            return Err(refused());
        }
        intent.bind(guard)?;
        intent.require_original()?;
        let pages = intent.pages()?;
        self.attempt.require_exact(pages, &intent.names.attempt)?;
        if self.raw_covered {
            self.raw.require_exact(pages, &intent.names.raw)?;
        } else {
            self.raw.cover(pages, &intent.names.raw)?;
            self.raw_covered = true;
        }
        self.attempt.require_exact(pages, &intent.names.attempt)?;
        intent.require_original()?;
        Ok(())
    }
}
