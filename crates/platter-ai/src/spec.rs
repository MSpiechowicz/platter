//! The model Platter uses: one pinned file. Everything about it is fixed in the code: the web view can
//! name neither another file nor another address.

use std::time::Duration;

/// A model file: what it is, where it comes from and what it must look like on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    /// Name shown to the user.
    pub name: &'static str,
    /// Format shown to the user.
    pub format: &'static str,
    /// File name inside the models folder.
    pub file: &'static str,
    /// Exact size in bytes.
    pub bytes: u64,
    /// SHA-256 of the file, 64 lower-case hexadecimal digits.
    pub sha256: &'static str,
    /// Where the file is downloaded from (`https` only).
    pub url: &'static str,
    /// The model's licence.
    pub licence: &'static str,
    /// A load is refused when less memory than this is available: the process's resident memory after the
    /// largest answer (measured, A5.0) plus 512 MiB.
    pub min_free_bytes: u64,
    /// The same for a model loaded on a graphics card: the process's resident memory after the largest
    /// answer there plus 512 MiB (G1).
    pub gpu_min_free_bytes: u64,
    /// A graphics card is used only with at least this much free video memory: what the model took there
    /// after the largest answer plus a margin (G1).
    pub gpu_video_bytes: u64,
    /// Tokens the context can hold at most: the prompt and the answer.
    pub context_tokens: u32,
    /// Time the labelling job gives this model for one folder before it skips it.
    pub folder_limit: Duration,
    /// Time the whole labelling job may take.
    pub job_limit: Duration,
    /// Time the model gets to read a free question into a query before the general question is asked
    /// instead.
    pub interpret_limit: Duration,
}

impl ModelSpec {
    /// The name of the file while it is being downloaded.
    pub fn partial_file(&self) -> String {
        format!("{}.partial", self.file)
    }
}

const MIB: u64 = 1024 * 1024;
/// Resident memory with the model on a graphics card after the largest answer (3981 tokens), 806 MiB,
/// plus 512 MiB, rounded up (G1, Radeon RX 7900 XTX, Vulkan).
const GPU_MIN_FREE_MIB: u64 = 1344;
/// Video memory the model took after the largest answer, 3303 MiB, plus 1 GiB for the driver and other
/// programs, rounded up (G1).
const GPU_VIDEO_NEED_MIB: u64 = 4352;

/// Qwen3.5-4B, Q4_K_M quantisation, from a pinned revision.
pub const MODEL: ModelSpec = ModelSpec {
    name: "Qwen3.5-4B",
    format: "GGUF · Q4_K_M",
    file: "Qwen3.5-4B-Q4_K_M.gguf",
    bytes: 2_740_937_888,
    sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
    url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/e87f176479d0855a907a41277aca2f8ee7a09523/Qwen3.5-4B-Q4_K_M.gguf",
    licence: "Apache-2.0",
    // Resident memory after the largest answer (4096-token context), 4315 MiB, plus 512 MiB (A5.0).
    min_free_bytes: 4827 * MIB,
    gpu_min_free_bytes: GPU_MIN_FREE_MIB * MIB,
    gpu_video_bytes: GPU_VIDEO_NEED_MIB * MIB,
    context_tokens: 4096,
    // About 6.6 s a folder on 8 threads (A5.0); 15 s would leave little margin on a busy machine.
    folder_limit: Duration::from_secs(25),
    // 50 folders at about 6.6 s take 5.5 minutes.
    job_limit: Duration::from_secs(10 * 60),
    // Reading a question took 8.2 s at most on 8 threads (A5.0).
    interpret_limit: Duration::from_secs(20),
};

/// Files of a model Platter no longer uses, in the models folder. Qwen3.5-2B was dropped: on the A5.0
/// questions and a run on a real disk it misread 6 of 18 folders against the 4B's 2 of 18. They are
/// removed at start (see [`crate::retire::remove_retired`]), by exact name.
pub const RETIRED_FILES: [&str; 2] = ["Qwen3.5-2B-Q4_K_M.gguf", "Qwen3.5-2B-Q4_K_M.gguf.partial"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_model_is_an_https_file_with_a_full_sha256() {
        let spec = MODEL;

        assert_eq!(spec.name, "Qwen3.5-4B");
        assert!(spec.url.starts_with("https://"));
        assert_eq!(spec.sha256.len(), 64);
        assert!(
            spec.sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        );
        assert!(!spec.file.contains('/'), "a bare file name, never a path");
        assert!(
            spec.min_free_bytes > spec.bytes + 512 * MIB,
            "room for the model, its context and 512 MiB"
        );
        assert!(
            spec.gpu_min_free_bytes < spec.min_free_bytes,
            "the card holds the weights"
        );
        assert!(spec.gpu_video_bytes > spec.bytes, "the weights fit on the card");
        assert_eq!(spec.context_tokens, 4096);
        assert!(!spec.folder_limit.is_zero());
        assert!(spec.job_limit >= spec.folder_limit);
        assert!(!spec.interpret_limit.is_zero());
    }

    #[test]
    fn the_model_gets_its_measured_times() {
        assert_eq!(MODEL.folder_limit, Duration::from_secs(25));
        assert_eq!(MODEL.job_limit, Duration::from_secs(600));
        assert_eq!(MODEL.interpret_limit, Duration::from_secs(20));
    }

    #[test]
    fn the_partial_file_sits_next_to_the_model() {
        assert_eq!(MODEL.partial_file(), "Qwen3.5-4B-Q4_K_M.gguf.partial");
    }

    #[test]
    fn the_retired_files_are_bare_names_and_not_the_model() {
        for file in RETIRED_FILES {
            assert!(!file.contains('/'), "{file}");
            assert_ne!(file, MODEL.file);
            assert_ne!(file, MODEL.partial_file());
        }
    }
}
