//! DISTINCT exporter-text CURRENT grammar. No initialized current sender ratchets are admitted.
//! This is negative resource validation, never signer/member/Current or original-source proof.
use super::{cursor::Cursor, Account, Error, Row};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::owner_restore) struct Facts {
    pub leaves: usize,
    pub parents: usize,
    pub secret_leaves: usize,
    pub secret_parents: usize,
    pub handshake: usize,
    pub application: usize,
    pub own_index: u64,
    pub secret_index: u64,
    pub secret_size: u64,
    pub epoch: u64,
    pub unmerged_max: Option<u32>,
    pub resumption_max: u64,
    pub resumption_cursor: u64,
    pub resumption_count: usize,
    pub resumption_epochs: [Option<u64>; 32],
}
impl Facts {
    pub fn finish(&self) -> Result<(), Error> {
        if !(1..=64).contains(&self.leaves)
            || self.parents != self.leaves - 1
            // SecretTree intentionally has N parent slots, unlike ABinaryTree's N-1.
            || self.secret_leaves != self.leaves || self.secret_parents != self.leaves
            || self.handshake != self.leaves || self.application != self.leaves
            || self.own_index != self.secret_index || self.own_index >= self.leaves as u64
            || self.secret_size != (2 * self.leaves - 1) as u64
            || self.unmerged_max.is_some_and(|index| index as usize >= self.leaves)
        {
            return Err(Error);
        }
        // Exact pinned founder ring, NOT sorted epochs and NOT a last-32 assumption
        // before its first full wrap. Welcome configuration0 stays genuinely empty.
        match self.resumption_max {
            0 if self.resumption_count == 0
                && self.resumption_cursor == 0
                && self.resumption_epochs == [None; 32] => {}
            32 => {
                let count = if self.epoch < 32 {
                    self.epoch as usize + 1
                } else {
                    32
                };
                let cursor = if self.epoch < 32 {
                    self.epoch + 1
                } else {
                    (self.epoch - 31) % 32
                };
                if self.resumption_count != count || self.resumption_cursor != cursor {
                    return Err(Error);
                }
                for i in 0..32 {
                    let expected = if i >= count {
                        None
                    } else if self.epoch < 32 {
                        Some(i as u64)
                    } else if self.epoch < 63 {
                        Some(if i == 0 || i > cursor as usize {
                            i as u64
                        } else {
                            31 + i as u64
                        })
                    } else {
                        let distance = (cursor + 32 - i as u64) % 32;
                        Some(self.epoch - distance)
                    };
                    if self.resumption_epochs[i] != expected {
                        return Err(Error);
                    }
                }
            }
            _ => return Err(Error),
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Signer,
    Tree,
    PublicTree,
    LeafSlot,
    ParentSlot,
    BlankSlot,
    Leaf,
    Payload,
    Capabilities,
    LeafSource,
    Lifetime,
    Parent,
    Unmerged,
    Context,
    Confirmation,
    EpochSecrets,
    MessageStore,
    Message,
    SecretTree,
    Resumption,
    ResumptionLimit,
    ResumptionCursor,
    ResumptionEntries,
    Config,
    WirePolicy,
    RatchetConfig,
    AddedAt,
    Time,
    Credential,
    Extensions,
    SecretWrap,
    Secret,
    Key,
    PublicKey,
    Signature,
    V32,
    V64,
    VHash,
    VCredential,
    VGroup,
    ContextGroup,
    Bytes32,
    Bytes64,
    BytesHash,
    BytesCredential,
    BytesGroup,
    BytesPublic,
    BytesContext,
    Zero,
    SuiteNumber,
    Index,
    SecretIndex,
    TreeSize,
    Epoch,
    U64,
    Nanos,
    BoolTrue,
    Empty,
    Null,
    Basic,
    Mls10,
    SuiteName,
    Ed25519,
    Ciphertext,
    Operational,
    Versions,
    Suites,
    Credentials,
    Leaves,
    Parents,
    SecretLeaves,
    SecretParents,
    Handshake,
    Application,
    OptionalSecret,
    Indices,
}
#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    alias: Option<&'static str>,
    shape: Shape,
    optional: bool,
}
const fn f(name: &'static str, shape: Shape) -> Field {
    Field {
        name,
        alias: None,
        shape,
        optional: false,
    }
}
fn fields(shape: Shape) -> Option<&'static [Field]> {
    use Shape::*;
    // Calling a const fn in an ordinary array expression does not itself make that
    // temporary array a static table. Give every schema an explicit const context so this
    // borrowed parser performs no table allocation/copy and returns a genuine static slice.
    macro_rules! table {
        ($($field:expr),* $(,)?) => {{
            const FIELDS: &[Field] = &[$($field),*];
            FIELDS
        }};
    }
    Some(match shape {
        Signer => table![
            f("private", Bytes32),
            f("public", BytesPublic),
            f("signature_scheme", Ed25519),
        ],
        Tree => table![f("tree", PublicTree), f("tree_hash", Bytes32)],
        PublicTree => table![
            f("leaf_nodes", Leaves),
            f("parent_nodes", Parents),
            f("default_leaf", BlankSlot),
            f("default_parent", BlankSlot),
        ],
        LeafSlot => table![f("node", Leaf)],
        ParentSlot => table![f("node", Parent)],
        BlankSlot => table![f("node", Null)],
        Leaf => table![f("payload", Payload), f("signature", Signature)],
        Payload => table![
            f("encryption_key", Key),
            f("signature_key", PublicKey),
            f("credential", Credential),
            f("capabilities", Capabilities),
            f("leaf_node_source", LeafSource),
            f("extensions", Extensions),
        ],
        Capabilities => table![
            f("versions", Versions),
            f("ciphersuites", Suites),
            f("extensions", Empty),
            f("proposals", Empty),
            f("credentials", Credentials),
        ],
        Lifetime => table![f("not_before", U64), f("not_after", U64)],
        Parent => table![
            f("encryption_key", Key),
            f("parent_hash", VHash),
            f("unmerged_leaves", Unmerged),
        ],
        Unmerged => table![f("list", Indices)],
        Context => table![
            f("protocol_version", Mls10),
            f("ciphersuite", SuiteName),
            f("group_id", ContextGroup),
            f("epoch", Epoch),
            f("tree_hash", V32),
            f("confirmed_transcript_hash", VHash),
            f("extensions", Extensions),
        ],
        Confirmation => table![f("mac_value", V32)],
        EpochSecrets => table![
            f("init_secret", SecretWrap),
            f("exporter_secret", SecretWrap),
            f("epoch_authenticator", SecretWrap),
            f("external_secret", SecretWrap),
            f("resumption_psk", SecretWrap),
        ],
        MessageStore => table![
            f("max_epochs", Zero),
            f("past_epoch_trees", Empty),
            f("message_secrets", Message),
        ],
        Message => table![
            f("sender_data_secret", SecretWrap),
            f("membership_key", SecretWrap),
            f("confirmation_key", SecretWrap),
            f("serialized_context", BytesContext),
            f("secret_tree", SecretTree),
            Field {
                name: "added_at",
                alias: None,
                shape: AddedAt,
                optional: true,
            },
        ],
        SecretTree => table![
            f("own_index", SecretIndex),
            f("leaf_nodes", SecretLeaves),
            f("parent_nodes", SecretParents),
            f("handshake_sender_ratchets", Handshake),
            f("application_sender_ratchets", Application),
            f("size", TreeSize),
        ],
        Resumption => table![
            f("max_number_of_secrets", ResumptionLimit),
            f("resumption_psk", ResumptionEntries),
            f("cursor", ResumptionCursor),
        ],
        Config => table![
            f("wire_format_policy", WirePolicy),
            f("padding_size", Zero),
            Field {
                name: "past_epoch_deletion_policy",
                alias: Some("max_past_epochs"),
                shape: Zero,
                optional: false,
            },
            f("number_of_resumption_psks", Zero),
            f("use_ratchet_tree_extension", BoolTrue),
            f("sender_ratchet_configuration", RatchetConfig),
        ],
        WirePolicy => table![f("outgoing", Ciphertext), f("incoming", Ciphertext)],
        RatchetConfig => table![
            f("out_of_order_tolerance", U64),
            f("maximum_forward_distance", U64),
        ],
        Time => table![f("secs_since_epoch", U64), f("nanos_since_epoch", Nanos)],
        Credential => table![
            f("credential_type", Basic),
            f("serialized_credential_content", VCredential),
        ],
        Extensions => table![f("unique", Empty)],
        SecretWrap => table![f("secret", Secret)],
        Secret => table![f("value", V32)],
        Key => table![f("key", V32)],
        PublicKey => table![f("value", V32)],
        Signature => table![f("value", V64)],
        V32 => table![f("vec", Bytes32)],
        V64 => table![f("vec", Bytes64)],
        VHash => table![f("vec", BytesHash)],
        VCredential => table![f("vec", BytesCredential)],
        VGroup => table![f("vec", BytesGroup)],
        ContextGroup => table![f("value", VGroup)],
        _ => return None,
    })
}

