//! `rig-create`: the YouTube helper on the command line.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use aien_rig_creator::{run, Format, Job, DEFAULT_SERVER};

const HELP: &str = "rig-create: titles, description, chapters, tags, Shorts picks and clean captions from a video transcript

USAGE
  rig-create <transcript file> [options]

  The file can be .srt, .vtt (including YouTube auto-captions), YouTube .json3,
  the Rig speech engine's .json, or text copied from YouTube's \"Show transcript\".
  Results go into a folder next to the file, named <file>-rig. Open youtube.txt.

OPTIONS
  --out <folder>          Where to write the results
  --about \"<text>\"        One line about the channel, e.g. \"Minecraft survival builds\"
  --captions-only         Only clean the captions (no model needed)
  --server <url>          Model server (default: $RIG_SERVER or http://127.0.0.1:18120/v1)
  --model <id>            Model id (default: $RIG_MODEL or the server's first model)
  --api-key <key>         Bearer key if the server needs one (default: $RIG_API_KEY)
  --context <tokens>      The model's context size (default 8192)
  --chunk-tokens <n>      Transcript tokens per part (default: worked out from --context)
  --temperature <t>       0 = same answer every time (default 0.3)
  --short-min <seconds>   Shortest Shorts clip (default 15)
  --short-max <seconds>   Longest Shorts clip (default 60)
  --format <name>         Force the input format: srt, vtt, json3, json, timed-text, text
  --timeout <seconds>     Longest wait for one model answer (default 900)
  -h, --help              Show this help
  -V, --version           Show the version
";

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("rig-create: {msg}");
            ExitCode::from(code)
        }
    }
}

fn real_main() -> Result<(), (u8, String)> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = |m: String| (2u8, format!("{m}\nRun rig-create --help for usage."));
    let mut input: Option<PathBuf> = None;
    let mut job = Job::new("");
    job.server = env_or("RIG_SERVER", DEFAULT_SERVER);
    job.model = std::env::var("RIG_MODEL").ok().filter(|s| !s.is_empty());
    job.api_key = std::env::var("RIG_API_KEY").ok().filter(|s| !s.is_empty());

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut value = || -> Result<String, (u8, String)> {
            i += 1;
            args.get(i).cloned().ok_or_else(|| usage(format!("{a} needs a value")))
        };
        match a {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("rig-create {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--out" | "-o" => job.out_dir = Some(PathBuf::from(value()?)),
            "--about" => job.options.about = Some(value()?),
            "--captions-only" => job.captions_only = true,
            "--server" => job.server = value()?,
            "--model" => job.model = Some(value()?),
            "--api-key" => job.api_key = Some(value()?),
            "--context" => job.options.context_tokens = num(&value()?, a)?,
            "--chunk-tokens" => job.options.chunk_tokens = Some(num(&value()?, a)?),
            "--temperature" => job.options.temperature = num(&value()?, a)?,
            "--short-min" => job.options.clips.min_secs = num(&value()?, a)?,
            "--short-max" => job.options.clips.max_secs = num(&value()?, a)?,
            "--timeout" => job.request_timeout = Duration::from_secs(num(&value()?, a)?),
            "--format" => {
                let v = value()?;
                job.format = Some(Format::from_name(&v).ok_or_else(|| usage(format!("unknown format {v}")))?);
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(usage(format!("unknown option {s}"))),
            s => {
                if input.is_some() {
                    return Err(usage("give one transcript file at a time".into()));
                }
                input = Some(PathBuf::from(s));
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        print!("{HELP}");
        return Err((2, "no transcript file given".into()));
    };
    if job.options.clips.min_secs <= 0.0 || job.options.clips.max_secs < job.options.clips.min_secs {
        return Err(usage("--short-min must be above 0 and at most --short-max".into()));
    }
    job.input = input;

    let mut progress = |m: &str| eprintln!("  {m}");
    let out = run(&job, &mut progress).map_err(|e| (1u8, e.to_string()))?;

    println!();
    if let Some(p) = &out.package {
        println!("Title ideas:");
        for (i, t) in p.titles.iter().enumerate() {
            println!("  {}. {t}", i + 1);
        }
        println!("Chapters: {}   Tags: {}   Shorts picks: {}", p.chapters.len(), p.tags.len(), p.shorts.len());
        for w in &p.warnings {
            println!("Please check: {w}");
        }
    }
    println!(
        "Filler words found: {} ({} per minute)",
        out.captions.report.total, out.captions.report.per_minute
    );
    println!("Results are in {}", out.out_dir.display());
    if out.package.is_some() {
        println!("Open youtube.txt there to copy everything into YouTube.");
    }
    Ok(())
}

fn env_or(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| d.to_string())
}

fn num<T: std::str::FromStr>(s: &str, flag: &str) -> Result<T, (u8, String)> {
    s.trim()
        .parse()
        .map_err(|_| (2u8, format!("{flag} needs a number, got {s}")))
}
