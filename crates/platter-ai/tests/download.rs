//! The downloader and the model check, against a scripted server and a tiny stand-in for the model.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::testing::{FakeFetch, FakeReply, Gate, tiny_spec};
use platter_ai::{DownloadError, Fetch, Helpers, ModelLock, ModelSpec, VerifyError, download, verify};

const URL: &str = "https://models.example/tiny.gguf";

fn content() -> Vec<u8> {
    (0..300u32).map(|n| (n * 7 % 251) as u8).collect()
}

struct Setup {
    _root: tempfile::TempDir,
    dir: PathBuf,
    spec: ModelSpec,
    content: Vec<u8>,
}

impl Setup {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let content = content();

        Self {
            dir: root.path().join("models"),
            spec: tiny_spec(&content),
            content,
            _root: root,
        }
    }

    fn model(&self) -> PathBuf {
        self.dir.join(self.spec.file)
    }

    fn partial(&self) -> PathBuf {
        self.dir.join(self.spec.partial_file())
    }

    /// A partial file from an earlier run that holds the first `bytes` of the model.
    fn leave_partial(&self, bytes: usize) {
        fs::create_dir_all(&self.dir).unwrap();
        fs::write(self.partial(), &self.content[..bytes]).unwrap();
    }

    fn run(&self, fetch: &Arc<FakeFetch>) -> Result<(), DownloadError> {
        download(
            &self.spec,
            &self.dir,
            Arc::clone(fetch) as Arc<dyn Fetch>,
            &AtomicBool::new(false),
            &|_, _| {},
            &Helpers::new(),
        )
    }

    fn assert_installed(&self) {
        assert_eq!(fs::read(self.model()).unwrap(), self.content);
        assert!(!self.partial().exists(), "the partial file is renamed away");
    }
}

fn fake(replies: impl IntoIterator<Item = FakeReply>) -> Arc<FakeFetch> {
    Arc::new(FakeFetch::new(replies))
}

fn urls(fetch: &FakeFetch) -> Vec<String> {
    fetch.requests().into_iter().map(|request| request.url).collect()
}

fn offsets(fetch: &FakeFetch) -> Vec<Option<u64>> {
    fetch.requests().into_iter().map(|request| request.from).collect()
}

#[test]
fn a_fresh_download_arrives_checked_and_private() {
    let setup = Setup::new();
    let fetch = fake([FakeReply::ok(&setup.content)]);
    let seen = Mutex::new(Vec::new());

    download(
        &setup.spec,
        &setup.dir,
        Arc::clone(&fetch) as Arc<dyn Fetch>,
        &AtomicBool::new(false),
        &|received, total| {
            seen.lock().unwrap().push((received, total));
        },
        &Helpers::new(),
    )
    .unwrap();

    setup.assert_installed();
    assert_eq!(fetch.requests().len(), 1);
    assert_eq!(offsets(&fetch), [None]);
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.first(), Some(&(0, 300)));
    assert_eq!(seen.last(), Some(&(300, 300)));
    assert!(seen.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&setup.dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(setup.model()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn only_the_pinned_address_is_asked_and_only_for_a_range_when_resuming() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([FakeReply::partial(&setup.content[100..], 100, 300)]);

    setup.run(&fetch).unwrap();

    assert_eq!(urls(&fetch), [URL]);
    assert_eq!(offsets(&fetch), [Some(100)]);
}

#[test]
fn a_partial_file_is_continued_with_a_range_and_the_whole_is_checked() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let seen = Mutex::new(Vec::new());
    let fetch = fake([FakeReply::partial(&setup.content[100..], 100, 300)]);

    download(
        &setup.spec,
        &setup.dir,
        Arc::clone(&fetch) as Arc<dyn Fetch>,
        &AtomicBool::new(false),
        &|received, _| {
            seen.lock().unwrap().push(received);
        },
        &Helpers::new(),
    )
    .unwrap();

    setup.assert_installed();
    assert_eq!(
        seen.lock().unwrap().first(),
        Some(&100),
        "it starts from what is there"
    );
}

#[test]
fn a_continued_download_with_a_wrong_start_fails_the_checksum_and_leaves_nothing() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.dir).unwrap();
    fs::write(setup.partial(), vec![0u8; 100]).unwrap();
    let fetch = fake([FakeReply::partial(&setup.content[100..], 100, 300)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Mismatch(_)), "{error}");
    assert!(!setup.partial().exists());
    assert!(!setup.model().exists());
}