struct Scan<'de, 'a> {
    cursor: Cursor<'de, 'a>,
    group: &'a [u8; 16],
    signer: &'a [u8; 32],
    facts: &'a mut Facts,
}
impl Scan<'_, '_> {
    fn object(&mut self, fields: &[Field], depth: usize) -> Result<(), Error> {
        self.cursor.take(b'{')?;
        let mut seen = 0u32;
        if !self.cursor.eat(b'}')? {
            loop {
                let name = self.cursor.string()?;
                self.cursor.account.work(
                    fields
                        .len()
                        .checked_mul(name.len().checked_add(1).ok_or(Error)?)
                        .ok_or(Error)?,
                )?;
                let (position, field) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, field)| field.name == name || field.alias == Some(name))
                    .ok_or(Error)?;
                let bit = 1u32.checked_shl(position as u32).ok_or(Error)?;
                if seen & bit != 0 {
                    return Err(Error);
                }
                seen |= bit;
                self.cursor.take(b':')?;
                // Ratchet configuration is fixed rather than authority decoded from this row.
                if name == "out_of_order_tolerance" {
                    if self.cursor.number()? != 5 {
                        return Err(Error);
                    }
                } else if name == "maximum_forward_distance" {
                    if self.cursor.number()? != 1000 {
                        return Err(Error);
                    }
                } else {
                    self.value(field.shape, depth + 1)?;
                }
                if self.cursor.eat(b'}')? {
                    break;
                }
                self.cursor.take(b',')?;
            }
        }
        for (position, field) in fields.iter().enumerate() {
            if !field.optional && seen & (1 << position) == 0 {
                return Err(Error);
            }
        }
        Ok(())
    }
    fn sequence(
        &mut self,
        shape: Shape,
        minimum: usize,
        maximum: usize,
        depth: usize,
    ) -> Result<usize, Error> {
        self.cursor.take(b'[')?;
        let mut count = 0usize;
        if !self.cursor.eat(b']')? {
            loop {
                count = count
                    .checked_add(1)
                    .filter(|count| *count <= maximum)
                    .ok_or(Error)?;
                self.value(shape, depth + 1)?;
                if self.cursor.eat(b']')? {
                    break;
                }
                self.cursor.take(b',')?;
            }
        }
        if count < minimum {
            return Err(Error);
        }
        Ok(count)
    }
    fn bytes(
        &mut self,
        minimum: usize,
        maximum: usize,
        expected: Option<&[u8]>,
    ) -> Result<usize, Error> {
        self.cursor.take(b'[')?;
        let mut count = 0usize;
        if !self.cursor.eat(b']')? {
            loop {
                if count >= maximum {
                    return Err(Error);
                }
                let byte = u8::try_from(self.cursor.number()?).map_err(|_| Error)?;
                if expected.is_some_and(|expected| expected.get(count) != Some(&byte)) {
                    return Err(Error);
                }
                count += 1;
                if self.cursor.eat(b']')? {
                    break;
                }
                self.cursor.take(b',')?;
            }
        }
        if count < minimum {
            return Err(Error);
        }
        Ok(count)
    }
    fn value(&mut self, shape: Shape, depth: usize) -> Result<(), Error> {
        use Shape::*;
        if depth > 24 {
            return Err(Error);
        }
        self.cursor.account.work(1)?;
        self.cursor.space()?;
        // Real public slots may be blank, including padding. The separate default slots MUST
        // be blank. SecretTree nullable values have their own actual inline element layouts.
        if matches!(shape, Leaf | Parent | OptionalSecret | AddedAt)
            && self.cursor.peek()? == Some(b'n')
        {
            return self.cursor.word(b"null");
        }
        if let Some(fields) = fields(shape) {
            return self.object(fields, depth);
        }
        match shape {
            Bytes32 => {
                self.bytes(32, 32, None)?;
            }
            Bytes64 => {
                self.bytes(64, 64, None)?;
            }
            BytesHash => {
                if !matches!(self.bytes(0, 32, None)?, 0 | 32) {
                    return Err(Error);
                }
            }
            BytesCredential => {
                self.bytes(1, 512, None)?;
            }
            BytesGroup => {
                self.bytes(16, 16, Some(self.group))?;
            }
            BytesPublic => {
                self.bytes(32, 32, Some(self.signer))?;
            }
            BytesContext => {
                self.bytes(1, 256, None)?;
            }
            Zero => {
                if self.cursor.number()? != 0 {
                    return Err(Error);
                }
            }
            ResumptionLimit => {
                let maximum = self.cursor.number()?;
                if !matches!(maximum, 0 | 32) {
                    return Err(Error);
                }
                self.facts.resumption_max = maximum;
            }
            ResumptionCursor => self.facts.resumption_cursor = self.cursor.number()?,
            ResumptionEntries => {
                self.cursor.take(b'[')?;
                let mut count = 0usize;
                if !self.cursor.eat(b']')? {
                    loop {
                        if count >= 32 {
                            return Err(Error);
                        }
                        self.cursor.take(b'[')?;
                        let epoch = self.cursor.number()?;
                        // Slot order is checked against the real cursor/epoch only
                        // after all rows are scanned; JSON field order is not authority.
                        self.cursor.take(b',')?;
                        self.value(SecretWrap, depth + 2)?;
                        self.cursor.take(b']')?;
                        self.facts.resumption_epochs[count] = Some(epoch);
                        count += 1;
                        if self.cursor.eat(b']')? {
                            break;
                        }
                        self.cursor.take(b',')?;
                    }
                }
                self.facts.resumption_count = count;
            }
            SuiteNumber => {
                if self.cursor.number()? != 3 {
                    return Err(Error);
                }
            }
            Index => self.facts.own_index = self.cursor.number()?,
            SecretIndex => self.facts.secret_index = self.cursor.number()?,
            TreeSize => self.facts.secret_size = self.cursor.number()?,
            Epoch => self.facts.epoch = self.cursor.number()?,
            U64 => {
                self.cursor.number()?;
            }
            Nanos => {
                if self.cursor.number()? >= 1_000_000_000 {
                    return Err(Error);
                }
            }
            BoolTrue => self.cursor.word(b"true")?,
            Empty => {
                self.cursor.take(b'[')?;
                self.cursor.take(b']')?;
            }
            Null => self.cursor.word(b"null")?,
            Basic | Mls10 | SuiteName | Ed25519 | Ciphertext | Operational => {
                let expected = match shape {
                    Basic => "Basic",
                    Mls10 => "Mls10",
                    Ed25519 => "ED25519",
                    Ciphertext => "AlwaysCiphertext",
                    Operational => "Operational",
                    _ => "MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519",
                };
                if self.cursor.string()? != expected {
                    return Err(Error);
                }
            }
            Versions => {
                self.sequence(Mls10, 1, 1, depth)?;
            }
            Suites => {
                self.sequence(SuiteNumber, 1, 1, depth)?;
            }
            Credentials => {
                self.sequence(Basic, 1, 1, depth)?;
            }
            Leaves => self.facts.leaves = self.sequence(LeafSlot, 1, 64, depth)?,
            Parents => self.facts.parents = self.sequence(ParentSlot, 0, 63, depth)?,
            SecretLeaves => {
                self.facts.secret_leaves = self.sequence(OptionalSecret, 1, 64, depth)?
            }
            SecretParents => {
                self.facts.secret_parents = self.sequence(OptionalSecret, 1, 64, depth)?
            }
            Handshake => self.facts.handshake = self.sequence(Null, 1, 64, depth)?,
            Application => self.facts.application = self.sequence(Null, 1, 64, depth)?,
            // Full public parent lists remain sorted, unique and bounded by actual slots.
            Indices => {
                self.cursor.take(b'[')?;
                let mut previous = None;
                let mut count = 0usize;
                if !self.cursor.eat(b']')? {
                    loop {
                        count += 1;
                        if count > 64 {
                            return Err(Error);
                        }
                        let index = u32::try_from(self.cursor.number()?).map_err(|_| Error)?;
                        if previous.is_some_and(|previous| previous >= index) {
                            return Err(Error);
                        }
                        previous = Some(index);
                        self.facts.unmerged_max =
                            Some(self.facts.unmerged_max.map_or(index, |old| old.max(index)));
                        if self.cursor.eat(b']')? {
                            break;
                        }
                        self.cursor.take(b',')?;
                    }
                }
            }
            OptionalSecret => self.object(&[f("secret", Secret)], depth)?,
            AddedAt => self.value(Time, depth + 1)?,
            LeafSource => {
                self.cursor.take(b'{')?;
                let variant = self.cursor.string()?;
                self.cursor.take(b':')?;
                match variant {
                    "KeyPackage" => self.value(Lifetime, depth + 1)?,
                    "Commit" => self.value(VHash, depth + 1)?,
                    _ => return Err(Error),
                }
                self.cursor.take(b'}')?;
            }
            _ => return Err(Error),
        }
        Ok(())
    }
}

