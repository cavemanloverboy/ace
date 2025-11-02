use std::array::from_ref;
use std::path::Path;

use counter::{CounterAsyncIx, CounterState};
use litesvm::LiteSVM;
use sokoban::NodeAllocatorMap;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program::clock::Clock;
use solana_program::message::Message;
use solana_program::system_instruction;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

// Counter program ID
const COUNTER_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("CounterProgram111111111111111111111111111111");

fn main() {
    println!("=== Advanced Async/Sync Counter Demo ===\n");
    println!("NOTE: Each operation uses a unique user to simulate real-world usage\n");

    let mut svm = LiteSVM::new()
        .with_blockhash_check(false)
        .with_sigverify(false)
        .with_transaction_history(50);

    // Load program
    let path = Path::new("./target/deploy/counter.so");
    println!(
        "Loading program from: {} (exists: {})",
        path.display(),
        path.exists()
    );
    svm.add_program_from_file(COUNTER_PROGRAM_ID, path).unwrap();
    let svm = &mut svm;

    let payer = Pubkey::new_unique();
    svm.airdrop(&payer, 10_000_000_000).unwrap();

    let state_account = Keypair::new();

    // Calculate actual state size
    let state_size = std::mem::size_of::<CounterState>();
    println!("State size: {} bytes", state_size);

    // Create state account
    let create_ix = system_instruction::create_account(
        &payer,
        &state_account.pubkey(),
        svm.minimum_balance_for_rent_exemption(state_size),
        state_size as u64,
        &COUNTER_PROGRAM_ID,
    );

    execute(
        svm,
        &payer,
        from_ref(&create_ix),
        &[state_account.pubkey()],
        "Create state account",
    );

    // Create multiple users with names
    let users = vec![
        (
            "Alice",
            const { solana_pubkey::pubkey!("a1icea1icea1icea1icea1icea1icea1icea1icea1i") },
        ),
        (
            "Bob",
            const { solana_pubkey::pubkey!("bobbobbobbobbobbobbobbobbobbobbobbobbobbobb") },
        ),
        (
            "Carol",
            const { solana_pubkey::pubkey!("caroLcaroLcaroLcaroLcaroLcaroLcaroLcaroLcar") },
        ),
        (
            "Dave",
            const { solana_pubkey::pubkey!("davedavedavedavedavedavedavedavedavedavedav") },
        ),
        (
            "Eve",
            const { solana_pubkey::pubkey!("eveeveeveeveeveeveeveeveeveeveeveeveeveevee") },
        ),
    ];

    // Alice refills many actions for everyone
    for _ in 0..100 {
        let refill_ix = create_sync_instruction(&state_account.pubkey(), &users[0].1, 0);
        execute(svm, &payer, from_ref(&refill_ix), &[], "");
    }

    // Show all users
    println!("\nUsers participating:");
    for (name, user) in &users {
        println!("  {} -> {}", name, short_pubkey(&user));
    }

    // Queue operations from different users with a story
    println!("\nUsers queuing operations:");

    // Alice increments
    let ix = create_async_instruction(&state_account.pubkey(), &users[0].1, 1);
    execute(svm, &payer, from_ref(&ix), &[], "Alice queues increment");

    // Bob decrements
    let ix = create_async_instruction(&state_account.pubkey(), &users[1].1, 0);
    execute(svm, &payer, from_ref(&ix), &[], "Bob queues decrement");

    // Carol increments
    let ix = create_async_instruction(&state_account.pubkey(), &users[2].1, 1);
    execute(svm, &payer, from_ref(&ix), &[], "Carol queues increment");

    // Dave decrements
    let ix = create_async_instruction(&state_account.pubkey(), &users[3].1, 0);
    execute(svm, &payer, from_ref(&ix), &[], "Dave queues decrement");

    // Eve increments
    let ix = create_async_instruction(&state_account.pubkey(), &users[4].1, 1);
    execute(svm, &payer, from_ref(&ix), &[], "Eve queues increment");

    print_detailed_state(
        svm,
        &state_account.pubkey(),
        "After 5 users queue operations",
    );

    // Process with a different user (system operator)
    svm.warp_to_slot(get_current_slot(&svm) + 3);
    let operator = Keypair::new();
    println!(
        "\nSystem operator ({}) processing queue",
        short_pubkey(&operator.pubkey())
    );
    let process_ix = create_process_async_instruction(&state_account.pubkey(), &operator.pubkey());
    execute(
        svm,
        &payer,
        from_ref(&process_ix),
        &[],
        "Operator processes queue",
    );

    print_detailed_state(
        &svm,
        &state_account.pubkey(),
        "After processing (Bob and Dave's decrements should execute first)",
    );

    // Show some users doing more operations
    println!("\n--- Round 2: More user activity ---");

    // New users join
    let frank = const { solana_pubkey::pubkey!("frankfrankfrankfrankfrankfrankfrankfrankfra") };
    let grace = const { solana_pubkey::pubkey!("gracegracegracegracegracegracegracegracegra") };
    println!("\nNew users join:");
    println!("  Frank -> {}", short_pubkey(&frank));
    println!("  Grace -> {}", short_pubkey(&grace));

    let ix = create_async_instruction(&state_account.pubkey(), &frank, 0);
    execute(svm, &payer, from_ref(&ix), &[], "Frank queues decrement");

    let ix = create_async_instruction(&state_account.pubkey(), &grace, 1);
    execute(svm, &payer, from_ref(&ix), &[], "Grace queues increment");

    print_detailed_state(
        svm,
        &state_account.pubkey(),
        "After new users join and queue operations",
    );

    // Final summary
    println!("\n=== Demo Complete ===");
    print_detailed_state(&svm, &state_account.pubkey(), "Final program state");
}