#[test]
fn a_server_that_answers_200_to_a_range_starts_the_file_again() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([FakeReply::ok(&setup.content)]);

    setup.run(&fetch).unwrap();

    setup.assert_installed();
    assert_eq!(offsets(&fetch), [Some(100)]);
}

#[test]
fn a_content_range_that_does_not_match_starts_again_from_nothing() {
    for wrong in [
        "bytes 50-299/300",
        "bytes 100-299/999",
        "bytes 100-199/300",
        "bytes 100-299/*",
        "items 100-299/300",
        "garbage",
    ] {
        let setup = Setup::new();
        setup.leave_partial(100);
        let fetch = fake([
            FakeReply::partial(&setup.content[100..], 100, 300).content_range(wrong),
            FakeReply::ok(&setup.content),
        ]);

        setup
            .run(&fetch)
            .unwrap_or_else(|error| panic!("{wrong}: {error}"));

        setup.assert_installed();
        assert_eq!(offsets(&fetch), [Some(100), None], "{wrong}");
    }
}

#[test]
fn a_range_the_server_cannot_serve_starts_again_but_only_once() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([FakeReply::status(416), FakeReply::ok(&setup.content)]);

    setup.run(&fetch).unwrap();

    setup.assert_installed();
    assert_eq!(offsets(&fetch), [Some(100), None]);

    let again = Setup::new();
    again.leave_partial(100);
    let stubborn = fake([
        FakeReply::partial(&again.content[100..], 100, 300).content_range("garbage"),
        FakeReply::partial(&again.content[100..], 100, 300).content_range("garbage"),
        FakeReply::ok(&again.content),
    ]);
    // The second wrong answer is the first after a restart (no offset is asked for any more): it is a
    // `206` to a request that had no `Range`, which is refused.
    assert!(matches!(again.run(&stubborn), Err(DownloadError::Refused(_))));
}

#[test]
fn a_redirect_to_http_is_refused() {
    for target in [
        "http://cdn.example/tiny.gguf",
        "ftp://cdn.example/tiny.gguf",
        "https://user:secret@cdn.example/tiny.gguf",
        "tiny.gguf",
    ] {
        let setup = Setup::new();
        let fetch = fake([FakeReply::redirect(target), FakeReply::ok(&setup.content)]);

        let error = setup.run(&fetch).unwrap_err();

        assert!(matches!(error, DownloadError::Refused(_)), "{target}: {error}");
        assert_eq!(fetch.requests().len(), 1, "{target}: nothing is fetched from it");
        assert!(!setup.model().exists());
    }
}

#[test]
fn an_address_that_is_not_https_is_never_asked() {
    let mut setup = Setup::new();
    setup.spec.url = "http://models.example/tiny.gguf";
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Refused(_)));
    assert!(fetch.requests().is_empty());
}

#[test]
fn five_redirects_are_followed_and_a_sixth_is_refused() {
    let setup = Setup::new();
    let mut five: Vec<_> = (1..=5)
        .map(|hop| FakeReply::redirect(&format!("https://cdn.example/{hop}")))
        .collect();
    five.push(FakeReply::ok(&setup.content));
    let fetch = fake(five);

    setup.run(&fetch).unwrap();

    setup.assert_installed();
    assert_eq!(fetch.requests().len(), 6);
    assert_eq!(urls(&fetch)[5], "https://cdn.example/5");

    let looping = Setup::new();
    let mut six: Vec<_> = (1..=6)
        .map(|hop| FakeReply::redirect(&format!("https://cdn.example/{hop}")))
        .collect();
    six.push(FakeReply::ok(&looping.content));
    let fetch = fake(six);

    let error = looping.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Refused(_)), "{error}");
    assert_eq!(fetch.requests().len(), 6, "the sixth redirect is not followed");
    assert!(!looping.model().exists());
}

#[test]
fn a_redirect_to_a_path_stays_on_the_same_host_and_keeps_the_range() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([
        FakeReply::redirect("/moved/tiny.gguf"),
        FakeReply::redirect("//cdn.example/part"),
        FakeReply::partial(&setup.content[100..], 100, 300),
    ]);

    setup.run(&fetch).unwrap();

    assert_eq!(
        urls(&fetch),
        [
            URL,
            "https://models.example/moved/tiny.gguf",
            "https://cdn.example/part"
        ]
    );
    assert_eq!(offsets(&fetch), [Some(100); 3]);
}

