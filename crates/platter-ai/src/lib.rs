//! The advisor model: its download, its verification, the runtime that runs it and the worker thread
//! that owns them.
//!
//! The crate has two traits of its own, [`Fetch`] (HTTPS) and [`Engine`] (the model), so that tests
//! need neither the network nor a model. The real model runtime is behind the feature `llama`; without
//! it the crate still downloads and verifies, and an app reports the advisor as unavailable.
//!
//! The model runs in a helper process (Unix): [`process::ProcessEngine`] is the [`Engine`] the app uses,
//! [`runtime::serve`] is the helper's side, and [`protocol`] is what they say to each other. A graphics driver
//! that crashes or hangs then ends the helper, not the app, and the worker finishes the job on the processor
//! and tells [`WorkerConfig::on_gpu_incident`].
//!
//! The crate has no OS-specific code. Folders, descriptors, starting the helper and memory figures come from
//! `platter-platform`. Nothing here writes to the disk except the model's own file in the models folder
//! (and the removal of the retired model's two files from it).

pub mod answer;
pub mod device;
pub mod download;
pub mod engine;
pub mod facts;
pub mod fetch;
pub mod grammar;
pub mod interpret;
pub mod label;
#[cfg(feature = "llama")]
pub mod llama;
pub mod lock;
pub mod parse;
#[cfg(unix)]
pub mod process;
pub mod prompt;
pub mod protocol;
pub mod question;
pub mod reading;
pub mod retire;
#[cfg(unix)]
pub mod runtime;
pub mod spec;
#[cfg(feature = "testing")]
pub mod testing;
pub mod verify;
pub mod worker;

pub use answer::{
    By, FactRow, FactSheet, FactTotal, FileFact, OFFTOPIC_TEXT, OUTDATED_NOTE, Place, Segment, TotalKind,
    Units, format_size, render, template, template_segments,
};
pub use download::{DownloadError, Helpers, SPACE_HEADROOM, download};
pub use engine::{
    AiError, Engine, Exit, Generation, GpuDevice, IncidentKind, LoadInfo, Placement, ProgressHook,
    RuntimeIncident,
};
pub use facts::{AgeShares, FolderFacts, RuleContext, facts};
pub use fetch::{Fetch, FetchResponse, UreqFetch, unreadable_proxy_variable};
pub use interpret::{INTERPRET_SHOTS, INTERPRET_SYSTEM, interpret_prompt, parse_question};
pub use label::{Budget, LabelRun, StageLimit, Stopped, label};
pub use lock::ModelLock;
pub use parse::{Judgement, ParseError, parse};
#[cfg(unix)]
pub use process::{Limits, ProcessEngine, RealSpawner, RuntimeProcess, Spawned, Spawner, Waited};
pub use prompt::prompt;
pub use question::{Intent, Kind, MAX_QUESTION_CHARS, QuestionError, ScanQuestion, clean_question, ground};
pub use reading::{MediaKind, Signals, WellKnown, fold, signals, words_of_question};
pub use retire::remove_retired;
pub use spec::{MODEL, ModelSpec, RETIRED_FILES};
pub use verify::{VerifiedModel, VerifyError, verify};
pub use worker::{
    AdvisorHandle, AdvisorWorker, Device, GPU_FALLBACK, GpuIncident, Host, IdleTimes, IncidentFn, Labelling,
    ModelState, Options, Progress, STOPPED, StateTicket, Status, StatusFn, UnloadAfter, WorkerConfig,
    WorkerError, default_threads, threads_for,
};
