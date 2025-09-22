#![allow(unexpected_cfgs)]

use std::{
    hint::black_box,
    ops::{Deref, DerefMut},
};

use apq_core::{
    deser_containers::{OwnedOrBorrowed, OwnedOrBorrowedMut},
    AsyncIx, AsyncState, FromBytes, Program, SyncIx,
};
use bytemuck::{Pod, Zeroable};
use pinocchio::{
    account_info::AccountInfo,
    entrypoint,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvars::{clock::Clock, Sysvar},
    ProgramResult,
};
use sokoban::{red_black_tree::RBNode, NodeAllocatorMap, RedBlackTree, SENTINEL};

// Counter program implementation
#[derive(Debug)]
#[repr(u64)]
pub enum CounterSyncIx {
    RefillActions = 0,
}

impl CounterSyncIx {
    /// can use macros to derive this without user error
    const MAX_VARIANT: u64 = 0;
}

impl FromBytes for CounterSyncIx {
    type Target<'a> = &'a Self;
    type TargetMut<'a> = &'a mut Self;
    fn from_bytes<'a>(bytes: &'a [u8]) -> Result<&'a Self, ProgramError> {
        // We could do an owned version like this with 1 byte
        let _ = black_box(
            bytes
                .get(0)
                .is_some_and(|b| *b == 0)
                .then_some(CounterSyncIx::RefillActions)
                .ok_or(ProgramError::InvalidInstructionData),
        );

        // Or a zc version like this
        let (ix, _rem) = bytes
            .split_at_checked(8)
            .ok_or(ProgramError::InvalidInstructionData)?;
        if unsafe { *ix.as_ptr().cast::<u64>() } > CounterSyncIx::MAX_VARIANT {
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(unsafe { &*ix.as_ptr().cast::<CounterSyncIx>() })
    }

    fn from_bytes_mut<'a>(_bytes: &'a mut [u8]) -> Result<&'a mut Self, ProgramError> {
        unimplemented!("unused in this program")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u64)]
pub enum CounterAsyncIx {
    Decrement = 0, // 0 comes before 1
    Increment = 1,
}

impl CounterAsyncIx {
    const MAX_VARIANT: u64 = 1;
    pub unsafe fn from_u64_unchecked(a: u64) -> CounterAsyncIx {
        unsafe { core::mem::transmute(a) }
    }
}

impl FromBytes for CounterAsyncIx {
    type Target<'a> = OwnedOrBorrowed<'a, Self>;
    type TargetMut<'a> = OwnedOrBorrowedMut<'a, Self>;
    fn from_bytes<'a>(bytes: &'a [u8]) -> Result<OwnedOrBorrowed<'a, Self>, ProgramError> {
        let (ix, _rem) = bytes
            .split_at_checked(8)
            .ok_or(ProgramError::InvalidInstructionData)?;

        if unsafe { ix.as_ptr().cast::<u64>().read_unaligned() } > CounterAsyncIx::MAX_VARIANT {
            pinocchio_log::log!(
                "got ix variant {} > {}",
                unsafe { ix.as_ptr().cast::<u64>().read_unaligned() },
                CounterAsyncIx::MAX_VARIANT
            );
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(OwnedOrBorrowed::Owned(unsafe {
            ix.as_ptr().cast::<CounterAsyncIx>().read_unaligned()
        }))
    }

    fn from_bytes_mut<'a>(
        _bytes: &'a mut [u8],
    ) -> Result<OwnedOrBorrowedMut<'a, Self>, ProgramError> {
        unimplemented!()
    }
}

/// We first sort by auction (slot), then by ixn type, then by seq
#[derive(Copy, Clone, Zeroable, Pod, PartialEq, PartialOrd, Eq, Ord, Default, Debug)]
#[repr(C)]
pub struct AsyncIxKey {
    pub slot: u64,
    pub ixn_value: u64,
    pub seq: u64,
}

#[derive(Copy, Clone, Zeroable, Pod)]
#[repr(C)]
pub struct CounterState {
    /// Sequence number to assign to each action for time priority
    ///
    /// Starts at 1 to also be an init check
    pub seq: u64,

    /// Number of actions left before you need to add more
    ///
    /// Analogous to a user balance for financial markets
    pub num_actions: u64,

    /// The counter value that everyone cares about
    ///
    /// Analogous to market state + user balances for financial markets
    pub counter: u64,

