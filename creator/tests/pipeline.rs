//! The whole pipeline against a fake model and a fake HTTP server.
//! No GPU or real model needed, so this runs in CI on Windows and Linux.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aien_rig_creator::chapters;
use aien_rig_creator::llm::{ChatModel, ChatReply, ChatRequest};
use aien_rig_creator::pipeline::{create, Options};
use aien_rig_creator::time::{fmt_clock, parse_timestamp};
use aien_rig_creator::transcript::{parse_str, Format, Segment, Transcript};
use aien_rig_creator::{run, Job, Result};

/// A 10 minute transcript: one 6 second line every 6 seconds.
fn long_transcript() -> Transcript {
    let segs: Vec<Segment> = (0..100)
        .map(|i| Segment {
            start: i as f64 * 6.0,
            end: i as f64 * 6.0 + 6.0,
            text: format!("this is line {i} of the video and um we keep talking about the build"),
        })
        .collect();
    Transcript { format: Format::Srt, timing_estimated: false, language: None, declared_duration: None, segments: segs }
}

/// Answers like a model would, with some of the usual mess.
struct FakeModel {
    calls: RefCell<Vec<String>>,
    /// Make the first notes reply unreadable, to test the retry.
    garble_first: bool,
}

impl FakeModel {
    fn new(garble_first: bool) -> Self {
        FakeModel { calls: RefCell::new(Vec::new()), garble_first }
    }
}

fn first_time(text: &str) -> f64 {
    text.lines()
        .find_map(|l| l.strip_prefix('[').and_then(|r| r.split(']').next()).and_then(parse_timestamp))
        .unwrap_or(0.0)
}

impl ChatModel for FakeModel {
    fn model_id(&self) -> String {
        "fake".into()
    }

    fn chat(&self, req: &ChatRequest) -> Result<ChatReply> {
        let n = self.calls.borrow().len();
        self.calls.borrow_mut().push(req.user.clone());
        let text = if req.user.contains("Merge them into one set of notes") {
            r#"{"summary": "Merged notes.", "topics": [], "moments": [], "keywords": ["merged"]}"#.to_string()
        } else if req.user.contains("Transcript (each line starts with its time)") {
            if self.garble_first && n == 0 {
                "Sorry, I cannot help with that.".to_string()
            } else {
                let t = first_time(&req.user);
                format!(
                    "<think>hmm</think>```json\n{{\"summary\": \"Part starting at {c}.\", \"topics\": [{{\"start\": \"{c}\", \"title\": \"Topic at {c}\"}}], \"moments\": [{{\"start\": \"{c}\", \"end\": \"{e}\", \"hook\": \"this is line\", \"why\": \"funny\"}}], \"keywords\": [\"build\", \"video\"],}}\n```",
                    c = fmt_clock(t + 6.0),
                    e = fmt_clock(t + 40.0)
                )
            }
        } else if req.user.contains("Notes on the video, part by part") {
            // Chapters break the rules on purpose (late first, too close, past the end).
            r##"Here you go: {"titles": ["1. Big Build Day", "2. Can We Finish It?", "Big build day", "3 Hours <Building>"],
               "description": "We build a thing.\n00:00 Intro\nIt goes well.",
               "chapters": [{"start": "00:20", "title": "Start"}, {"start": "00:25", "title": "Too close"}, {"start": "03:00", "title": "Middle"}, {"start": "07:30", "title": "Late part"}, {"start": "12:00", "title": "Past the end"}],
               "tags": ["#minecraft", "build", "Build", "survival"],
               "shorts": [{"start": "01:00", "end": "01:30", "hook": "watch this", "why": "big moment"}, {"start": "01:05", "end": "01:35", "hook": "dup", "why": "overlaps"}]}"##
                .to_string()
        } else if req.user.contains("Write exactly 5 different YouTube title ideas") {
            r#"{"titles": ["Big Build Day", "Fresh Title A", "Fresh Title B", "Fresh Title C"]}"#.to_string()
        } else {
            "{}".to_string()
        };
        Ok(ChatReply { text, finish_reason: Some("stop".into()), ..Default::default() })
    }
}

