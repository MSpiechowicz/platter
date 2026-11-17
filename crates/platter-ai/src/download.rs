//! Downloading the model: resumable, checked at every step, and only ever to its own folder.
//!
//! - The address is the spec's, `https` only. Redirects are followed here, one checked hop at a time
//!   (at most [`MAX_REDIRECTS`]); a hop to anything but `https` ends the download.
//! - The models folder is locked (see [`crate::lock`]) for the whole download, so two windows never fill
//!   one partial file. Free space is checked before the first request: what is still missing plus
//!   [`SPACE_HEADROOM`]. The bytes go to `<file>.partial` (0600, in a 0700
//!   folder), which is continued with a `Range` request after a stop. A server that answers `200`, or
//!   whose `Content-Range` does not match, makes the download start again from nothing.
//! - More bytes than the model has end the download and delete the partial file. The SHA-256 is computed
//!   while streaming (a partial file found at the start is hashed first); at the end size and checksum
//!   are checked, the file is synced, renamed into place and the folder synced. A wrong checksum deletes
//!   the partial file, so a wrong file never gets the model's name.
//! - Cancelling (or losing the connection) keeps the partial file for a later resume.
//! - The network is only touched on helper threads (one per request, one per body), and the download
//!   waits for them in short steps while it looks at the cancel flag. A cancel therefore takes effect
//!   within [`POLL`] even when the connection is stalled; the helper is abandoned and ends by itself
//!   when its blocked call returns (the timeouts of the [`Fetch`] implementation). An abandoned helper
//!   is counted ([`Helpers`]): a new download is refused while one is still alive, so repeated starts
//!   and cancels on a stalled network cannot pile threads up.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::fetch::{Fetch, FetchResponse};
use crate::lock::ModelLock;
use crate::spec::ModelSpec;
use crate::verify::hex;

/// Redirects followed for one request.
pub const MAX_REDIRECTS: usize = 5;
/// Bytes read and written at a time.
const CHUNK: usize = 64 * 1024;
/// How often a wait for the network looks at the cancel flag.
const POLL: Duration = Duration::from_millis(100);
/// Free space that must remain besides the bytes still to download.
pub const SPACE_HEADROOM: u64 = 1 << 30;

/// Why a download ended without the model. The text is for the user.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("The download was cancelled.")]
    Cancelled,
    #[error(
        "There is not enough free disk space: {needed} bytes are needed (the rest of the model and 1 GiB to spare) and {available} are free."
    )]
    NotEnoughSpace { needed: u64, available: u64 },
    /// The server or a redirect asked for something Platter does not do.
    #[error("{0}")]
    Refused(String),
    #[error("The connection failed: {0}")]
    Network(String),
    #[error("The server sent more data than the model has, so the download was stopped and deleted.")]
    TooLarge,
    #[error("The downloaded file is not the expected model ({0}), so it was deleted.")]
    Mismatch(&'static str),
    #[error("The model file cannot be written: {0}")]
    Io(String),
}

fn io_error(error: io::Error) -> DownloadError {
    DownloadError::Io(error.to_string())
}

/// How long a new download waits for helpers of an earlier one to end before it is refused. A helper
/// that was only about to return (a cancel that came just before its read finished) is gone by then;
/// one on a stalled network is not.
const HELPERS_GRACE: Duration = Duration::from_secs(1);

/// The helper threads that are blocked in a network call. Clones count the same threads.
#[derive(Debug, Clone)]
pub struct Helpers {
    live: Arc<AtomicUsize>,
    grace: Duration,
}

impl Default for Helpers {
    fn default() -> Self {
        Self::with_grace(HELPERS_GRACE)
    }
}

impl Helpers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Like [`Helpers::new`], with another time for a new download to wait for the helpers that are
    /// still alive (tests shorten it).
    pub fn with_grace(grace: Duration) -> Self {
        Self {
            live: Arc::default(),
            grace,
        }
    }

    /// Waits up to the grace time for every helper to end; whether none is alive.
    fn settle(&self) -> bool {
        let deadline = std::time::Instant::now() + self.grace;

        while self.live() > 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(2));
        }
        true
    }

    /// How many helpers are blocked in a network call now (abandoned by a cancel, or still being waited
    /// for).
    pub fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// Counts a helper from now until the returned value is dropped, which the helper does as soon as its
    /// blocked call returns, before it reports the result.
    fn begin(&self) -> Busy {
        self.live.fetch_add(1, Ordering::SeqCst);

        Busy {
            live: Arc::clone(&self.live),
        }
    }
}