#[test]
fn more_data_than_the_model_has_aborts_and_deletes_the_partial_file() {
    let setup = Setup::new();
    let mut too_much = setup.content.clone();
    too_much.extend([1u8; 40]);
    let fetch = fake([FakeReply::ok(&too_much).content_length(None)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::TooLarge), "{error}");
    assert!(!setup.partial().exists());
    assert!(!setup.model().exists());
}

#[test]
fn a_server_that_announces_another_size_is_refused_before_anything_is_written() {
    let setup = Setup::new();
    let fetch = fake([FakeReply::ok(&setup.content).content_length(Some(301))]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Mismatch(_)), "{error}");
    assert!(!setup.partial().exists());
    assert!(!setup.model().exists());
}

#[test]
fn a_wrong_checksum_deletes_the_partial_file_and_the_model_never_appears() {
    let setup = Setup::new();
    let other: Vec<u8> = setup.content.iter().map(|byte| byte ^ 0xFF).collect();
    let fetch = fake([FakeReply::ok(&other)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Mismatch(_)), "{error}");
    assert!(!setup.partial().exists());
    assert!(!setup.model().exists());
}

#[test]
fn a_cut_connection_keeps_the_partial_file_and_the_next_try_continues_it() {
    let setup = Setup::new();
    let fetch = fake([FakeReply::ok(&setup.content).fail_after(120)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Network(_)), "{error}");
    assert_eq!(fs::read(setup.partial()).unwrap(), setup.content[..120]);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(setup.partial()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!setup.model().exists());

    let again = fake([FakeReply::partial(&setup.content[120..], 120, 300)]);
    setup.run(&again).unwrap();
    setup.assert_installed();
    assert_eq!(offsets(&again), [Some(120)]);
}

#[test]
fn a_body_that_ends_early_is_a_stop_not_a_finished_file() {
    let setup = Setup::new();
    let fetch = fake([FakeReply::ok(&setup.content[..100]).content_length(None)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Network(_)), "{error}");
    assert_eq!(fs::read(setup.partial()).unwrap(), setup.content[..100]);
    assert!(!setup.model().exists());
}

#[test]
fn cancelling_keeps_the_partial_file_and_the_download_can_be_resumed() {
    let setup = Setup::new();
    let cancel = AtomicBool::new(false);
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = download(
        &setup.spec,
        &setup.dir,
        Arc::clone(&fetch) as Arc<dyn Fetch>,
        &cancel,
        &|received, _| {
            if received >= 64 {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &Helpers::new(),
    )
    .unwrap_err();

    assert!(matches!(error, DownloadError::Cancelled), "{error}");
    let kept = fs::read(setup.partial()).unwrap();
    assert!((64..300).contains(&kept.len()));
    assert_eq!(kept, setup.content[..kept.len()]);
    assert!(!setup.model().exists());

    let again = fake([FakeReply::partial(
        &setup.content[kept.len()..],
        kept.len() as u64,
        300,
    )]);
    setup.run(&again).unwrap();
    setup.assert_installed();
}

#[test]
fn a_download_cancelled_before_it_starts_asks_for_nothing() {
    let setup = Setup::new();
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = download(
        &setup.spec,
        &setup.dir,
        Arc::clone(&fetch) as Arc<dyn Fetch>,
        &AtomicBool::new(true),
        &|_, _| {},
        &Helpers::new(),
    )
    .unwrap_err();

    assert!(matches!(error, DownloadError::Cancelled));
    assert!(fetch.requests().is_empty());
}

#[test]
fn too_little_free_space_is_reported_before_any_request() {
    let mut setup = Setup::new();
    // More than any disk has; the stand-in's checksum does not matter, nothing is asked.
    setup.spec.bytes = 1 << 62;
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::NotEnoughSpace { .. }), "{error}");
    assert!(error.to_string().contains("free disk space"));
    assert!(fetch.requests().is_empty());
    assert!(!setup.partial().exists());
}

