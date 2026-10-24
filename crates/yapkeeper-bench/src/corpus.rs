//! The synthetic corpus: Claude Code, Codex and Pi histories, and the
//! project folders their sessions ran in, written from a seed so the same
//! seed always gives the same bytes. Each provider's file layout, the mix
//! of locations and the shape of a conversation stay in here.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yapkeeper::Roots;

/// What to generate. Every value is independent of the machine, so two runs
/// with the same shape measure the same corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub seed: u64,
    /// Sessions across every provider.
    pub sessions: usize,
    /// Claude Code subagent transcripts, each a file of its own beside a
    /// session's.
    pub subagents: usize,
    /// Roughly how many bytes of history to write; a file is never cut
    /// short of its first exchange, so the corpus may run a little over.
    pub bytes: u64,
}

impl Default for Shape {
    /// Sized like the histories seen in planning: about 800 sessions in
    /// about 870 files, about 1 GB.
    fn default() -> Shape {
        Shape {
            seed: 1,
            sessions: 800,
            subagents: 70,
            bytes: 1_000_000_000,
        }
    }
}

/// What `generate` wrote.
pub struct Corpus {
    /// The provider history directories, inside the corpus.
    pub roots: Roots,
    pub claude: usize,
    pub codex: usize,
    pub pi: usize,
    /// Sessions whose folder is gone, outside the temp folders.
    pub orphaned: usize,
    /// Sessions that ran in a system temp folder, since cleared away.
    pub temp: usize,
    /// History files, subagent transcripts included.
    pub files: usize,
    /// History bytes.
    pub bytes: u64,
}

impl Corpus {
    /// Sessions across every provider.
    pub fn sessions(&self) -> usize {
        self.claude + self.codex + self.pi
    }

    /// Claude Code subagent transcripts written, which may be fewer than
    /// asked for when there was no Claude Code session to attach them to.
    pub fn subagents(&self) -> usize {
        self.files - self.sessions()
    }
}

/// The projects live sessions ran in.
const PROJECTS: [&str; 6] = ["atlas", "harbor", "lumen", "orbit", "quartz", "tundra"];

/// What a session is about: its messages say this word now and then, so a
/// query for one finds some sessions, not all.
const TOPICS: [&str; 8] = [
    "migration",
    "retry",
    "db",
    "billing",
    "deploy",
    "parser",
    "cache",
    "timeout",
];

/// The queries the benchmark times, each matching sessions in this corpus
/// through its topics, projects, providers and orphans: a word, a short
/// word, filters only, and a location filter with a word.
pub(crate) const QUERIES: [&str; 4] = ["migration", "db", "#codex is:orphaned", "@atlas retry"];

/// What every other word is made of, two or three to a word: a wide
/// vocabulary in which no topic or project name can appear by chance,
/// since each pairs one consonant with one vowel.
const SYLLABLES: [&str; 16] = [
    "ka", "lo", "mi", "ne", "ru", "sa", "to", "vi", "ze", "po", "gu", "fe", "ho", "ja", "wi", "ye",
];