/// One helper in [`Helpers`].
struct Busy {
    live: Arc<AtomicUsize>,
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Downloads `spec` into `dir` (created when missing). `progress(received, total)` is called after
/// every chunk, and once at the start with what is already there. `cancel` is looked at between chunks
/// and every [`POLL`] while the network does not answer. `helpers` counts the network threads of this and
/// earlier downloads: the download is refused while one is still alive after a short wait. It is refused
/// too while another process holds the lock of `dir`.
pub fn download(
    spec: &ModelSpec,
    dir: &Path,
    fetch: Arc<dyn Fetch>,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    helpers: &Helpers,
) -> Result<(), DownloadError> {
    if !helpers.settle() {
        return Err(DownloadError::Refused(
            "The previous download is still closing. Try again in a moment.".to_owned(),
        ));
    }
    platter_platform::create_private_dir(dir).map_err(io_error)?;

    // Held until the model is in place (or the download has ended some other way).
    let _lock = match ModelLock::try_acquire(dir).map_err(io_error)? {
        Some(lock) => lock,
        None => {
            return Err(DownloadError::Refused(
                "Another Platter window is downloading the model.".to_owned(),
            ));
        }
    };

    let target = dir.join(spec.file);
    let partial_path = dir.join(spec.partial_file());
    refuse_link(&target)?;
    refuse_link(&partial_path)?;

    let existing = std::fs::metadata(&partial_path).map_or(0, |meta| meta.len());
    check_space(dir, spec.bytes.saturating_sub(existing))?;

    let mut partial = Partial::open(&partial_path, spec.bytes, cancel)?;
    progress(partial.len, spec.bytes);

    let outcome = fill(spec, &fetch, &mut partial, cancel, progress, helpers);
    match outcome {
        Ok(()) => {}
        Err(error @ (DownloadError::TooLarge | DownloadError::Mismatch(_))) => {
            partial.discard();
            return Err(error);
        }
        Err(error) => return Err(error),
    }

    if partial.checksum() != spec.sha256 {
        partial.discard();
        return Err(DownloadError::Mismatch("its checksum is wrong"));
    }

    partial.file.sync_all().map_err(io_error)?;
    std::fs::rename(&partial.path, &target).map_err(io_error)?;
    sync_dir(dir).map_err(io_error)?;

    Ok(())
}

/// Makes the rename in `dir` durable.
#[cfg(not(windows))]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Windows cannot open a folder to sync it without backup semantics, and failing here would report a
/// download that is complete and verified as failed. The file itself was synced before the rename.
#[cfg(windows)]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Brings the partial file up to the model's size, asking the server for what is missing.
fn fill(
    spec: &ModelSpec,
    fetch: &Arc<dyn Fetch>,
    partial: &mut Partial,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    helpers: &Helpers,
) -> Result<(), DownloadError> {
    let mut restarted = false;

    while partial.len < spec.bytes {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }

        let from = (partial.len > 0).then_some(partial.len);
        let response = request(fetch, spec.url, from, cancel, helpers)?;

        match response.status {
            200 => {
                // Fresh, or the server ignored the `Range`: either way the body is the whole file.
                if from.is_some() {
                    partial.restart().map_err(io_error)?;
                }
                if response.content_length.is_some_and(|length| length != spec.bytes) {
                    return Err(DownloadError::Mismatch("the server announced another size"));
                }
            }
            206 => {
                if !range_matches(&response, from, spec.bytes) {
                    // Not the bytes we asked for: forget the partial file and ask for everything.
                    drop(response);
                    if restarted {
                        return Err(DownloadError::Refused(
                            "The server keeps answering with the wrong part of the file.".to_owned(),
                        ));
                    }
                    restarted = true;
                    partial.restart().map_err(io_error)?;
                    continue;
                }
            }
            416 if from.is_some() && !restarted => {
                restarted = true;
                partial.restart().map_err(io_error)?;
                continue;
            }
            status => {
                return Err(DownloadError::Refused(format!(
                    "The server answered {status} instead of sending the model."
                )));
            }
        }

        stream(response.body, partial, spec.bytes, cancel, progress, helpers)?;
        if partial.len < spec.bytes {
            return Err(DownloadError::Network("The connection closed early.".to_owned()));
        }
    }

    Ok(())
}

/// Copies the body to the partial file until it ends.
fn stream(
    body: Box<dyn Read + Send>,
    partial: &mut Partial,
    total: u64,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    helpers: &Helpers,
) -> Result<(), DownloadError> {
    let mut reader = BodyReader::start(body, helpers.clone())?;

    loop {
        let chunk = reader.next_chunk(cancel)?;
        if chunk.is_empty() {
            return Ok(());
        }

        if partial.len + chunk.len() as u64 > total {
            return Err(DownloadError::TooLarge);
        }
        partial.append(&chunk).map_err(io_error)?;
        progress(partial.len, total);
    }
}