#[test]
fn full_pipeline_with_fake_model() {
    let tr = long_transcript();
    let model = FakeModel::new(true);
    let opts = Options { chunk_tokens: Some(400), context_tokens: 3200, ..Options::default() };
    let mut steps = Vec::new();
    let p = create(&tr, &model, &opts, &mut |m: &str| steps.push(m.to_string())).unwrap();

    // Several parts, the first one retried after an unreadable reply.
    assert!(p.notes.len() >= 3, "parts: {}", p.notes.len());
    assert!(p.requests.iter().any(|r| r.step == "notes part 1 (retry)" && r.parsed));
    assert!(steps.iter().any(|s| s.starts_with("Reading part 2 of")));

    // Titles: numbering and brackets gone, duplicate dropped, topped up to 5.
    assert_eq!(p.titles.len(), 5, "{:?}", p.titles);
    assert_eq!(p.titles[0], "Big Build Day");
    assert_eq!(p.titles[1], "Can We Finish It?");
    assert_eq!(p.titles[2], "3 Hours Building");

    // Chapters always pass YouTube's rules.
    assert!(chapters::check(&p.chapters, p.duration_secs).is_empty(), "{:?}", p.chapters);
    assert!(p.chapter_problems.is_empty());
    assert!(p.chapters.len() >= 3);
    assert_eq!(p.chapters[0].start, 0.0);
    assert!(p.full_description.starts_with("We build a thing.\nIt goes well.\n\n00:00 "), "{}", p.full_description);

    // Tags deduped and without '#'.
    assert_eq!(p.tags, ["minecraft", "build", "survival"]);

    // Shorts: the overlapping pick is dropped and notes moments top it up to 3.
    assert!(p.shorts.len() >= 3, "{:?}", p.shorts);
    assert_eq!(p.shorts[0].start, 60.0);
    for s in &p.shorts {
        assert!(s.seconds() >= 15.0 && s.seconds() <= 60.0, "{s:?}");
        assert!(tr.segments.iter().any(|g| g.start == s.start), "start not on a line: {s:?}");
    }
    for w in p.shorts.windows(2) {
        let overlap = (w[0].end.min(w[1].end) - w[0].start.max(w[1].start)).max(0.0);
        assert!(overlap <= 0.5 * w[0].seconds().min(w[1].seconds()));
    }
}

#[test]
fn notes_that_do_not_fit_are_condensed() {
    let tr = long_transcript();
    let model = FakeModel::new(false);
    // Small parts and a tiny final budget force merge rounds.
    let opts = Options { chunk_tokens: Some(150), context_tokens: 2000 + 700 + 200, final_reply_tokens: 2000, ..Options::default() };
    let p = create(&tr, &model, &opts, &mut |_| {}).unwrap();
    assert!(p.requests.iter().any(|r| r.step.starts_with("combine notes")), "{:?}", p.requests);
    assert!(chapters::check(&p.chapters, p.duration_secs).is_empty());
}

#[test]
fn model_that_never_answers_well_still_gives_valid_chapters() {
    struct Useless;
    impl ChatModel for Useless {
        fn model_id(&self) -> String {
            "useless".into()
        }
        fn chat(&self, _: &ChatRequest) -> Result<ChatReply> {
            Ok(ChatReply { text: "I like turtles".into(), ..Default::default() })
        }
    }
    let tr = long_transcript();
    let p = create(&tr, &Useless, &Options::default(), &mut |_| {}).unwrap();
    assert!(chapters::check(&p.chapters, p.duration_secs).is_empty());
    assert!(!p.warnings.is_empty());
}

// ---------- fake HTTP server ----------

/// Serve canned HTTP responses in order; record request bodies.
fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for resp in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            seen2.lock().unwrap().push(format!("{}{}", first.trim(), String::from_utf8_lossy(&body)));
            stream.write_all(resp.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });
    (format!("http://{addr}"), seen)
}