/// Writes the corpus `shape` describes into `dir`, creating it and the
/// folders live sessions ran in. `dir` must not sit under a system temp
/// folder, or every live session there would count as temp. Fails when a
/// file can't be written.
pub fn generate(shape: &Shape, dir: &Path) -> io::Result<Corpus> {
    let mut random = Random::new(shape.seed);
    let roots = Roots {
        claude: dir.join("claude"),
        codex: dir.join("codex"),
        pi: dir.join("pi"),
    };
    let work = dir.join("work");
    for project in PROJECTS {
        fs::create_dir_all(work.join(project).join(".git"))?;
    }
    let mut corpus = Corpus {
        roots,
        claude: 0,
        codex: 0,
        pi: 0,
        orphaned: 0,
        temp: 0,
        files: 0,
        bytes: 0,
    };

    // Sizes are drawn from a long tail, as real histories have a few huge
    // sessions and many small ones, then scaled to the total asked for.
    let weights: Vec<f64> = (0..shape.sessions + shape.subagents)
        .map(|_| 1.0 / (0.02 + random.unit()))
        .collect();
    let total: f64 = weights.iter().sum();
    let budgets: Vec<u64> = weights
        .iter()
        .map(|weight| (shape.bytes as f64 * weight / total) as u64)
        .collect();
    let (session_budgets, subagent_budgets) = budgets.split_at(shape.sessions);

    let mut claude_sessions = Vec::new();
    for (n, budget) in session_budgets.iter().enumerate() {
        let id = random.uuid();
        let cwd = match random.below(100) {
            0..8 => {
                corpus.orphaned += 1;
                format!("/synthetic/gone/{}-{n}", random.pick(&PROJECTS))
            }
            8..14 => {
                corpus.temp += 1;
                format!("/tmp/yapkeeper-bench-{n}")
            }
            14..70 => work.join(random.pick(&PROJECTS)).display().to_string(),
            70..90 => worktree(&work, random.pick(&PROJECTS), n)?,
            _ => {
                let folder = work.join("notes").join(format!("folder-{n}"));
                fs::create_dir_all(&folder)?;
                folder.display().to_string()
            }
        };
        // Spread over the year before the planning date, a minute apart at least.
        let started = BASE_SECOND - random.below(365 * 24 * 60) as i64 * 60;
        let session = SessionPlan {
            id,
            cwd,
            topic: random.pick(&TOPICS),
            started,
            budget: *budget,
        };
        let path = match random.below(100) {
            0..60 => {
                corpus.claude += 1;
                let path = claude_session(&corpus.roots.claude, &session, &mut random)?;
                claude_sessions.push(session);
                path
            }
            60..85 => {
                corpus.codex += 1;
                codex_session(&corpus.roots.codex, &session, &mut random)?
            }
            _ => {
                corpus.pi += 1;
                pi_session(&corpus.roots.pi, &session, &mut random)?
            }
        };
        corpus.files += 1;
        corpus.bytes += fs::metadata(path)?.len();
    }

    for (n, budget) in subagent_budgets.iter().enumerate() {
        // With no Claude Code session to attach to, a subagent would be
        // left out of the index, so none is written.
        if claude_sessions.is_empty() {
            break;
        }
        let parent = &claude_sessions[random.below(claude_sessions.len() as u64) as usize];
        let path = claude_dir(&corpus.roots.claude, &parent.cwd)
            .join(&parent.id)
            .join("subagents")
            .join(format!("agent-{n:04}.jsonl"));
        let subagent = SessionPlan {
            budget: *budget,
            ..parent.clone()
        };
        // Every record in a subagent transcript is a sidechain.
        write_conversation(&path, &[], &subagent, &mut random, |turn| {
            claude_records(turn, true)
        })?;
        corpus.files += 1;
        corpus.bytes += fs::metadata(path)?.len();
    }
    Ok(corpus)
}

/// 2026-10-23T00:00:00Z, the day the session formats were inspected.
const BASE_SECOND: i64 = 1_792_713_600;

/// One session to write.
#[derive(Clone)]
struct SessionPlan {
    id: String,
    cwd: String,
    /// One of `TOPICS`.
    topic: &'static str,
    /// Seconds since the Unix epoch.
    started: i64,
    /// Roughly how many bytes its file should hold.
    budget: u64,
}

/// A linked worktree of `project`, laid out as `<project>.wt/<branch>`
/// with a `.git` file, as `git worktree add` leaves one; returns its path.
fn worktree(work: &Path, project: &str, n: usize) -> io::Result<String> {
    let branch = format!("branch-{n}");
    let checkout = work.join(format!("{project}.wt")).join(&branch);
    fs::create_dir_all(&checkout)?;
    let gitdir = work.join(project).join(".git/worktrees").join(&branch);
    fs::write(
        checkout.join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )?;
    Ok(checkout.display().to_string())
}

