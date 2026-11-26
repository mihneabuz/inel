use crate::managed::group::BufGroupId;

const SLOT_MASK: u64 = u32::MAX as u64;

const BUF_GROUP_OFFSET: u64 = 32;
const BUF_GROUP_ID_MASK: u64 = (u16::MAX as u64) << BUF_GROUP_OFFSET;

const FLAGS_OFFSET: u64 = BUF_GROUP_OFFSET + 16;
const CREATES_RAW_FD_FLAG: u64 = 1 << FLAGS_OFFSET;
const CREATES_DIRECT_FD_FLAG: u64 = 1 << (FLAGS_OFFSET + 1);

/// Identifies a submitted op.
///
/// It contains the key to the [`Completions`] slab and it's also written into the
/// SQE's `user_data` field, so it can be retrieved from the CQE.
///
/// This is the only way to smuggle information between the SQE and CQE, so we also
/// use it to store the buffer group id for buffer select ops and other hint flags,
/// such as if the op is expected to create an fd. This allows us to clean up these
/// resources in the unlikely scenario that the op was dropped while it still had
/// some results available in the [`Completions`] slab.
///
/// # Safety
///
/// A `Key` must not be used anymore after a using it to cancel the op or witnessing
/// the final result CQE - since the handler it identifies will be cleaned up.
#[repr(transparent)]
#[derive(PartialEq, Eq, Debug)]
pub struct Key(u64);

impl Key {
    const IGNORE: Self = Self::from_slot(u32::MAX);

    pub const fn ignored() -> Self {
        Self::IGNORE
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_raw(&self) -> u64 {
        self.0
    }

    pub const fn from_slot(slot: u32) -> Self {
        Self(slot as u64)
    }

    pub const fn as_slot(&self) -> u32 {
        (self.0 & SLOT_MASK) as u32
    }

    pub const fn should_ignore(&self) -> bool {
        self.0 == Self::IGNORE.0
    }

    pub const fn set_hints(&mut self, hints: Hints) {
        self.0 |= hints.0
    }

    pub const fn hints(&self) -> Hints {
        Hints(self.0)
    }
}

/// Hints used to smuggle information about the op.
///
/// The CQE's `user_data` field is the only way to smuggle information from the SQE,
/// so we use this type to allow ops to add hints to the [`Key`], which the managed
/// [`Uring`] can then use to ensure resources are cleaned up on lost results.
///
/// All hints must fit in the upper 32 bits of the inner u64, as the rest are used
/// for the index into the [`Completions`] slab.
#[repr(transparent)]
#[derive(Debug)]
pub struct Hints(u64);

impl Hints {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn set_creates_raw_fd(&mut self) {
        self.0 |= CREATES_RAW_FD_FLAG;
    }

    pub const fn set_creates_direct_fd(&mut self) {
        self.0 |= CREATES_DIRECT_FD_FLAG;
    }

    pub const fn creates_any_fd(&self) -> bool {
        self.0 & (CREATES_DIRECT_FD_FLAG | CREATES_RAW_FD_FLAG) != 0
    }

    pub const fn creates_raw_fd(&self) -> bool {
        self.0 & CREATES_RAW_FD_FLAG != 0
    }

    pub const fn creates_direct_fd(&self) -> bool {
        self.0 & CREATES_DIRECT_FD_FLAG != 0
    }

    pub const fn set_used_group_id(&mut self, gid: BufGroupId) {
        self.0 |= (gid.as_raw() as u64) << BUF_GROUP_OFFSET;
    }

    pub const fn used_group_id(&self) -> Option<BufGroupId> {
        if self.0 & BUF_GROUP_ID_MASK == 0 {
            None
        } else {
            let raw = (self.0 & BUF_GROUP_ID_MASK) >> BUF_GROUP_OFFSET;
            BufGroupId::from_raw(raw as u16)
        }
    }
}