fn http(status: &str, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
}

fn chat_body(content: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 50}
    })
    .to_string()
}

#[test]
fn http_client_finds_model_and_waits_out_busy_server() {
    let notes = r#"{"summary": "s", "topics": [{"start": "00:00", "title": "Intro"}], "moments": [], "keywords": ["k"]}"#;
    let fin = r#"{"titles": ["A", "B", "C", "D", "E"], "description": "D.", "chapters": [{"start": "00:00", "title": "One"}, {"start": "00:20", "title": "Two"}, {"start": "00:40", "title": "Three"}], "tags": ["t"], "shorts": [{"start": "00:05", "end": "00:25", "hook": "h", "why": "w"}]}"#;
    let (base, seen) = serve(vec![
        http("200 OK", "", r#"{"object":"list","data":[{"id":"rig-test-model"}]}"#),
        http("503 Service Unavailable", "Retry-After: 1\r\n", r#"{"error":"busy"}"#),
        http("200 OK", "", &chat_body(notes)),
        http("200 OK", "", &chat_body(fin)),
    ]);

    let dir = std::env::temp_dir().join(format!("rig-create-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("clip.srt");
    let mut srt = String::new();
    for i in 0..12 {
        srt.push_str(&format!(
            "{}\n00:00:{:02},000 --> 00:00:{:02},000\nline {i} um with words\n\n",
            i + 1,
            i * 5,
            i * 5 + 5
        ));
    }
    std::fs::write(&input, srt).unwrap();

    let mut job = Job::new(&input);
    job.server = base; // no /v1: added automatically
    job.request_timeout = Duration::from_secs(20);
    let out = run(&job, &mut |_| {}).unwrap();

    let p = out.package.as_ref().unwrap();
    assert_eq!(p.model, "rig-test-model");
    assert_eq!(p.requests.iter().map(|r| r.busy_retries).sum::<u32>(), 1);
    assert_eq!(p.titles.len(), 5);
    assert!(p.chapter_problems.is_empty());
    let seen = seen.lock().unwrap();
    assert!(seen[0].starts_with("GET /v1/models"));
    assert!(seen[2].starts_with("POST /v1/chat/completions"));
    assert!(seen[2].contains("\"model\":\"rig-test-model\""));

    for f in ["youtube.txt", "description.txt", "tags.txt", "result.json", "captions.clean.srt", "captions.marked.srt", "fillers.txt"] {
        assert!(out.out_dir.join(f).is_file(), "missing {f}");
    }
    let clean = std::fs::read_to_string(out.out_dir.join("captions.clean.srt")).unwrap();
    assert!(!clean.contains(" um "));
    let marked = std::fs::read_to_string(out.out_dir.join("captions.marked.srt")).unwrap();
    assert!(marked.contains("{um}"));
    let desc = std::fs::read_to_string(out.out_dir.join("description.txt")).unwrap();
    assert!(desc.contains("00:00 One\n00:20 Two\n00:40 Three"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreachable_server_gives_a_plain_error() {
    // Bind then drop to get a port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!("rig-create-noserver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("a.srt");
    std::fs::write(&input, "1\n00:00:00,000 --> 00:00:05,000\nhello\n").unwrap();
    let mut job = Job::new(&input);
    job.server = format!("http://127.0.0.1:{port}/v1");
    let err = run(&job, &mut |_| {}).unwrap_err().to_string();
    assert!(err.contains("cannot reach the model server"), "{err}");

    // Captions-only needs no server.
    job.captions_only = true;
    let out = run(&job, &mut |_| {}).unwrap();
    assert!(out.package.is_none());
    assert!(out.out_dir.join("captions.clean.srt").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sample_files_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let srt = std::fs::read_to_string(root.join("castle-build.srt")).unwrap();
    let t = parse_str(&srt, Format::Srt).unwrap();
    assert!(t.segments.len() > 50);
    assert!(t.duration() > 300.0);
}