    /// The asynchronous queue for decrements and increments
    ///
    /// Analogous to cancels and takes for financial markets
    pub async_queue: RedBlackTree<AsyncIxKey, Pubkey, 8192>,
}

impl CounterState {
    #[cfg(test)]
    fn new() -> Self {
        CounterState {
            counter: 0,
            num_actions: 0,
            seq: 0,
            async_queue: RedBlackTree::new(),
        }
    }

    pub fn peek_async(&self) -> Option<(u32, &RBNode<AsyncIxKey, Pubkey>)> {
        // this is kinda stupid af but lets do this for now lol
        let mut addr = self.async_queue.root;
        if addr == SENTINEL {
            return None;
        }

        let mut last_addr = addr;
        while addr != SENTINEL {
            last_addr = addr;
            addr = self.async_queue.get_left(addr);
        }

        Some((last_addr, self.async_queue.get_node(last_addr)))
    }

    pub fn pop_async(&mut self) -> Option<RBNode<AsyncIxKey, Pubkey>> {
        // TODO: change sokoban to allow for remove_addr,
        // currenly called _remove_tree_node
        let (_addr, &val) = self.peek_async()?;
        self.async_queue.remove(&val.key);
        Some(val)
    }
}

// For this we will cheat and use bytemuck
impl FromBytes for CounterState {
    type Target<'a> = &'a Self;
    type TargetMut<'a> = &'a mut Self;
    fn from_bytes<'a>(bytes: &'a [u8]) -> Result<Self::Target<'a>, ProgramError> {
        bytemuck::try_from_bytes(bytes).map_err(|_| ProgramError::InvalidAccountData)
    }

    fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> Result<Self::TargetMut<'a>, ProgramError> {
        bytemuck::try_from_bytes_mut(bytes).map_err(|_| ProgramError::InvalidAccountData)
    }
}

impl SyncIx for CounterSyncIx {
    fn process<S: AsyncState>(
        &self,
        _data: &[u8],
        _accounts: &[AccountInfo],
        state: &mut S,
    ) -> ProgramResult {
        match self {
            CounterSyncIx::RefillActions => {
                // This is a bit of a hack to access the concrete state
                // In a real implementation, you might want a better pattern
                let counter_state = unsafe { &mut *(state as *mut S as *mut CounterState) };
                counter_state.num_actions += 1;
                pinocchio_log::log!(
                    "Action requested. Total actions: {}",
                    counter_state.num_actions
                );
                Ok(())
            }
        }
    }
}

/// This could be an enum but for now we will make this a key for both inc/dec
pub struct CounterAsyncIxArgs {
    seq: u64,
}

impl AsyncIx for CounterAsyncIx {
    type Args = CounterAsyncIxArgs;

    fn process<S: AsyncState>(&self, args: &Self::Args, state: &mut S) -> ProgramResult {
        let counter_state = unsafe { &mut *(state as *mut S as *mut CounterState) };

        match self {
            CounterAsyncIx::Increment => {
                counter_state.counter = counter_state.counter.saturating_add(1);
                pinocchio_log::log!(
                    "Incremented; Seq {}. New value: {}",
                    args.seq,
                    counter_state.counter
                );
            }
            CounterAsyncIx::Decrement => {
                counter_state.counter = counter_state.counter.saturating_sub(1);
                pinocchio_log::log!(
                    "Decremented; Seq {}; New value: {}",
                    args.seq,
                    counter_state.counter
                );
            }
        }
        Ok(())
    }
}

/// This could be an enum but for now we will make this a key for both inc/dec
pub struct QueueAsyncArgs {
    key: Pubkey,
}

impl AsyncState for CounterState {
    type SyncIx = CounterSyncIx;
    type AsyncIx = CounterAsyncIx;

    type QueueArgs = QueueAsyncArgs;

    fn queue_async(
        &mut self,
        ixn: &Self::AsyncIx,
        args: &Self::QueueArgs,
    ) -> Result<(), ProgramError> {
        if self.num_actions == 0 {
            return Err(ProgramError::Custom(0x0));
        }
        // Insert in priority order
        let slot = get_slot();
        let key = AsyncIxKey {
            ixn_value: *ixn as u64,
            slot,
            seq: self.seq,
        };
        self.seq += 1;
        self.num_actions -= 1;
        self.async_queue.insert(key, args.key);

        let log_msg = format!(
            "Queued async instruction {:?} in slot {} with seq {}. Queue length: {}",
            ixn,
            slot,
            key.seq,
            self.async_queue.len()
        );
        pinocchio::msg!(&log_msg);

        Ok(())
    }