fn get_current_slot(svm: &LiteSVM) -> u64 {
    svm.get_sysvar::<Clock>().slot
}

// Generate a short identifier for a pubkey (first 8 chars)
fn short_pubkey(pubkey: &Pubkey) -> String {
    pubkey.to_string()[..8].to_string()
}

fn create_sync_instruction(state_account: &Pubkey, user: &Pubkey, sync_ix: u64) -> Instruction {
    let mut data = vec![0u8]; // 0 = sync instruction
    data.extend_from_slice(&sync_ix.to_le_bytes());

    Instruction {
        program_id: COUNTER_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*state_account, false),
            AccountMeta::new_readonly(*user, false),
        ],
        data,
    }
}

fn create_async_instruction(state_account: &Pubkey, user: &Pubkey, async_ix: u64) -> Instruction {
    let mut data = vec![1u8]; // 1 = async instruction
    data.extend_from_slice(&async_ix.to_le_bytes());

    Instruction {
        program_id: COUNTER_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*state_account, false),
            AccountMeta::new_readonly(*user, false),
        ],
        data,
    }
}

fn create_process_async_instruction(state_account: &Pubkey, user: &Pubkey) -> Instruction {
    Instruction {
        program_id: COUNTER_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*state_account, false),
            AccountMeta::new_readonly(*user, false),
        ],
        data: vec![2u8], // 2 = process async
    }
}

#[track_caller]
fn execute(
    svm: &mut LiteSVM,
    &payer: &Pubkey,
    instructions: &[Instruction],
    additional_signers: &[Pubkey],
    description: &str,
) {
    if description != "" {
        println!("\n>> {}", description);
    }

    let mut signers = vec![payer];
    signers.extend_from_slice(additional_signers);

    let message = Message::new(instructions, Some(&payer));
    let tx = Transaction::new_unsigned(message);

    match svm.send_transaction(tx) {
        Ok(res) => {
            if !res.logs.is_empty() && description != "" {
                println!("   Logs:");
                for log in &res.logs {
                    // Print all logs for debugging
                    println!("     {}", log);
                }
            }
        }
        Err(e) => {
            println!("called from {}", std::panic::Location::caller());
            panic!("   ERROR: {:?}", e);
        }
    }
}

fn print_detailed_state(svm: &LiteSVM, state_account: &Pubkey, context: &str) {
    println!("\n[State: {}]", context);

    if let Some(account) = svm.get_account(state_account) {
        let state: &CounterState = bytemuck::from_bytes(&account.data);

        println!("  Sequence: {}", state.seq);
        println!("  Num Actions: {}", state.num_actions);
        println!("  Counter: {}", state.counter);

        let queue = state.async_queue.iter();
        println!("  Queued instructions:");
        for (i, ixn) in queue.enumerate() {
            let ixn_type: CounterAsyncIx = unsafe { core::mem::transmute(ixn.0.ixn_value) };
            let user: Pubkey = Pubkey::new_from_array(*ixn.1);
            let seq = ixn.0.seq;
            let slot = ixn.0.slot;
            println!("   {i:>3}: {ixn_type:?}; seq {seq} in slot {slot}; {user}");
        }
    } else {
        panic!("  Account not found!");
    }
}