/// Waits for `answer`, looking at `cancel` every [`POLL`]. A cancel wins over an answer that is ready.
fn wait<T>(answer: &Receiver<T>, cancel: &AtomicBool) -> Result<T, DownloadError> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }

        match answer.recv_timeout(POLL) {
            Ok(value) => return Ok(value),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(DownloadError::Network(
                    "The connection stopped unexpectedly.".to_owned(),
                ));
            }
        }
    }
}

/// A body that is read on its own thread, one chunk per request. Dropping the reader abandons a read
/// that is still blocked: the thread ends when that read returns.
struct BodyReader {
    wants: Sender<(Vec<u8>, Busy)>,
    chunks: Receiver<io::Result<Vec<u8>>>,
    helpers: Helpers,
}

impl BodyReader {
    fn start(mut body: Box<dyn Read + Send>, helpers: Helpers) -> Result<Self, DownloadError> {
        let (wants, wanted) = mpsc::channel::<(Vec<u8>, Busy)>();
        let (done, chunks) = mpsc::channel();

        thread::Builder::new()
            .name("platter-download-read".to_owned())
            .spawn(move || {
                for (mut buffer, busy) in wanted {
                    let result = loop {
                        match body.read(&mut buffer) {
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            other => break other,
                        }
                    };
                    // The blocked call is over; an idle reader ends by itself when it is dropped.
                    drop(busy);
                    let chunk = result.map(|read| {
                        buffer.truncate(read);
                        buffer
                    });

                    if done.send(chunk).is_err() {
                        break;
                    }
                }
            })
            .map_err(io_error)?;

        Ok(Self {
            wants,
            chunks,
            helpers,
        })
    }

    /// The next bytes of the body; empty when it has ended.
    fn next_chunk(&mut self, cancel: &AtomicBool) -> Result<Vec<u8>, DownloadError> {
        self.wants
            .send((vec![0u8; CHUNK], self.helpers.begin()))
            .map_err(|_| DownloadError::Network("The connection stopped unexpectedly.".to_owned()))?;

        wait(&self.chunks, cancel)?.map_err(|error| DownloadError::Network(error.to_string()))
    }
}

/// `GET url` (from the offset `from`), following up to [`MAX_REDIRECTS`] redirects, `https` only.
fn request(
    fetch: &Arc<dyn Fetch>,
    url: &str,
    from: Option<u64>,
    cancel: &AtomicBool,
    helpers: &Helpers,
) -> Result<FetchResponse, DownloadError> {
    let mut url = url.to_owned();

    for hop in 0..=MAX_REDIRECTS {
        require_https(&url)?;
        let response = get(fetch, &url, from, cancel, helpers)?;

        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if hop == MAX_REDIRECTS {
            return Err(DownloadError::Refused(format!(
                "The address redirects more than {MAX_REDIRECTS} times."
            )));
        }

        let Some(location) = response.location.as_deref() else {
            return Err(DownloadError::Refused(
                "The server redirected without saying where.".to_owned(),
            ));
        };
        url = resolve(&url, location)?;
    }

    unreachable!("the last hop returns or fails")
}

/// One `fetch.get` on a helper thread, abandoned when `cancel` is set while the server does not answer.
fn get(
    fetch: &Arc<dyn Fetch>,
    url: &str,
    from: Option<u64>,
    cancel: &AtomicBool,
    helpers: &Helpers,
) -> Result<FetchResponse, DownloadError> {
    let (done, answer) = mpsc::channel();
    let fetch = Arc::clone(fetch);
    let url = url.to_owned();
    let busy = helpers.begin();

    thread::Builder::new()
        .name("platter-download-get".to_owned())
        .spawn(move || {
            let answer = fetch.get(&url, from);
            // The blocked call is over: counted as gone before the answer can be seen.
            drop(busy);
            let _ = done.send(answer);
        })
        .map_err(io_error)?;

    wait(&answer, cancel)?.map_err(|error| DownloadError::Network(error.to_string()))
}

/// The absolute `https` address a redirect from `base` leads to.
fn resolve(base: &str, location: &str) -> Result<String, DownloadError> {
    let lower = location.to_ascii_lowercase();

    let next = if lower.starts_with("https://") {
        location.to_owned()
    } else if location.starts_with("//") {
        format!("https:{location}")
    } else if location.starts_with('/') {
        let authority_end = base["https://".len()..]
            .find(['/', '?', '#'])
            .map_or(base.len(), |at| at + "https://".len());
        format!("{}{location}", &base[..authority_end])
    } else {
        return Err(DownloadError::Refused(
            "The server redirected to an address that is not https.".to_owned(),
        ));
    };

    require_https(&next)?;
    Ok(next)
}

