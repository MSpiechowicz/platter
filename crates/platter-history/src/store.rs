//! The history file: SQLite, opened without following a link, checked, migrated and written in
//! transactions. Every failure is a [`Problem`]: a plain sentence the app can show.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

/// The schema this build writes and reads (`PRAGMA user_version`).
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = include_str!("schema.sql");
const BUSY_TIMEOUT: Duration = Duration::from_millis(2000);

/// Why the history file cannot be used, as a sentence for people (no raw database text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    message: String,
    busy: bool,
}

impl Problem {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            busy: false,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Another program had the file locked: it may be usable later, unlike a damaged file.
    pub fn is_busy(&self) -> bool {
        self.busy
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Problem {}

impl From<rusqlite::Error> for Problem {
    fn from(error: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode::{DatabaseBusy, DatabaseCorrupt, DatabaseLocked, NotADatabase};

        let code = error.sqlite_error_code();
        let message = match code {
            Some(DatabaseBusy | DatabaseLocked) => "the history file is in use by another program",
            Some(DatabaseCorrupt | NotADatabase) => "the history file is damaged",
            _ => "the history file could not be used",
        };

        Self {
            message: message.to_owned(),
            busy: matches!(code, Some(DatabaseBusy | DatabaseLocked)),
        }
    }
}

pub type Result<T> = std::result::Result<T, Problem>;

/// How a cleanup run got rid of what it cleaned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MethodKind {
    Trash,
    Deleted,
    EmptiedTrash,
    Tool,
}

/// One method of a run. `command` is the command a tool ran, as text (`paccache -rk2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Method {
    pub kind: MethodKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// The disk a record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeRef<'a> {
    pub mount_point: &'a str,
    pub label: &'a str,
}

/// A volume as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVolume {
    pub id: i64,
    pub mount_point: String,
    pub label: String,
}

/// The used space of a volume on one local day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Unix seconds when it was read.
    pub at: i64,
    pub total: u64,
    pub used: u64,
}

/// A stored day of usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    pub at: i64,
    pub total: u64,
    pub used: u64,
}

/// What a finished scan found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanRecord {
    pub at: i64,
    pub allocated: u64,
    pub apparent: u64,
    pub files: u64,
    pub dirs: u64,
    pub errors: u64,
}

/// An entry a run moved to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    /// The exact path it had.
    pub original: Vec<u8>,
    /// The exact path it has in the trash.
    pub trashed: Vec<u8>,
    /// Device and inode, which the entry keeps when it is moved within a filesystem.
    pub identity: (u64, u64),
    pub is_dir: bool,
    pub allocated: u64,
}

/// A cleanup run to record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewCleanup {
    pub at: i64,
    /// Free space after minus before, when it could be measured.
    pub freed_measured: Option<i64>,
    /// What the run counts as freed (see `docs/architecture.md`, "History").
    pub freed: u64,
    /// The size of what the run moved to the trash.
    pub trashed: u64,
    pub cancelled: bool,
    pub emptied_trash: bool,
    /// The trash folders (their exact paths) this run emptied completely. Recording the run forgets the
    /// paths of the items of earlier runs that were in the `files` folder of one of them.
    pub emptied_trashes: Vec<Vec<u8>>,
    pub what: Vec<String>,
    pub methods: Vec<Method>,
    pub items: Vec<NewItem>,
}

/// A trashed entry as stored. Only an entry that may still be in the trash has its paths: those of one that
/// was put back, or that was found gone, are cleared (see [`Store::mark_restored`] and
/// [`Store::mark_gone`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredItem {
    pub id: i64,
    pub original: Vec<u8>,
    pub trashed: Vec<u8>,
    pub identity: (u64, u64),
    pub is_dir: bool,
    pub allocated: u64,
}