    fn process_next_async(&mut self) -> ProgramResult {
        if let Some(next) = self.pop_async() {
            let ixn = unsafe { std::mem::transmute::<&u64, &CounterAsyncIx>(&next.key.ixn_value) };
            let args = CounterAsyncIxArgs { seq: next.key.seq };
            ixn.process(&args, self)?;
        }
        Ok(())
    }

    fn has_pending_async(&self, slot: u64) -> bool {
        let Some((_addr, val)) = self.peek_async() else {
            return false;
        };

        val.key.slot + 1 <= slot
    }
}

fn get_slot() -> u64 {
    Clock::get().unwrap().slot
}

pub struct CounterProgram;

impl Program for CounterProgram {
    type Sync = CounterSyncIx;
    type Async = CounterAsyncIx;
    type State = CounterState;

    fn process(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let [state_account, user, _rem @ ..] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Load state with zero-copy
        let mut state_data = state_account.try_borrow_mut_data()?;
        // Check if this is an initialization
        if unsafe { *state_data.as_ptr().cast::<u64>() == 0 } {
            initialize_state(&mut state_data);
        }
        let mut state = Self::State::from_bytes_mut(&mut state_data[..])?;

        // Parse instruction
        let ix_type = instruction_data[0];
        let ix_data = &instruction_data[1..];

        match ix_type {
            0 => {
                pinocchio::msg!("Executing Synchronous Instruction");

                // Sync instruction
                let sync_ix = Self::Sync::from_bytes(&mut &ix_data[..])?;
                sync_ix.process(ix_data, accounts, state.deref_mut())?;
            }
            1 => {
                pinocchio::msg!("Queueing Aynchronous Instruction");

                // Async instruction - queue it
                let async_ix = Self::Async::from_bytes(&mut &ix_data[..])?;
                let args = QueueAsyncArgs { key: *user.key() };
                state.queue_async(async_ix.deref(), &args)?;
            }
            2 => {
                pinocchio::msg!("Executing Aynchronous Instruction");

                // Process next async instruction
                let slot = get_slot();
                while state.has_pending_async(slot) {
                    state.process_next_async()?;
                }

                pinocchio_log::log!("No pending async instructions");
            }
            _ => return Err(ProgramError::InvalidInstructionData),
        }

        // TODO: Save state when owned. in this example we never used owned so not important
        // state.serialize(&mut &mut state_data[..])?;

        Ok(())
    }
}

fn initialize_state(state_data: &mut [u8]) {
    pinocchio_log::log!("Initializing state");
    let CounterState {
        ref mut seq,
        ref mut async_queue,
        // zero initialized
        num_actions: _,
        counter: _,
    } = bytemuck::from_bytes_mut(&mut state_data[..]);
    *seq = 1;
    async_queue.initialize();
}

entrypoint!(process_instruction);

// #[inline(always)]
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    CounterProgram::process(program_id, accounts, instruction_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[rustfmt::skip]
    fn test_priority_queue() {
        let mut state = CounterState::new();

        // Queue items with different priorities
        state.queue_async(&CounterAsyncIx::Increment, &QueueAsyncArgs { key: [0; 32] }).unwrap();
        state.queue_async(&CounterAsyncIx::Decrement, &QueueAsyncArgs { key: [0; 32] }).unwrap();
        state.queue_async(&CounterAsyncIx::Increment, &QueueAsyncArgs { key: [0; 32] }).unwrap();
        state.queue_async(&CounterAsyncIx::Decrement, &QueueAsyncArgs { key: [0; 32] }).unwrap();

        assert_eq!(state.async_queue.len(), 4);

        // Pop should give us items in priority order
        for _ in 0..2 {
            match unsafe {
                CounterAsyncIx::from_u64_unchecked(state.pop_async().unwrap().key.ixn_value)
            } {
                CounterAsyncIx::Decrement => {}
                _ => panic!("Expected decerment"),
            }
        }

        for _ in 0..2 {
            match unsafe {
                CounterAsyncIx::from_u64_unchecked(state.pop_async().unwrap().key.ixn_value)
            } {
                CounterAsyncIx::Increment => {}
                _ => panic!("Expected increment"),
            }
        }
    }
}
