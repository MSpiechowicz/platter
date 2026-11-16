use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

fn entries(max_entries: u64) -> BudgetLimits {
    BudgetLimits {
        max_entries,
        ..BudgetLimits::UNLIMITED
    }
}

/// Admits entries with `name_len`-byte names on `lease` until it is refused; how many got in.
fn admit_all(lease: &mut Lease<'_>, name_len: usize, most: u64) -> u64 {
    let mut admitted = 0;
    while admitted < most && lease.admit(name_len, false) {
        admitted += 1;
    }
    admitted
}

#[test]
fn a_lease_admits_exactly_the_entries_that_fit_and_refuses_the_next_one() {
    for cap in [0, 1, 2, 7, 100, 1024, 1025, 5000] {
        let budget = EntryBudget::new(entries(cap), 1);
        let mut lease = budget.lease();

        assert_eq!(admit_all(&mut lease, 3, u64::MAX), cap, "cap {cap}");
        assert!(!lease.admit(3, false), "cap {cap}: still refused");
        assert_eq!(budget.limit_hit(), Some(LimitHit::Entries), "cap {cap}");
        assert!(budget.is_exhausted());
    }
}

#[test]
fn a_limit_that_is_not_reached_is_not_hit() {
    let budget = EntryBudget::new(entries(100), 1);
    let mut lease = budget.lease();

    assert_eq!(admit_all(&mut lease, 3, 100), 100, "exactly at the cap");

    assert_eq!(budget.limit_hit(), None);
    assert!(lease.commit(100, 300, 100 * 214));
    assert_eq!(budget.used(), (100, 300));
    assert_eq!(budget.limit_hit(), None);
}

#[test]
fn only_the_first_limit_that_was_hit_is_kept() {
    let budget = EntryBudget::new(
        BudgetLimits {
            max_entries: 10,
            max_name_bytes: 15,
            max_memory_bytes: u64::MAX,
        },
        1,
    );
    let mut lease = budget.lease();

    assert!(admit_all(&mut lease, 4, u64::MAX) < 10);
    assert_eq!(budget.limit_hit(), Some(LimitHit::NameBytes));

    budget.note_hit(LimitHit::Entries);
    assert_eq!(budget.limit_hit(), Some(LimitHit::NameBytes));
}

#[test]
fn a_listing_that_fails_gives_everything_it_reserved_back() {
    let budget = EntryBudget::new(entries(10_000), 4);

    {
        let mut lease = budget.lease();
        assert_eq!(admit_all(&mut lease, 8, 700), 700);
        assert!(
            budget.used().0 >= 700,
            "the batch is reserved while the lease lives"
        );
        // The listing fails here: the lease is dropped without a commit.
    }

    assert_eq!(budget.used(), (0, 0), "nothing leaks");
    assert_eq!(budget.limit_hit(), None);
}

#[test]
fn committing_keeps_what_the_listing_holds_and_gives_back_the_rest() {
    let budget = EntryBudget::new(entries(10_000), 4);

    let mut lease = budget.lease();
    assert_eq!(admit_all(&mut lease, 8, 700), 700);
    // Three of the admitted entries vanished before they were returned.
    assert!(lease.commit(697, 697 * 8, 697 * 224));

    assert_eq!(budget.used(), (697, 697 * 8));
}

#[test]
fn a_source_that_never_asked_is_settled_against_the_budget_afterwards() {
    let budget = EntryBudget::new(entries(100), 1);

    assert!(budget.lease().commit(100, 1000, 100 * 228), "exactly at the cap");
    assert_eq!(budget.used(), (100, 1000));

    assert!(!budget.lease().commit(1, 10, 228), "one more does not fit");
    assert_eq!(budget.limit_hit(), Some(LimitHit::Entries));
    assert_eq!(budget.used(), (100, 1000), "the refused listing holds nothing");
}

#[test]
fn name_bytes_have_a_limit_of_their_own() {
    let budget = EntryBudget::new(
        BudgetLimits {
            max_name_bytes: 1000,
            ..BudgetLimits::UNLIMITED
        },
        1,
    );
    let mut lease = budget.lease();

    assert_eq!(admit_all(&mut lease, 10, u64::MAX), 100);
    assert_eq!(budget.limit_hit(), Some(LimitHit::NameBytes));
}