/// Claude Code's folder for sessions run in `cwd`: the path with every
/// `/` and `.` made a `-`.
fn claude_dir(root: &Path, cwd: &str) -> PathBuf {
    root.join("projects").join(cwd.replace(['/', '.'], "-"))
}

fn claude_session(root: &Path, session: &SessionPlan, random: &mut Random) -> io::Result<PathBuf> {
    let path = claude_dir(root, &session.cwd).join(format!("{}.jsonl", session.id));
    write_conversation(&path, &[], session, random, |turn| {
        claude_records(turn, false)
    })?;
    Ok(path)
}

fn codex_session(root: &Path, session: &SessionPlan, random: &mut Random) -> io::Result<PathBuf> {
    let started = timestamp(session.started);
    let day = &started[..10];
    let path = root
        .join("sessions")
        .join(day.replace('-', "/"))
        .join(format!(
            "rollout-{}-{}.jsonl",
            started[..19].replace(':', "-"),
            session.id
        ));
    let meta = json!({
        "timestamp": started,
        "type": "session_meta",
        "payload": { "id": session.id, "cwd": session.cwd, "source": "cli" },
    });
    write_conversation(&path, &[meta], session, random, codex_turn)?;
    Ok(path)
}

fn pi_session(root: &Path, session: &SessionPlan, random: &mut Random) -> io::Result<PathBuf> {
    let started = timestamp(session.started);
    let path = root
        .join(format!("-{}-", session.cwd.replace('/', "-")))
        .join(format!(
            "{}_{}.jsonl",
            started.replace(':', "-"),
            session.id
        ));
    let header = json!({
        "type": "session",
        "version": 3,
        "id": session.id,
        "timestamp": started,
        "cwd": session.cwd,
    });
    write_conversation(&path, &[header], session, random, pi_turn)?;
    Ok(path)
}

/// One exchange as a provider records it: a prompt, a reply, a tool call
/// and its output, at `second`.
struct Turn<'a> {
    session: &'a SessionPlan,
    second: i64,
    /// Which exchange this is in the file, from 0.
    n: usize,
    prompt: String,
    reply: String,
    command: String,
    output: String,
}

/// Writes `header` then whole exchanges to `path` until it holds at least
/// `session.budget` bytes, and always one exchange.
fn write_conversation(
    path: &Path,
    header: &[Value],
    session: &SessionPlan,
    random: &mut Random,
    records: fn(&Turn) -> Vec<Value>,
) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("a history file sits in a folder"))?;
    let mut out = BufWriter::new(fs::File::create(path)?);
    let mut written = 0;
    for record in header {
        written += write_record(&mut out, record)?;
    }
    let mut n = 0;
    while n == 0 || written < session.budget {
        let turn = Turn {
            session,
            second: session.started + n as i64 * 30,
            n,
            prompt: random.sentence(8, 20, session.topic),
            reply: random.sentence(20, 60, session.topic),
            command: format!("rg {} src", session.topic),
            output: random.output(session.topic),
        };
        for record in records(&turn) {
            written += write_record(&mut out, &record)?;
        }
        n += 1;
    }
    out.flush()
}

/// Writes `record` as one JSONL line and returns how many bytes it took.
fn write_record(out: &mut impl Write, record: &Value) -> io::Result<u64> {
    let line = format!("{record}\n");
    out.write_all(line.as_bytes())?;
    Ok(line.len() as u64)
}

fn claude_records(turn: &Turn, sidechain: bool) -> Vec<Value> {
    let time = timestamp(turn.second);
    let tool = format!("toolu_{:06}", turn.n);
    let record = |kind: &str, content: Value| {
        json!({
            "type": kind,
            "sessionId": turn.session.id,
            "cwd": turn.session.cwd,
            "isSidechain": sidechain,
            "timestamp": time,
            "message": { "role": kind, "content": content },
        })
    };
    vec![
        record("user", json!(turn.prompt)),
        record(
            "assistant",
            json!([
                { "type": "text", "text": turn.reply },
                { "type": "tool_use", "id": tool, "name": "Bash",
                  "input": { "command": turn.command } },
            ]),
        ),
        record(
            "user",
            json!([{ "type": "tool_result", "tool_use_id": tool, "content": turn.output }]),
        ),
    ]
}