// The distinct private Add candidate repeats this same negative shape scan
// before loading its graph; sibling access grants no mutation or Source.
pub(in crate::owner_restore) fn check(
    row: Row,
    bytes: &[u8],
    group: &[u8; 16],
    signer: &[u8; 32],
    facts: &mut Facts,
    account: &mut Account,
) -> Result<(), Error> {
    if bytes.len() > 512 * 1024 {
        return Err(Error);
    }
    // The repeated SAME-row pass must recompute its own max, not inherit a larger original
    // value that could hide a changed/lowered row. N may occur later in JSON field order.
    if row == Row::Tree {
        facts.unmerged_max = None;
    }
    if row == Row::Resumption {
        // Repeated actual getters recompute the SAME row facts. Never accumulate an earlier
        // prefix, which could conceal a changed/shortened store between provider reads.
        facts.resumption_max = 0;
        facts.resumption_cursor = 0;
        facts.resumption_count = 0;
        facts.resumption_epochs = [None; 32];
    }
    let shape = match row {
        Row::Signer => Shape::Signer,
        Row::Tree => Shape::Tree,
        Row::Proposals | Row::OwnLeaves => Shape::Empty,
        Row::Context => Shape::Context,
        Row::Interim => Shape::Bytes32,
        Row::Confirmation => Shape::Confirmation,
        Row::Epoch => Shape::EpochSecrets,
        Row::OwnIndex => Shape::Index,
        Row::Messages => Shape::MessageStore,
        Row::Resumption => Shape::Resumption,
        Row::Config => Shape::Config,
        Row::State => Shape::Operational,
    };
    let mut scan = Scan {
        cursor: Cursor::new(bytes, account),
        group,
        signer,
        facts,
    };
    scan.value(shape, 0)?;
    scan.cursor.finish()
}
