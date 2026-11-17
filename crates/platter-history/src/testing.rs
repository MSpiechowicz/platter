//! Helpers for the tests of other crates.

use std::path::Path;

use rusqlite::Connection;

/// Another program that has the history file locked, until this is dropped.
pub struct Lock(#[expect(dead_code, reason = "held for as long as the lock lasts")] Connection);

/// Locks the history file at `path` (which must exist) so that nobody else can read or write it.
pub fn lock_exclusively(path: &Path) -> Lock {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("the file locks");

    Lock(connection)
}

/// How many trashed entries the history file at `path` still holds a path for (the original place or the
/// place in the trash). Read through a connection of its own.
pub fn entries_with_paths(path: &Path) -> u64 {
    let connection = Connection::open(path).expect("the file opens");
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM cleanup_item WHERE length(original) > 0 OR length(trashed) > 0",
            [],
            |row| row.get(0),
        )
        .expect("the entries are counted");

    u64::try_from(count).unwrap_or(0)
}

/// Appends `suffix` to the original path of every trashed entry in the history file at `path`, as
/// another program could while the app waits. Read and written through a connection of its own.
pub fn rename_originals(path: &Path, suffix: &[u8]) {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute(
            "UPDATE cleanup_item SET original = CAST(original || ?1 AS BLOB)",
            [suffix],
        )
        .expect("the paths are changed");
}

/// Appends `suffix` to the trash path of every trashed entry in the history file at `path`, as another
/// program could while the app waits.
pub fn rename_trashed(path: &Path, suffix: &[u8]) {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute(
            "UPDATE cleanup_item SET trashed = CAST(trashed || ?1 AS BLOB)",
            [suffix],
        )
        .expect("the paths are changed");
}

/// Gives every trashed entry in the history file at `path` another identity (inode), as if the stored
/// rows were of other files.
pub fn change_identities(path: &Path) {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute("UPDATE cleanup_item SET ino = ino + 1", [])
        .expect("the identities are changed");
}

/// Removes the first trashed entry of the history file at `path`, so a run has fewer entries than it had.
pub fn drop_first_entry(path: &Path) {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute(
            "DELETE FROM cleanup_item WHERE id = (SELECT MIN(id) FROM cleanup_item)",
            [],
        )
        .expect("the entry is removed");
}

/// Adds a copy of every trashed entry in the history file at `path`, so a run has more entries than it had.
pub fn duplicate_entries(path: &Path) {
    let connection = Connection::open(path).expect("the file opens");
    connection
        .execute(
            "INSERT INTO cleanup_item (cleanup_id, original, trashed, dev, ino, is_dir, allocated)
             SELECT cleanup_id, original || x'2d32', trashed || x'2d32', dev, ino + 1000, is_dir, allocated
             FROM cleanup_item",
            [],
        )
        .expect("the entries are copied");
}
