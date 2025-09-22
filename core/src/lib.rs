use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

// This was pretty midcurve tbh
pub mod deser_containers {
    use std::ops::{Deref, DerefMut};

    #[repr(u64)]
    pub enum OwnedOrBorrowed<'a, T> {
        Owned(T),
        Borrowed(&'a T),
    }

    impl<'a, T> Deref for OwnedOrBorrowed<'a, T> {
        type Target = T;
        fn deref(&self) -> &Self::Target {
            match self {
                OwnedOrBorrowed::Owned(t) | &OwnedOrBorrowed::Borrowed(t) => t,
            }
        }
    }

    #[repr(u64)]
    pub enum OwnedOrBorrowedMut<'a, T> {
        Owned(T),
        BorrowedMut(&'a mut T),
    }

    impl<'a, T> Deref for OwnedOrBorrowedMut<'a, T> {
        type Target = T;
        fn deref(&self) -> &Self::Target {
            match self {
                OwnedOrBorrowedMut::Owned(t) => t,
                OwnedOrBorrowedMut::BorrowedMut(t) => t,
            }
        }
    }

    impl<'a, T> DerefMut for OwnedOrBorrowedMut<'a, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            match self {
                OwnedOrBorrowedMut::Owned(t) => t,
                OwnedOrBorrowedMut::BorrowedMut(t) => t,
            }
        }
    }
}

/// This is a trait that allows for flexibility between nonzc/zc methods
#[rustfmt::skip]
pub trait FromBytes: Sized {
    type Target<'a>;
    type TargetMut<'a>;
    fn from_bytes<'a>(bytes: &'a [u8]) -> Result<Self::Target<'a>, ProgramError>;
    fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> Result<Self::TargetMut<'a>, ProgramError>;
}

// Core traits for the async/sync program pattern.
// fairly flexible but specific use cases may need more

pub trait SyncIx: FromBytes {
    fn process<S: AsyncState>(
        &self,
        data: &[u8],
        accounts: &[AccountInfo],
        state: &mut S,
    ) -> ProgramResult;
}

pub trait AsyncIx: FromBytes + Ord {
    type Args;
    fn process<S: AsyncState>(&self, args: &Self::Args, state: &mut S) -> ProgramResult;
}

pub trait AsyncState: FromBytes {
    type SyncIx: SyncIx;
    type AsyncIx: AsyncIx;
    type QueueArgs;

    fn queue_async(
        &mut self,
        ix: &Self::AsyncIx,
        args: &Self::QueueArgs,
    ) -> Result<(), ProgramError>;
    fn process_next_async(&mut self) -> ProgramResult;
    fn has_pending_async(&self, slot: u64) -> bool;
}

pub trait Program {
    type Sync: SyncIx;
    type Async: AsyncIx;
    type State: AsyncState<SyncIx = Self::Sync, AsyncIx = Self::Async>;

    fn process(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult;
}