#[test]
fn the_models_folder_is_locked_by_one_holder_at_a_time() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.dir).unwrap();

    let first = ModelLock::try_acquire(&setup.dir).unwrap();
    let second = ModelLock::try_acquire(&setup.dir).unwrap();

    assert!(first.is_some());
    assert!(second.is_none(), "a second holder is turned away");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(setup.dir.join(".lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    drop(first);

    assert!(
        ModelLock::try_acquire(&setup.dir).unwrap().is_some(),
        "free again"
    );
}

#[test]
fn a_download_is_refused_while_another_window_holds_the_folder_and_asks_for_nothing() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([FakeReply::partial(&setup.content[100..], 100, 300)]);
    let other_window = ModelLock::try_acquire(&setup.dir).unwrap().unwrap();

    let error = setup.run(&fetch).unwrap_err();

    assert!(
        matches!(&error, DownloadError::Refused(text) if text == "Another Platter window is downloading the model."),
        "{error}"
    );
    assert!(fetch.requests().is_empty());
    assert_eq!(fs::read(setup.partial()).unwrap(), setup.content[..100]);

    drop(other_window);
    setup.run(&fetch).unwrap();
    setup.assert_installed();
}

#[test]
fn a_finished_download_lets_go_of_the_folder() {
    let setup = Setup::new();

    setup.run(&fake([FakeReply::ok(&setup.content)])).unwrap();

    assert!(ModelLock::try_acquire(&setup.dir).unwrap().is_some());
}

#[test]
fn starting_and_cancelling_on_a_stalled_network_leaves_at_most_one_helper() {
    let setup = Setup::new();
    let gate = Gate::new();
    let fetch = Arc::new(FakeFetch::new([]).then_serve(&setup.content).stall_at(&gate));
    let helpers = Helpers::with_grace(Duration::from_millis(5));
    let cancel = AtomicBool::new(false);
    let start = || {
        download(
            &setup.spec,
            &setup.dir,
            Arc::clone(&fetch) as Arc<dyn Fetch>,
            &cancel,
            &|_, _| {},
            &helpers,
        )
    };

    // The first download reaches the server, which never answers; the cancel returns at once.
    let first = thread::scope(|scope| {
        let running = scope.spawn(start);
        assert!(
            gate.wait_for_visitor(Duration::from_secs(10)),
            "the request is out"
        );
        cancel.store(true, Ordering::SeqCst);

        running.join().unwrap()
    });
    assert!(matches!(first, Err(DownloadError::Cancelled)), "{first:?}");
    assert_eq!(helpers.live(), 1, "the stalled request is abandoned, still alive");

    for _ in 0..100 {
        cancel.store(false, Ordering::SeqCst);
        let again = start();

        assert!(
            matches!(&again, Err(DownloadError::Refused(text)) if text.contains("still closing")),
            "{again:?}"
        );
        assert!(helpers.live() <= 1);
    }
    assert_eq!(fetch.requests().len(), 1, "no other request was made");

    // The network answers at last: the helper ends, and the next download goes through.
    gate.open();
    let started = Instant::now();
    while helpers.live() > 0 {
        assert!(started.elapsed() < Duration::from_secs(10), "the helper ended");
        thread::sleep(Duration::from_millis(2));
    }
    cancel.store(false, Ordering::SeqCst);
    start().unwrap();
    setup.assert_installed();
    assert_eq!(helpers.live(), 0);
}

#[cfg(unix)]
#[test]
fn a_link_at_the_model_path_is_refused_and_not_followed() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.dir).unwrap();
    let victim = setup.dir.join("victim");
    fs::write(&victim, b"mine").unwrap();
    symlink(&victim, setup.model()).unwrap();
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Refused(_)), "{error}");
    assert!(fetch.requests().is_empty());
    assert_eq!(fs::read(&victim).unwrap(), b"mine");
}

#[cfg(unix)]
#[test]
fn a_link_at_the_partial_path_is_refused_and_the_file_it_points_to_is_untouched() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.dir).unwrap();
    let victim = setup.dir.join("victim");
    fs::write(&victim, b"mine").unwrap();
    symlink(&victim, setup.partial()).unwrap();
    let fetch = fake([FakeReply::ok(&setup.content)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(matches!(error, DownloadError::Refused(_)), "{error}");
    assert!(fetch.requests().is_empty());
    assert_eq!(fs::read(&victim).unwrap(), b"mine");
}

#[test]
fn a_finished_partial_file_is_checked_and_installed_without_asking_the_server() {
    let setup = Setup::new();
    setup.leave_partial(300);
    let fetch = fake([]);

    setup.run(&fetch).unwrap();

    setup.assert_installed();
    assert!(fetch.requests().is_empty());
}

#[test]
fn a_partial_file_longer_than_the_model_cannot_be_a_start_of_it() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.dir).unwrap();
    fs::write(setup.partial(), vec![9u8; 500]).unwrap();
    let fetch = fake([FakeReply::ok(&setup.content)]);

    setup.run(&fetch).unwrap();

    setup.assert_installed();
    assert_eq!(offsets(&fetch), [None]);
}