#[test]
fn a_name_is_counted_up_to_the_longest_the_tree_keeps() {
    let budget = EntryBudget::new(
        BudgetLimits {
            max_name_bytes: 70_000,
            ..BudgetLimits::UNLIMITED
        },
        1,
    );
    let mut lease = budget.lease();

    assert!(lease.admit(1 << 20, false), "cut to 65535 bytes, which fit");
    assert!(!lease.admit(1 << 20, false), "a second one does not");
}

#[test]
fn long_names_run_into_the_memory_limit_long_before_the_entry_cap() {
    // 708 bytes an entry with a 250 byte name (the name counts twice); the entry cap is far away.
    let per_entry = ENTRY_BASE_COST_BYTES + NAME_COPIES * 250;
    let budget = EntryBudget::new(
        BudgetLimits {
            max_entries: 1_000_000,
            max_name_bytes: u64::MAX,
            max_memory_bytes: 1000 * per_entry,
        },
        1,
    );
    let mut lease = budget.lease();

    let admitted = admit_all(&mut lease, 250, u64::MAX);

    assert!(admitted <= 1000, "never more than the memory allows: {admitted}");
    assert!(admitted >= 950, "and not much less than it allows: {admitted}");
    assert_eq!(budget.limit_hit(), Some(LimitHit::Memory));
}

#[test]
fn short_names_run_into_the_entry_cap_first_when_memory_is_plentiful() {
    let budget = EntryBudget::new(
        BudgetLimits {
            max_entries: 5000,
            max_name_bytes: u64::MAX,
            max_memory_bytes: 1 << 40,
        },
        1,
    );

    assert_eq!(admit_all(&mut budget.lease(), 8, u64::MAX), 5000);
    assert_eq!(budget.limit_hit(), Some(LimitHit::Entries));
}

#[test]
fn the_limit_holds_however_many_workers_reserve_at_once() {
    const WORKERS: usize = 32;
    const CAP: u64 = 50_000;
    let admitted = AtomicU64::new(0);
    let budget = EntryBudget::new(entries(CAP), WORKERS);

    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                let mut lease = budget.lease();
                let mine = admit_all(&mut lease, 6, u64::MAX);
                admitted.fetch_add(mine, Ordering::SeqCst);

                // The listing ends with what it admitted; the room it did not use goes back.
                assert!(lease.commit(mine as usize, mine * 6, mine * 220) || budget.is_exhausted());
            });
        }
    });

    let admitted = admitted.into_inner();
    assert!(admitted <= CAP, "never above the cap: {admitted}");
    assert!(
        admitted + WORKERS as u64 * BATCH_ENTRIES >= CAP,
        "at most the room other workers held idle was refused: {admitted}"
    );
    assert_eq!(budget.limit_hit(), Some(LimitHit::Entries));
}

#[test]
fn near_the_cap_a_worker_only_asks_for_a_share_of_what_is_left() {
    // 32 workers, 1000 entries left: nobody may sit on more than 1000 / 64 = 15 entries of it.
    let budget = EntryBudget::new(entries(1000), 32);
    let mut first = budget.lease();

    assert!(first.admit(1, false));

    let (reserved, _) = budget.used();
    assert!(reserved <= 1000 / 64, "reserved {reserved}");
}

#[test]
fn a_directory_is_counted_for_more_memory_than_a_file() {
    let limit = 100 * (ENTRY_BASE_COST_BYTES + DIR_EXTRA_COST_BYTES + 2 * 8);
    let admit_kind = |is_dir: bool| {
        let budget = EntryBudget::new(
            BudgetLimits {
                max_memory_bytes: limit,
                ..BudgetLimits::UNLIMITED
            },
            1,
        );
        let mut lease = budget.lease();
        let mut admitted = 0;
        while lease.admit(8, is_dir) {
            admitted += 1;
        }
        admitted
    };

    let dirs = admit_kind(true);
    let files = admit_kind(false);

    assert!(dirs <= 100, "{dirs}");
    assert!(dirs >= 90, "{dirs}");
    assert!(
        files > 2 * dirs,
        "a file costs far less than a directory: {files} against {dirs}"
    );
}