fn require_https(url: &str) -> Result<(), DownloadError> {
    let refused = |why: &str| DownloadError::Refused(format!("The download address is refused: {why}."));

    if !url
        .get(.."https://".len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        return Err(refused("it is not https"));
    }

    let rest = &url["https://".len()..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() {
        return Err(refused("it has no host"));
    }
    if authority.contains('@') {
        return Err(refused("it carries a user name"));
    }
    if url
        .chars()
        .any(|letter| letter.is_control() || letter.is_whitespace())
    {
        return Err(refused("it has odd characters"));
    }
    Ok(())
}

/// Whether a `206` answer is the part of the file that was asked for: `bytes <from>-<end>/<total>`.
fn range_matches(response: &FetchResponse, from: Option<u64>, total: u64) -> bool {
    let (Some(from), Some(header)) = (from, response.content_range.as_deref()) else {
        return false;
    };
    let Some(range) = header.trim().strip_prefix("bytes ") else {
        return false;
    };
    let Some((span, size)) = range.split_once('/') else {
        return false;
    };
    let Some((start, end)) = span.split_once('-') else {
        return false;
    };

    let (Ok(start), Ok(end), Ok(size)) = (
        start.trim().parse::<u64>(),
        end.trim().parse::<u64>(),
        size.trim().parse::<u64>(),
    ) else {
        return false;
    };

    start == from && size == total && end + 1 == total
}

/// A link where a file of ours goes is not followed and not replaced.
fn refuse_link(path: &Path) -> Result<(), DownloadError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(DownloadError::Refused(format!(
            "{} is a link and Platter does not follow it.",
            path.display()
        ))),
        Ok(_) | Err(_) => Ok(()),
    }
}

/// Whether the disk holding `dir` has room for `remaining` more bytes and [`SPACE_HEADROOM`] besides.
fn check_space(dir: &Path, remaining: u64) -> Result<(), DownloadError> {
    // Where the free space cannot be read, the writes themselves will say when the disk is full.
    enough_space(remaining, platter_platform::free_space(dir).ok())
}

fn enough_space(remaining: u64, available: Option<u64>) -> Result<(), DownloadError> {
    let needed = remaining.saturating_add(SPACE_HEADROOM);

    match available {
        Some(available) if available < needed => Err(DownloadError::NotEnoughSpace { needed, available }),
        Some(_) | None => Ok(()),
    }
}

/// The file being filled, its length, and the SHA-256 of everything in it.
struct Partial {
    file: File,
    path: PathBuf,
    len: u64,
    hasher: Sha256,
}

impl Partial {
    /// Opens (or creates) the partial file and hashes what it already holds. A file longer than the model
    /// cannot be a start of it and is emptied.
    fn open(path: &Path, total: u64, cancel: &AtomicBool) -> Result<Self, DownloadError> {
        let file = platter_platform::open_private_file(path).map_err(io_error)?;
        let len = file.metadata().map_err(io_error)?.len();

        let mut partial = Self {
            file,
            path: path.to_owned(),
            len,
            hasher: Sha256::new(),
        };
        if len > total {
            partial.restart().map_err(io_error)?;
            return Ok(partial);
        }

        partial.hash_existing(cancel)?;
        Ok(partial)
    }

    fn hash_existing(&mut self, cancel: &AtomicBool) -> Result<(), DownloadError> {
        let mut buffer = vec![0u8; CHUNK];
        let mut left = self.len;

        self.file.seek(SeekFrom::Start(0)).map_err(io_error)?;
        while left > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let want = buffer.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let read = self.file.read(&mut buffer[..want]).map_err(io_error)?;
            if read == 0 {
                // The file got shorter while it was read: start over from what is really there.
                self.restart().map_err(io_error)?;
                return Ok(());
            }
            self.hasher.update(&buffer[..read]);
            left -= read as u64;
        }
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.hasher.update(bytes);
        self.len += bytes.len() as u64;
        Ok(())
    }

    /// Back to an empty file.
    fn restart(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.hasher = Sha256::new();
        self.len = 0;
        Ok(())
    }

    fn checksum(&self) -> String {
        hex(&self.hasher.clone().finalize())
    }

    /// Deletes the file (what it holds is not usable).
    fn discard(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gibibyte_must_remain_besides_what_is_still_to_download() {
        let remaining = 5 * SPACE_HEADROOM;

        assert!(enough_space(remaining, Some(remaining + SPACE_HEADROOM)).is_ok());
        assert!(matches!(
            enough_space(remaining, Some(remaining + SPACE_HEADROOM - 1)),
            Err(DownloadError::NotEnoughSpace { needed, available })
                if needed == remaining + SPACE_HEADROOM && available == needed - 1
        ));
        assert!(
            enough_space(remaining, Some(remaining)).is_err(),
            "exactly the model is not enough"
        );
        assert!(
            enough_space(remaining, None).is_ok(),
            "unknown space is not a refusal"
        );
        assert!(enough_space(u64::MAX, Some(u64::MAX - 1)).is_err(), "no overflow");
    }
}