#[test]
fn a_server_error_is_reported_as_plain_text_and_keeps_the_partial_file() {
    let setup = Setup::new();
    setup.leave_partial(100);
    let fetch = fake([FakeReply::status(503)]);

    let error = setup.run(&fetch).unwrap_err();

    assert!(error.to_string().contains("503"), "{error}");
    assert_eq!(fs::read(setup.partial()).unwrap(), setup.content[..100]);
}

fn install(setup: &Setup, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(&setup.dir).unwrap();
    fs::write(setup.model(), bytes).unwrap();
    setup.model()
}

#[cfg(unix)]
#[test]
fn the_right_file_is_verified_and_can_be_read_through_its_pinned_path() {
    let setup = Setup::new();
    let path = install(&setup, &setup.content);

    let verified = verify(&setup.spec, &path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, vec![0u8; 300]).unwrap();

    assert_eq!(
        fs::read(verified.path().unwrap()).unwrap(),
        setup.content,
        "what is loaded is what was checked, whatever the name points to now"
    );
}

/// Windows has no name that follows a handle: the file is held with a share mode that lets nobody write,
/// rename or delete it, so the path names the checked bytes for as long as the check's result lives.
#[cfg(windows)]
#[test]
fn the_right_file_is_verified_and_nobody_can_change_it_while_the_result_is_held() {
    let setup = Setup::new();
    let path = install(&setup, &setup.content);

    let verified = verify(&setup.spec, &path).unwrap();
    let removed = fs::remove_file(&path);
    let written = fs::write(&path, vec![0u8; 300]);
    let renamed = fs::rename(&path, setup.dir.join("moved.gguf"));

    assert!(removed.is_err() && written.is_err() && renamed.is_err());
    assert_eq!(
        fs::read(verified.path().unwrap()).unwrap(),
        setup.content,
        "what is loaded is what was checked"
    );
    assert_eq!(verified.path().unwrap(), path);

    drop(verified);
    fs::write(&path, vec![0u8; 300]).expect("the file is free again");
}

#[test]
fn a_file_of_the_wrong_size_or_checksum_is_refused() {
    let setup = Setup::new();

    let short = install(&setup, &setup.content[..299]);
    let error = verify(&setup.spec, &short).unwrap_err();
    assert!(
        matches!(
            error,
            VerifyError::WrongSize {
                expected: 300,
                found: 299
            }
        ),
        "{error}"
    );
    assert!(error.is_corrupt());

    let same_size = install(&setup, &vec![5u8; 300]);
    let error = verify(&setup.spec, &same_size).unwrap_err();
    assert!(matches!(error, VerifyError::WrongHash), "{error}");
    assert!(error.is_corrupt());
}

#[cfg(unix)]
#[test]
fn a_link_a_folder_and_a_missing_file_are_not_the_model() {
    let setup = Setup::new();
    let real = install(&setup, &setup.content);
    let link = setup.dir.join("link.gguf");
    symlink(&real, &link).unwrap();

    assert!(matches!(verify(&setup.spec, &link), Err(VerifyError::NotRegular)));
    assert!(matches!(
        verify(&setup.spec, &setup.dir),
        Err(VerifyError::NotRegular)
    ));
    let missing = verify(&setup.spec, Path::new("/nonexistent/platter/tiny.gguf")).unwrap_err();
    assert!(matches!(missing, VerifyError::Missing));
    assert!(!missing.is_corrupt());
}

#[test]
fn the_real_client_builds_and_refuses_a_plain_http_address_without_touching_the_network() {
    let client = platter_ai::UreqFetch::new();

    let error = platter_ai::Fetch::get(&client, "http://127.0.0.1:9/model.gguf", None).unwrap_err();

    assert!(
        error.to_string().to_lowercase().contains("https"),
        "refused for being http, before any connection: {error}"
    );
}