fn codex_turn(turn: &Turn) -> Vec<Value> {
    let time = timestamp(turn.second);
    let call = format!("call_{:06}", turn.n);
    let item =
        |payload: Value| json!({ "timestamp": time, "type": "response_item", "payload": payload });
    vec![
        item(json!({ "type": "message", "role": "user",
                     "content": [{ "type": "input_text", "text": turn.prompt }] })),
        item(json!({ "type": "message", "role": "assistant",
                     "content": [{ "type": "output_text", "text": turn.reply }] })),
        item(
            json!({ "type": "function_call", "name": "shell", "call_id": call,
                     "arguments": json!({ "command": turn.command }).to_string() }),
        ),
        item(json!({ "type": "function_call_output", "call_id": call, "output": turn.output })),
    ]
}

fn pi_turn(turn: &Turn) -> Vec<Value> {
    let time = timestamp(turn.second);
    let message = |n: usize, message: Value| {
        json!({
            "type": "message",
            "id": format!("{:06}-{n}", turn.n),
            "timestamp": time,
            "message": message,
        })
    };
    vec![
        message(
            0,
            json!({ "role": "user", "content": [{ "type": "text", "text": turn.prompt }] }),
        ),
        message(
            1,
            json!({ "role": "assistant", "content": [
                { "type": "text", "text": turn.reply },
                { "type": "toolCall", "name": "bash", "arguments": { "command": turn.command } },
            ] }),
        ),
        message(
            2,
            json!({ "role": "toolResult", "content": [{ "type": "text", "text": turn.output }] }),
        ),
    ]
}

/// `second` since the Unix epoch as RFC 3339 in UTC, as every provider records it.
fn timestamp(second: i64) -> String {
    jiff::Timestamp::from_second(second)
        .expect("every corpus time is near 2026")
        .to_string()
}

/// SplitMix64: a small, fast generator whose sequence depends only on its
/// seed, on every machine, so the corpus does too.
struct Random {
    state: u64,
}

impl Random {
    fn new(seed: u64) -> Random {
        Random { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In `0..bound`; `bound` is never 0.
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// In `0.0..1.0`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }

    fn uuid(&mut self) -> String {
        let (a, b) = (self.next(), self.next());
        format!(
            "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
            a >> 32,
            (a >> 16) & 0xffff,
            a & 0xfff,
            b >> 52,
            b & 0xffff_ffff_ffff
        )
    }

    /// Between `min` and `max` words and a full stop; about one word in
    /// sixteen is `topic`.
    fn sentence(&mut self, min: u64, max: u64, topic: &str) -> String {
        let count = min + self.below(max - min + 1);
        let mut sentence = String::new();
        for n in 0..count {
            if n > 0 {
                sentence.push(' ');
            }
            match self.below(16) {
                0 => sentence.push_str(topic),
                _ => self.push_word(&mut sentence),
            }
        }
        sentence.push('.');
        sentence
    }

    /// Pushes a word of two or three of `SYLLABLES` onto `text`.
    fn push_word(&mut self, text: &mut String) {
        for _ in 0..2 + self.below(2) {
            text.push_str(self.pick(&SYLLABLES));
        }
    }

    /// Tool output like a search or test run prints: 20 to 120 lines, where
    /// most of a real history's bytes are.
    fn output(&mut self, topic: &str) -> String {
        let mut output = String::new();
        for line in 0..20 + self.below(101) {
            if line > 0 {
                output.push('\n');
            }
            output.push_str("src/");
            self.push_word(&mut output);
            output.push('/');
            self.push_word(&mut output);
            let sentence = self.sentence(4, 12, topic);
            output.push_str(&format!(".rs:{}: {sentence}", line + 1));
        }
        output
    }
}