/// A cleanup run as stored. Of its trashed entries only a bounded number of those not put back are
/// loaded (see [`Store::cleanups_since`]), with the counts of all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCleanup {
    pub id: i64,
    pub at: i64,
    pub freed: u64,
    pub trashed: u64,
    pub what: Vec<String>,
    pub methods: Vec<Method>,
    /// Entries the run moved to the trash, put back or not.
    pub item_count: u64,
    /// Of those, the ones not put back yet.
    pub unrestored: u64,
    /// Of those, the ones found gone from the trash (their paths are cleared, so they are not loaded).
    pub gone: u64,
    /// Entries not put back yet and not gone, as many as the load had room for (at most
    /// `unrestored - gone`).
    pub items: Vec<StoredItem>,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens the history file at `path`, creating it when it does not exist. A link at `path` is refused
    /// and never followed, and so is anything but a regular file. A file that is damaged, in use, or
    /// made by a newer version is a [`Problem`], and is left as it is.
    pub fn open(path: &Path) -> Result<Self> {
        refuse_a_link(path)?;

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(path, flags)?;

        Self::prepare(connection)
    }

    /// A history in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> Result<Self> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        harden(&connection)?;

        // Reading the version is the first thing that touches the file, so a file that is not a
        // database fails here.
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Problem::new(
                "the history file was made by a newer version of Platter",
            ));
        }
        refuse_code_in_the_file(&connection)?;

        // Before any write: a path that is cleared must not stay readable in the free space of the file.
        require_secure_delete(&connection)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        quick_check(&connection)?;

        let mut store = Self { connection };
        if version < SCHEMA_VERSION {
            store.migrate()?;
        }

        Ok(store)
    }

    /// Version 0 (a new file) to version 1, all or nothing.
    fn migrate(&mut self) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;

        Ok(())
    }

    /// Records a finished scan.
    pub fn record_scan(&mut self, volume: VolumeRef<'_>, scan: &ScanRecord) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let volume_id = volume_id(&transaction, volume)?;
        transaction.execute(
            "INSERT INTO scan (volume_id, at, allocated, apparent, files, dirs, errors)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                volume_id,
                scan.at,
                stored(scan.allocated),
                stored(scan.apparent),
                stored(scan.files),
                stored(scan.dirs),
                stored(scan.errors)
            ],
        )?;

        Ok(transaction.commit()?)
    }

    /// Records the usage of `volume` on local day `day`, unless that day already has one. Returns
    /// whether this one was kept.
    pub fn record_sample(&mut self, volume: VolumeRef<'_>, day: &str, usage: Usage) -> Result<bool> {
        let transaction = self.connection.transaction()?;
        let volume_id = volume_id(&transaction, volume)?;
        let added = transaction.execute(
            "INSERT OR IGNORE INTO usage_sample (volume_id, day, at, total, used)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![volume_id, day, usage.at, stored(usage.total), stored(usage.used)],
        )?;
        transaction.commit()?;

        Ok(added > 0)
    }

    /// Records a cleanup run with the entries it moved to the trash, and returns its id.
    pub fn record_cleanup(&mut self, volume: VolumeRef<'_>, cleanup: &NewCleanup) -> Result<i64> {
        let what = serde_json::to_string(&cleanup.what)
            .map_err(|_| Problem::new("a cleanup could not be recorded"))?;
        let methods = serde_json::to_string(&cleanup.methods)
            .map_err(|_| Problem::new("a cleanup could not be recorded"))?;

        let transaction = self.connection.transaction()?;
        let volume_id = volume_id(&transaction, volume)?;
        transaction.execute(
            "INSERT INTO cleanup (volume_id, at, freed_measured, freed, trashed, cancelled, emptied_trash, what, methods)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                volume_id,
                cleanup.at,
                cleanup.freed_measured,
                stored(cleanup.freed),
                stored(cleanup.trashed),
                cleanup.cancelled,
                cleanup.emptied_trash,
                what,
                methods
            ],
        )?;
        let cleanup_id = transaction.last_insert_rowid();

        // Before this run's own items go in: they were trashed by this run, not emptied by it.
        for folder in &cleanup.emptied_trashes {
            forget_trashed_under(&transaction, folder)?;
        }

        {
            let mut insert = transaction.prepare(
                "INSERT INTO cleanup_item (cleanup_id, original, trashed, dev, ino, is_dir, allocated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for item in &cleanup.items {
                insert.execute(params![
                    cleanup_id,
                    item.original,
                    item.trashed,
                    item.identity.0 as i64,
                    item.identity.1 as i64,
                    item.is_dir,
                    stored(item.allocated)
                ])?;
            }
        }
        transaction.commit()?;

        Ok(cleanup_id)
    }

    /// The volume the history is shown for: the one of the latest finished scan, else the one with the
    /// newest sample, else the one of the latest cleanup.
    pub fn shown_volume(&self) -> Result<Option<StoredVolume>> {
        let queries = [
            "SELECT volume.id, volume.mount_point, volume.label FROM scan
             JOIN volume ON volume.id = scan.volume_id ORDER BY scan.at DESC, scan.id DESC LIMIT 1",
            "SELECT volume.id, volume.mount_point, volume.label FROM usage_sample
             JOIN volume ON volume.id = usage_sample.volume_id
             ORDER BY usage_sample.at DESC LIMIT 1",
            "SELECT volume.id, volume.mount_point, volume.label FROM cleanup
             JOIN volume ON volume.id = cleanup.volume_id ORDER BY cleanup.at DESC, cleanup.id DESC LIMIT 1",
        ];

        for query in queries {
            let found = self.connection.query_row(query, [], volume_of).optional()?;
            if found.is_some() {
                return Ok(found);
            }
        }

        Ok(None)
    }

    /// Every volume that has a record.
    pub fn volumes(&self) -> Result<Vec<StoredVolume>> {
        let mut query = self
            .connection
            .prepare("SELECT id, mount_point, label FROM volume ORDER BY id")?;
        let volumes = query.query_map([], volume_of)?.collect::<rusqlite::Result<_>>()?;

        Ok(volumes)
    }

    /// The samples of a volume from local day `from_day` (`YYYY-MM-DD`) on, oldest first.
    pub fn samples(&self, volume: i64, from_day: &str) -> Result<Vec<Sample>> {
        let mut query = self.connection.prepare(
            "SELECT day, at, total, used FROM usage_sample WHERE volume_id = ?1 AND day >= ?2 ORDER BY day",
        )?;
        let samples = query
            .query_map(params![volume, from_day], |row| {
                Ok(Sample {
                    day: row.get(0)?,
                    at: row.get(1)?,
                    total: unstored(row.get(2)?),
                    used: unstored(row.get(3)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(samples)
    }

    /// The cleanups of a volume that finished at `since` (Unix seconds) or later, newest first, each with
    /// the counts of its trashed entries. Of the entries not put back, `item_cap` are loaded in all (the
    /// newest runs first), so a long history costs a bounded read.
    pub fn cleanups_since(&self, volume: i64, since: i64, mut item_cap: usize) -> Result<Vec<StoredCleanup>> {
        let counts = self.item_counts(volume, since)?;
        let mut query = self.connection.prepare(
            "SELECT id, at, freed, trashed, what, methods FROM cleanup
             WHERE volume_id = ?1 AND at >= ?2 ORDER BY at DESC, id DESC",
        )?;
        let rows = query
            .query_map(params![volume, since], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut cleanups = Vec::with_capacity(rows.len());
        for (id, at, freed, trashed, what, methods) in rows {
            let (item_count, unrestored, gone) = counts.get(&id).copied().unwrap_or((0, 0, 0));
            let items = if unrestored > gone && item_cap > 0 {
                self.unrestored_items(id, item_cap)?
            } else {
                Vec::new()
            };
            item_cap -= items.len();

            cleanups.push(StoredCleanup {
                id,
                at,
                freed: unstored(freed),
                trashed: unstored(trashed),
                what: serde_json::from_str(&what).unwrap_or_default(),
                methods: serde_json::from_str(&methods).unwrap_or_default(),
                item_count,
                unrestored,
                gone,
                items,
            });
        }

        Ok(cleanups)
    }

    /// For each cleanup of the volume since `since`: how many entries it moved to the trash, how many of
    /// them are not put back, and of those how many were found gone (their paths are cleared).
    fn item_counts(&self, volume: i64, since: i64) -> Result<HashMap<i64, (u64, u64, u64)>> {
        let mut query = self.connection.prepare(
            "SELECT cleanup_item.cleanup_id, COUNT(*), COUNT(*) - COUNT(cleanup_item.restored_at),
                    COALESCE(SUM(cleanup_item.restored_at IS NULL AND length(cleanup_item.trashed) = 0), 0)
             FROM cleanup_item JOIN cleanup ON cleanup.id = cleanup_item.cleanup_id
             WHERE cleanup.volume_id = ?1 AND cleanup.at >= ?2
             GROUP BY cleanup_item.cleanup_id",
        )?;
        let counts = query
            .query_map(params![volume, since], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    (
                        unstored(row.get(1)?),
                        unstored(row.get(2)?),
                        unstored(row.get(3)?),
                    ),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(counts)
    }

    /// The entries of `cleanup` that have not been put back, if it is a cleanup of `volume` that finished at
    /// `since` or later. `None` when it is not (unknown, another volume, too old).
    pub fn items_to_restore(&self, cleanup: i64, volume: i64, since: i64) -> Result<Option<Vec<StoredItem>>> {
        let found = self
            .connection
            .query_row(
                "SELECT 1 FROM cleanup WHERE id = ?1 AND volume_id = ?2 AND at >= ?3",
                params![cleanup, volume, since],
                |_| Ok(()),
            )
            .optional()?;
        if found.is_none() {
            return Ok(None);
        }

        self.unrestored_items(cleanup, usize::MAX).map(Some)
    }

    /// The first `limit` entries of `cleanup` that have not been put back and are not known to be gone
    /// (those have no paths left).
    fn unrestored_items(&self, cleanup: i64, limit: usize) -> Result<Vec<StoredItem>> {
        let mut query = self.connection.prepare(
            "SELECT id, original, trashed, dev, ino, is_dir, allocated FROM cleanup_item
             WHERE cleanup_id = ?1 AND restored_at IS NULL AND length(trashed) > 0 ORDER BY id LIMIT ?2",
        )?;
        let items = query
            .query_map(params![cleanup, stored(limit as u64)], |row| {
                Ok(StoredItem {
                    id: row.get(0)?,
                    original: row.get(1)?,
                    trashed: row.get(2)?,
                    identity: (row.get::<_, i64>(3)? as u64, row.get::<_, i64>(4)? as u64),
                    is_dir: row.get(5)?,
                    allocated: unstored(row.get(6)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(items)
    }

    /// Marks entries (by their stored ids) as put back at `at`, and forgets their paths: nothing needs
    /// them once they are back. One already marked keeps its time.
    pub fn mark_restored(&mut self, items: &[i64], at: i64) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut mark = transaction.prepare(&format!(
                "UPDATE cleanup_item SET restored_at = ?2, {FORGET_PATHS} WHERE id = ?1 AND restored_at IS NULL"
            ))?;
            for item in items {
                mark.execute(params![item, at])?;
            }
        }

        Ok(transaction.commit()?)
    }

    /// Marks the entries of `cleanup` as put back at `at` and forgets their paths, found by the paths they
    /// had and have in the trash (for an undo, which knows paths and not ids).
    pub fn mark_restored_matching(
        &mut self,
        cleanup: i64,
        paths: &[(Vec<u8>, Vec<u8>)],
        at: i64,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut mark = transaction.prepare(&format!(
                "UPDATE cleanup_item SET restored_at = ?4, {FORGET_PATHS}
                 WHERE cleanup_id = ?1 AND original = ?2 AND trashed = ?3 AND restored_at IS NULL"
            ))?;
            for (original, trashed) in paths {
                mark.execute(params![cleanup, original, trashed, at])?;
            }
        }

        Ok(transaction.commit()?)
    }

    /// Forgets the paths of entries (by their stored ids) that were found gone from the trash. They stay
    /// counted as not put back (a row with no paths and no `restored_at` is a gone one), so their run is
    /// still shown as no longer in the trash. One that was put back meanwhile is left as it is.
    pub fn mark_gone(&mut self, items: &[i64]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut forget = transaction.prepare(&format!(
                "UPDATE cleanup_item SET {FORGET_PATHS} WHERE id = ?1 AND restored_at IS NULL"
            ))?;
            for item in items {
                forget.execute(params![item])?;
            }
        }

        Ok(transaction.commit()?)
    }
}

/// What an entry keeps of its paths once nothing needs them: nothing (an empty blob is valid in version 1,
/// where the columns are `NOT NULL`).
const FORGET_PATHS: &str = "original = x'', trashed = x''";

/// Forgets the paths of the entries not put back whose trash path is inside the `files` folder of the
/// trash folder `folder`, which was emptied.
fn forget_trashed_under(transaction: &Transaction<'_>, folder: &[u8]) -> Result<()> {
    let mut prefix = folder.to_vec();
    while prefix.len() > 1 && prefix.last() == Some(&b'/') {
        prefix.pop();
    }
    if prefix.is_empty() || prefix == b"/" {
        return Ok(());
    }
    prefix.extend_from_slice(b"/files/");

    // `substr` of a blob counts bytes.
    transaction.execute(
        &format!(
            "UPDATE cleanup_item SET {FORGET_PATHS} WHERE restored_at IS NULL AND substr(trashed, 1, ?2) = ?1"
        ),
        params![prefix, i64::try_from(prefix.len()).unwrap_or(i64::MAX)],
    )?;

    Ok(())
}

fn volume_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredVolume> {
    Ok(StoredVolume {
        id: row.get(0)?,
        mount_point: row.get(1)?,
        label: row.get(2)?,
    })
}

/// The id of `volume`, stored on first use; its label follows the latest one given.
fn volume_id(transaction: &Transaction<'_>, volume: VolumeRef<'_>) -> Result<i64> {
    let id = transaction.query_row(
        "INSERT INTO volume (mount_point, label) VALUES (?1, ?2)
         ON CONFLICT (mount_point) DO UPDATE SET label = excluded.label RETURNING id",
        params![volume.mount_point, volume.label],
        |row| row.get(0),
    )?;

    Ok(id)
}

/// A size as SQLite stores it: an integer of 63 bits.
fn stored(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unstored(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// A file that cannot run code of its own on this connection: no trusted schema (so views, triggers and
/// functions in it are not called), and SQLite's defensive mode (no writing to the schema tables).
fn harden(connection: &Connection) -> Result<()> {
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;

    Ok(())
}

/// Turns on `secure_delete`, so what is deleted or overwritten (the paths that are cleared, see
/// [`Store::mark_restored`]) is zeroed in the file and not left in its free space. The bundled SQLite is
/// built with it off, and a build that cannot honour the setting is a problem.
fn require_secure_delete(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "secure_delete", "ON")?;

    let on: i64 = connection.query_row("PRAGMA secure_delete", [], |row| row.get(0))?;
    if on != 1 {
        return Err(Problem::new(
            "the history file could not be set to erase what it forgets",
        ));
    }

    Ok(())
}

/// Platter's file holds tables and indexes only. A trigger or a view is something it did not put there,
/// and it would run (or be read through) in Platter's own queries.
fn refuse_code_in_the_file(connection: &Connection) -> Result<()> {
    let found: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('trigger', 'view')",
        [],
        |row| row.get(0),
    )?;
    if found > 0 {
        return Err(Problem::new(
            "the history file holds something Platter did not put there",
        ));
    }

    Ok(())
}

fn quick_check(connection: &Connection) -> Result<()> {
    let verdict: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if verdict != "ok" {
        return Err(Problem::new("the history file is damaged"));
    }

    Ok(())
}

/// A link at `path` is never followed, and a path that is not a regular file is not a database.
fn refuse_a_link(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Problem::new(
            "the history file is a link and Platter does not follow it",
        )),
        Ok(meta) if !meta.is_file() => Err(Problem::new("the history file is not a regular file")),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Problem::new("the history file could not be opened")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: VolumeRef<'static> = VolumeRef {
        mount_point: "/",
        label: "Root",
    };

    fn usage(at: i64, used: u64) -> Usage {
        Usage {
            at,
            total: 1000,
            used,
        }
    }

    fn item(name: &str) -> NewItem {
        NewItem {
            original: format!("/home/u/{name}").into_bytes(),
            trashed: format!("/trash/files/{name}").into_bytes(),
            // Numbers that need all 64 bits survive the round trip.
            identity: (u64::MAX - 1, u64::MAX / 2 + 5),
            is_dir: name.ends_with('/'),
            allocated: 4096,
        }
    }

    fn cleanup(at: i64, names: &[&str]) -> NewCleanup {
        NewCleanup {
            at,
            freed_measured: Some(-5),
            freed: 0,
            trashed: 8192,
            cancelled: false,
            emptied_trash: false,
            emptied_trashes: Vec::new(),
            what: vec!["Old downloads".to_owned()],
            methods: vec![Method {
                kind: MethodKind::Trash,
                command: None,
            }],
            items: names.iter().map(|name| item(name)).collect(),
        }
    }

    #[test]
    fn a_new_file_is_made_and_what_was_recorded_is_there_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");

        {
            let mut store = Store::open(&path).unwrap();
            store
                .record_scan(
                    ROOT,
                    &ScanRecord {
                        at: 10,
                        allocated: 5,
                        ..ScanRecord::default()
                    },
                )
                .unwrap();
            store.record_sample(ROOT, "2026-11-17", usage(11, 400)).unwrap();
            store.record_cleanup(ROOT, &cleanup(12, &["a", "b/"])).unwrap();
        }
        let store = Store::open(&path).unwrap();

        let volume = store.shown_volume().unwrap().unwrap();
        assert_eq!(
            (volume.mount_point.as_str(), volume.label.as_str()),
            ("/", "Root")
        );
        assert_eq!(store.samples(volume.id, "2026-01-01").unwrap().len(), 1);
        let cleanups = store.cleanups_since(volume.id, 0, 100).unwrap();
        assert_eq!(cleanups.len(), 1);
        let run = &cleanups[0];
        assert_eq!((run.at, run.freed, run.trashed), (12, 0, 8192));
        assert_eq!(run.what, ["Old downloads"]);
        assert_eq!(run.methods[0].kind, MethodKind::Trash);
        assert_eq!(run.items.len(), 2);
        assert_eq!(run.items[0].identity, (u64::MAX - 1, u64::MAX / 2 + 5));
        assert!(run.items[1].is_dir);
        assert_eq!((run.item_count, run.unrestored), (2, 2));
    }

    #[test]
    fn a_day_keeps_its_first_sample() {
        let mut store = Store::open_in_memory().unwrap();

        let first = store.record_sample(ROOT, "2026-11-17", usage(100, 400)).unwrap();
        let second = store.record_sample(ROOT, "2026-11-17", usage(200, 450)).unwrap();
        let next_day = store.record_sample(ROOT, "2026-11-18", usage(300, 460)).unwrap();

        assert_eq!((first, second, next_day), (true, false, true));
        let volume = store.volumes().unwrap().remove(0);
        let samples = store.samples(volume.id, "2026-11-18").unwrap();
        assert_eq!(samples.len(), 1, "from the given day on");
        let all = store.samples(volume.id, "2026-11-01").unwrap();
        assert_eq!((all[0].at, all[0].used), (100, 400));
    }

    #[test]
    fn the_shown_volume_is_the_one_scanned_last_else_sampled_last_else_cleaned_last() {
        let mut store = Store::open_in_memory().unwrap();
        let data = VolumeRef {
            mount_point: "/data",
            label: "Data",
        };
        assert_eq!(store.shown_volume().unwrap(), None);

        store.record_cleanup(data, &cleanup(1, &[])).unwrap();
        assert_eq!(
            store.shown_volume().unwrap().unwrap().mount_point,
            "/data",
            "a cleanup alone will do"
        );

        store.record_sample(ROOT, "2026-11-01", usage(1, 1)).unwrap();
        store.record_sample(data, "2026-11-02", usage(2, 1)).unwrap();
        assert_eq!(store.shown_volume().unwrap().unwrap().mount_point, "/data");

        store
            .record_scan(
                ROOT,
                &ScanRecord {
                    at: 3,
                    ..ScanRecord::default()
                },
            )
            .unwrap();
        assert_eq!(store.shown_volume().unwrap().unwrap().mount_point, "/");
    }

    #[test]
    fn entries_put_back_are_not_offered_again() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.record_cleanup(ROOT, &cleanup(1, &["a", "b", "c"])).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let items = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();
        assert_eq!(items.len(), 3);

        store.mark_restored(&[items[0].id], 50).unwrap();
        store
            .mark_restored_matching(id, &[(items[1].original.clone(), items[1].trashed.clone())], 60)
            .unwrap();

        let left = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].original, b"/home/u/c");
        let stored = store.cleanups_since(volume.id, 0, 100).unwrap();
        assert_eq!(
            (stored[0].item_count, stored[0].unrestored, stored[0].items.len()),
            (3, 1, 1),
            "a put back entry stays counted and is not loaded"
        );
    }

    #[test]
    fn a_cleanup_is_restorable_only_on_its_volume_and_inside_the_window() {
        let mut store = Store::open_in_memory().unwrap();
        let data = VolumeRef {
            mount_point: "/data",
            label: "Data",
        };
        let id = store.record_cleanup(ROOT, &cleanup(1000, &["a"])).unwrap();
        let root = store.volumes().unwrap().remove(0).id;
        store.record_cleanup(data, &cleanup(1000, &[])).unwrap();
        let other = store.volumes().unwrap()[1].id;

        assert!(store.items_to_restore(id, root, 1000).unwrap().is_some());
        assert!(
            store.items_to_restore(id, other, 0).unwrap().is_none(),
            "another volume"
        );
        assert!(
            store.items_to_restore(id, root, 1001).unwrap().is_none(),
            "too old"
        );
        assert!(
            store.items_to_restore(id + 99, root, 0).unwrap().is_none(),
            "unknown"
        );
    }

    #[test]
    fn only_a_bounded_number_of_unrestored_entries_is_loaded_and_the_newest_runs_come_first() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .record_cleanup(ROOT, &cleanup(1, &["old1", "old2", "old3"]))
            .unwrap();
        store
            .record_cleanup(ROOT, &cleanup(2, &["new1", "new2"]))
            .unwrap();
        let volume = store.volumes().unwrap().remove(0);

        let runs = store.cleanups_since(volume.id, 0, 3).unwrap();

        assert_eq!(runs[0].at, 2);
        assert_eq!((runs[0].items.len(), runs[0].unrestored), (2, 2));
        assert_eq!(
            (runs[1].items.len(), runs[1].unrestored, runs[1].item_count),
            (1, 3, 3),
            "the rest is counted, not loaded"
        );
    }

    #[test]
    fn a_file_made_by_a_newer_version_is_a_problem_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        {
            let connection = Connection::open(&path).unwrap();
            connection.pragma_update(None, "user_version", 2).unwrap();
        }

        let Err(problem) = Store::open(&path) else {
            panic!("a newer file must not be used");
        };

        assert!(problem.message().contains("newer version"), "{problem}");
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn a_file_that_is_not_a_database_is_a_problem_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let garbage =
            b"this is not a database, it is a long enough text to look like a damaged page".repeat(80);
        std::fs::write(&path, &garbage).unwrap();

        let Err(problem) = Store::open(&path) else {
            panic!("garbage must not be used");
        };

        assert!(problem.message().contains("damaged"), "{problem}");
        assert_eq!(std::fs::read(&path).unwrap(), garbage);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.sqlite");
        Store::open(&real).unwrap();
        let link = dir.path().join("history.sqlite");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let Err(problem) = Store::open(&link) else {
            panic!("a link must be refused");
        };

        assert!(problem.message().contains("link"), "{problem}");
    }

    #[test]
    fn a_folder_is_not_a_history_file() {
        let dir = tempfile::tempdir().unwrap();

        assert!(Store::open(dir.path()).is_err());
    }

    #[test]
    fn a_file_another_program_holds_is_a_problem_after_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        Store::open(&path).unwrap();
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let Err(problem) = Store::open(&path) else {
            panic!("a locked file must not be used");
        };

        assert!(problem.message().contains("in use"), "{problem}");
        assert!(problem.is_busy(), "worth trying again later");
    }

    #[test]
    fn a_damaged_or_newer_file_is_not_worth_trying_again() {
        assert!(!Problem::new("the history file is damaged").is_busy());
        assert!(
            !Problem::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                None
            ))
            .is_busy()
        );
    }

    #[test]
    fn a_file_with_a_trigger_or_a_view_in_it_is_a_problem_and_the_trigger_never_runs() {
        for planted in [
            "CREATE TRIGGER wipe AFTER INSERT ON scan BEGIN DELETE FROM cleanup; END",
            "CREATE VIEW sneaky AS SELECT 1",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("history.sqlite");
            {
                let mut store = Store::open(&path).unwrap();
                store.record_cleanup(ROOT, &cleanup(1, &["a"])).unwrap();
            }
            Connection::open(&path).unwrap().execute_batch(planted).unwrap();

            let Err(problem) = Store::open(&path) else {
                panic!("a file with {planted} must not be used");
            };

            assert!(problem.message().contains("did not put there"), "{problem}");
            let check = Connection::open(&path).unwrap();
            let cleanups: i64 = check
                .query_row("SELECT COUNT(*) FROM cleanup", [], |row| row.get(0))
                .unwrap();
            assert_eq!(cleanups, 1, "nothing ran or changed");
        }
    }

    #[test]
    fn a_connection_does_not_trust_the_schema_and_is_defensive() {
        let store = Store::open_in_memory().unwrap();

        let trusted: i64 = store
            .connection
            .query_row("PRAGMA trusted_schema", [], |row| row.get(0))
            .unwrap();
        let writable = store
            .connection
            .execute_batch("PRAGMA writable_schema = ON; UPDATE sqlite_master SET name = name");

        assert_eq!(trusted, 0);
        assert!(writable.is_err(), "the schema tables cannot be written to");
    }

    #[test]
    fn a_problem_says_nothing_of_the_database_internals() {
        for message in [
            Problem::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                Some("database disk image is malformed; page 4".to_owned()),
            ))
            .message()
            .to_owned(),
            Problem::from(rusqlite::Error::InvalidQuery).message().to_owned(),
        ] {
            assert!(!message.contains(';') && !message.contains("page"), "{message}");
        }
    }

    /// What the file holds of the paths of the entries of `cleanup`: how many rows still have a path.
    fn rows_with_paths(store: &Store, cleanup: i64) -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM cleanup_item WHERE cleanup_id = ?1 AND (length(original) > 0 OR length(trashed) > 0)",
                params![cleanup],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn run_of(store: &Store, id: i64) -> StoredCleanup {
        let volume = store.volumes().unwrap().remove(0);
        store
            .cleanups_since(volume.id, 0, 100)
            .unwrap()
            .into_iter()
            .find(|run| run.id == id)
            .unwrap()
    }

    #[test]
    fn an_entry_put_back_keeps_no_path_and_the_others_keep_theirs() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.record_cleanup(ROOT, &cleanup(1, &["a", "b", "c"])).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let items = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();

        store.mark_restored(&[items[0].id], 50).unwrap();
        store
            .mark_restored_matching(id, &[(items[1].original.clone(), items[1].trashed.clone())], 60)
            .unwrap();

        assert_eq!(rows_with_paths(&store, id), 1, "only the one still in the trash");
        let run = run_of(&store, id);
        assert_eq!((run.item_count, run.unrestored, run.gone), (3, 1, 0));
        assert_eq!(run.items[0].original, b"/home/u/c");
    }

    #[test]
    fn an_entry_found_gone_keeps_no_path_and_still_counts_as_not_put_back() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.record_cleanup(ROOT, &cleanup(1, &["a", "b"])).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let items = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();

        store.mark_gone(&[items[0].id]).unwrap();

        assert_eq!(rows_with_paths(&store, id), 1);
        let run = run_of(&store, id);
        assert_eq!((run.item_count, run.unrestored, run.gone), (2, 2, 1));
        assert_eq!(run.items.len(), 1, "a gone entry is not loaded");
        assert_eq!(run.items[0].original, b"/home/u/b");
        let to_restore = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();
        assert_eq!(to_restore.len(), 1, "and there is nothing to put back for it");
    }

    #[test]
    fn a_path_that_was_cleared_is_not_left_in_the_file_or_its_journal() {
        const PUT_BACK: &[u8] = b"/home/u/Zq7-distinct-put-back-path";
        const GONE: &[u8] = b"/home/u/Zq7-distinct-gone-path";
        const KEPT: &[u8] = b"/home/u/Zq7-distinct-kept-path";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let mut store = Store::open(&path).unwrap();
        let mut run = cleanup(1, &[]);
        run.items = [PUT_BACK, GONE, KEPT]
            .iter()
            .map(|original| NewItem {
                original: original.to_vec(),
                trashed: [*original, b"-in-the-trash".as_slice()].concat(),
                ..item("x")
            })
            .collect();
        let id = store.record_cleanup(ROOT, &run).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let items = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();

        store.mark_restored(&[items[0].id], 50).unwrap();
        store.mark_gone(&[items[1].id]).unwrap();
        drop(store);

        let bytes = std::fs::read(&path).unwrap();
        let found = |needle: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
        assert!(!found(PUT_BACK), "the path of the entry put back is gone");
        assert!(!found(GONE), "the path of the entry found gone is gone");
        assert!(found(KEPT), "what is still needed is kept");
        assert!(!dir.path().join("history.sqlite-journal").exists());
    }

    #[test]
    fn an_entry_put_back_is_not_made_gone_afterwards() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.record_cleanup(ROOT, &cleanup(1, &["a"])).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let items = store.items_to_restore(id, volume.id, 0).unwrap().unwrap();
        store.mark_restored(&[items[0].id], 50).unwrap();

        store.mark_gone(&[items[0].id]).unwrap();

        let run = run_of(&store, id);
        assert_eq!((run.unrestored, run.gone), (0, 0), "it stays put back");
    }

    #[test]
    fn an_entry_a_concurrent_restore_put_back_is_not_made_gone_and_keeps_its_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let mut forgetting = Store::open(&path).unwrap();
        let mut restoring = Store::open(&path).unwrap();
        let id = forgetting.record_cleanup(ROOT, &cleanup(1, &["a", "b"])).unwrap();
        let volume = forgetting.volumes().unwrap().remove(0);
        // The overview looked at both and found both gone.
        let found_gone: Vec<i64> = forgetting
            .items_to_restore(id, volume.id, 0)
            .unwrap()
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect();

        // Before it writes, a restore (another connection) puts the first one back.
        restoring.mark_restored(&found_gone[..1], 50).unwrap();
        forgetting.mark_gone(&found_gone).unwrap();

        let mut read = forgetting
            .connection
            .prepare("SELECT restored_at FROM cleanup_item ORDER BY id")
            .unwrap();
        let restored_at: Vec<Option<i64>> = read
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        drop(read);
        assert_eq!(restored_at, [Some(50), None], "the put back one keeps its time");
        let run = run_of(&forgetting, id);
        assert_eq!(
            (run.item_count, run.unrestored, run.gone),
            (2, 1, 1),
            "one put back and one gone: not both gone"
        );
        assert_eq!(rows_with_paths(&forgetting, id), 0);
    }

    #[test]
    fn a_run_that_emptied_the_trash_forgets_the_paths_inside_that_trash_and_only_those() {
        let mut store = Store::open_in_memory().unwrap();
        let in_trash = store.record_cleanup(ROOT, &cleanup(1, &["a", "b"])).unwrap();
        // An entry in another trash, and one that only has a name that starts like the trash folder.
        let elsewhere = NewItem {
            trashed: b"/mnt/data/.Trash-1000/files/x".to_vec(),
            ..item("x")
        };
        let lookalike = NewItem {
            trashed: b"/trash/files-old/y".to_vec(),
            ..item("y")
        };
        let other = store
            .record_cleanup(
                ROOT,
                &NewCleanup {
                    items: vec![elsewhere, lookalike],
                    ..cleanup(2, &[])
                },
            )
            .unwrap();
        let put_back = store.record_cleanup(ROOT, &cleanup(3, &["z"])).unwrap();
        let volume = store.volumes().unwrap().remove(0);
        let z = store.items_to_restore(put_back, volume.id, 0).unwrap().unwrap();
        store.mark_restored(&[z[0].id], 4).unwrap();

        // The run also trashed an entry into that trash: it was trashed by the run, not emptied by it.
        let emptying = store
            .record_cleanup(
                ROOT,
                &NewCleanup {
                    emptied_trashes: vec![b"/trash/".to_vec()],
                    ..cleanup(5, &["late"])
                },
            )
            .unwrap();

        assert_eq!(rows_with_paths(&store, in_trash), 0);
        let run = run_of(&store, in_trash);
        assert_eq!(
            (run.item_count, run.unrestored, run.gone, run.items.len()),
            (2, 2, 2, 0)
        );
        assert_eq!(
            rows_with_paths(&store, other),
            2,
            "another trash, and a lookalike name"
        );
        assert_eq!(rows_with_paths(&store, emptying), 1);
        assert_eq!(rows_with_paths(&store, put_back), 0);
    }

    #[test]
    fn a_trash_folder_that_is_the_root_forgets_nothing() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.record_cleanup(ROOT, &cleanup(1, &["a"])).unwrap();

        store
            .record_cleanup(
                ROOT,
                &NewCleanup {
                    emptied_trashes: vec![b"/".to_vec(), Vec::new()],
                    ..cleanup(2, &[])
                },
            )
            .unwrap();

        assert_eq!(rows_with_paths(&store, id), 1);
    }
}
