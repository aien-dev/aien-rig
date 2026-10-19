//! AIEN Rig YouTube helper.
//!
//! Give it a video's transcript or captions; it writes five title ideas, a
//! description with YouTube chapters, tags, three to five Shorts picks with
//! timestamps, and captions with filler words marked and removed. All model
//! work happens on a local OpenAI-style server (the Rig engine by default).
//!
//! The quickest way in is [`run`]; the pieces are public for the app to use
//! one by one.

pub mod chapters;
pub mod chunk;
pub mod error;
pub mod filler;
pub mod jsonx;
pub mod llm;
pub mod output;
pub mod pipeline;
pub mod prompts;
pub mod shorts;
pub mod time;
pub mod transcript;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use error::{Error, Result};
pub use filler::{Captions, FillerReport};
pub use llm::{ChatModel, HttpModel, DEFAULT_SERVER};
pub use pipeline::{Options, Package};
pub use transcript::{Format, Transcript};

/// One run of the helper.
#[derive(Debug, Clone)]
pub struct Job {
    pub input: PathBuf,
    /// Force a format instead of detecting it.
    pub format: Option<Format>,
    /// Where to write; defaults to `<input name>-rig` next to the input.
    pub out_dir: Option<PathBuf>,
    pub server: String,
    /// Model id; `None` uses the first model the server lists.
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub options: Options,
    /// Only clean the captions; no model needed.
    pub captions_only: bool,
    /// Longest wait for one model reply.
    pub request_timeout: Duration,
}

impl Job {
    pub fn new(input: impl Into<PathBuf>) -> Job {
        Job {
            input: input.into(),
            format: None,
            out_dir: None,
            server: DEFAULT_SERVER.to_string(),
            model: None,
            api_key: None,
            options: Options::default(),
            captions_only: false,
            request_timeout: Duration::from_secs(900),
        }
    }
}

/// What a run produced.
#[derive(Debug)]
pub struct Outcome {
    pub out_dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub transcript: Transcript,
    pub captions: Captions,
    /// `None` for captions-only runs.
    pub package: Option<Package>,
}

/// `<folder>/<name>-rig` for input `<folder>/<name>.<ext>`.
pub fn default_out_dir(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}-rig"))
}

/// Read the transcript, clean the captions, ask the model for everything else,
/// and write the files. `progress` gets one short line per step.
pub fn run(job: &Job, progress: &mut dyn FnMut(&str)) -> Result<Outcome> {
    progress(&format!("Reading {}", job.input.display()));
    let tr = transcript::parse_file(&job.input, job.format)?;
    let duration = tr.duration();
    let captions = filler::process(&tr.segments, duration);
    let package = if job.captions_only {
        None
    } else {
        progress(&format!("Connecting to the model server at {}", llm::normalize_base(&job.server)));
        let model = HttpModel::connect(&job.server, job.model.as_deref(), job.api_key.as_deref(), job.request_timeout)?;
        Some(pipeline::create(&tr, &model, &job.options, progress)?)
    };
    let out_dir = job.out_dir.clone().unwrap_or_else(|| default_out_dir(&job.input));
    let source_name = job.input.file_name().and_then(|n| n.to_str()).unwrap_or("transcript");
    let files = output::write_all(&out_dir, source_name, &tr, &captions, package.as_ref())?;
    Ok(Outcome { out_dir, files, transcript: tr, captions, package })
}
