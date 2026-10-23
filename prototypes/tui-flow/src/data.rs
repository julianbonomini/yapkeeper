//! Synthetic sessions. Every project, path, title, and message here is invented.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    Resumable,
    /// The recorded directory is gone.
    Orphaned,
    /// The directory could not be inspected (unmounted volume, permissions).
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub role: Role,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub provider: Provider,
    /// Provider title, if any. Untitled sessions fall back to the opening prompt.
    pub title: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    /// Worktree root or plain folder.
    pub root: String,
    /// Subdirectory under `root` where the session ran, or empty.
    pub subdir: String,
    pub minutes_ago: u32,
    pub availability: Availability,
    pub hidden: bool,
    pub subagents: Vec<Vec<Msg>>,
    pub msgs: Vec<Msg>,
    /// Provider-written recap (Claude Code `away_summary`), when present.
    pub recap: Option<String>,
    /// Pull requests linked from the session.
    pub prs: Vec<u64>,
}

impl Session {
    pub fn dir(&self) -> String {
        format!("{}{}", self.root, self.subdir)
    }

    pub fn display_title(&self) -> String {
        match &self.title {
            Some(t) => t.clone(),
            None => self
                .msgs
                .iter()
                .find(|m| m.role == Role::User)
                .map(|m| m.text.lines().next().unwrap_or("").to_string())
                .unwrap_or_else(|| "(empty session)".into()),
        }
    }

    /// Display label: `project`, `project › worktree`, or the folder path.
    pub fn location(&self) -> String {
        match (&self.project, &self.branch) {
            (Some(p), Some(b)) => format!("{p} › {b}"),
            (Some(p), None) => p.clone(),
            _ => self.dir(),
        }
    }

    /// The `@` token form: `project`, `project:worktree`, or the folder path.
    pub fn loc_key(&self) -> String {
        match (&self.project, &self.branch) {
            (Some(p), Some(b)) => format!("{p}:{b}"),
            (Some(p), None) => p.clone(),
            _ => self.root.clone(),
        }
    }

    /// Scratch locations under the system temp directories.
    pub fn is_temp(&self) -> bool {
        [
            "/private/tmp",
            "/tmp/",
            "/var/folders",
            "/private/var/folders",
        ]
        .iter()
        .any(|t| self.root.starts_with(t))
    }

    /// Truncate the location label, keeping whichever end identifies it.
    pub fn location_fit(&self, w: usize) -> String {
        if self.project.is_some() {
            crate::text::trunc(&self.location(), w)
        } else {
            crate::text::trunc_left(&self.location(), w)
        }
    }

    pub fn resume_argv(&self, dir: &str) -> Vec<String> {
        match self.provider {
            Provider::Claude => vec!["claude".into(), "--resume".into(), self.id.clone()],
            Provider::Codex => vec![
                "codex".into(),
                "resume".into(),
                self.id.clone(),
                "-C".into(),
                dir.into(),
            ],
        }
    }
}

struct B(Session);

impl B {
    fn new(provider: Provider, id: &str, minutes_ago: u32) -> Self {
        B(Session {
            id: id.into(),
            provider,
            title: None,
            project: None,
            branch: None,
            root: String::new(),
            subdir: String::new(),
            minutes_ago,
            availability: Availability::Resumable,
            hidden: false,
            subagents: vec![],
            msgs: vec![],
            recap: None,
            prs: vec![],
        })
    }
    fn title(mut self, t: &str) -> Self {
        self.0.title = Some(t.into());
        self
    }
    fn wt(mut self, project: &str, branch: &str, root: &str) -> Self {
        self.0.project = Some(project.into());
        self.0.branch = Some(branch.into());
        self.0.root = root.into();
        self
    }
    fn folder(mut self, root: &str) -> Self {
        self.0.root = root.into();
        self
    }
    fn sub(mut self, subdir: &str) -> Self {
        self.0.subdir = subdir.into();
        self
    }
    fn orphan(mut self) -> Self {
        self.0.availability = Availability::Orphaned;
        self
    }
    fn unknown(mut self) -> Self {
        self.0.availability = Availability::Unknown;
        self
    }
    fn hidden(mut self) -> Self {
        self.0.hidden = true;
        self
    }
    fn u(mut self, t: &str) -> Self {
        self.0.msgs.push(Msg {
            role: Role::User,
            text: t.into(),
        });
        self
    }
    fn a(mut self, t: &str) -> Self {
        self.0.msgs.push(Msg {
            role: Role::Assistant,
            text: t.into(),
        });
        self
    }
    fn t(mut self, t: &str) -> Self {
        self.0.msgs.push(Msg {
            role: Role::Tool,
            text: t.into(),
        });
        self
    }
    fn agent(mut self, task: &str, result: &str) -> Self {
        self.0.subagents.push(vec![
            Msg {
                role: Role::User,
                text: task.into(),
            },
            Msg {
                role: Role::Assistant,
                text: result.into(),
            },
        ]);
        self
    }
}

const API: &str = "~/code/acme-api";
const API_BILLING: &str = "~/code/acme-api.wt/billing-retry";
const API_SPIKE: &str = "~/code/acme-api.wt/old-spike";
const API_PG: &str = "~/code/acme-api.wt/pg17-upgrade";
const INK: &str = "~/code/inkwell";
const INK_THEME: &str = "~/code/inkwell.wt/dark-theme";
const TIDE: &str = "/Volumes/work/tidepool";
const DOT: &str = "~/dotfiles";

/// The launch directory used when `--cwd` is not given.
pub const DEFAULT_CWD: &str = "~/code/acme-api.wt/billing-retry/services/invoice";

pub fn sessions() -> Vec<Session> {
    use Provider::{Claude, Codex};
    let mut v = vec![
        B::new(Claude, "7f3c2a10-4be1-4c55-9d0e-1a2b3c4d5e01", 14)
            .title("Fix flaky invoice retry tests")
            .wt("acme-api", "billing-retry", API_BILLING)
            .sub("/services/invoice")
            .u("The retry test in invoice_retry_test.rs fails maybe one run in ten. Can you find out why?")
            .t("$ cargo test invoice_retry -- --nocapture\n…test retries_after_backoff ... FAILED")
            .a("The test sleeps for the backoff and then asserts on wall-clock time. Under load the scheduler wakes late, so the assertion races. I'll inject a fake clock instead of sleeping.")
            .u("Use the existing TestClock from the ledger crate rather than adding a new one.")
            .a("Done. retries_after_backoff now advances TestClock by the backoff interval. 200 consecutive runs pass.")
            .agent("Search the repo for other tests that sleep and assert on elapsed time.", "Found 3 more: ledger_settle_test, webhook_timeout_test, payout_batch_test.")
            .0,
        B::new(Codex, "019a6e2f-8c1d-7aa0-b3f4-5e6d7c8b9a02", 95)
            .title("Fix flaky invoice retry tests")
            .wt("acme-api", "main", API)
            .u("invoice retry tests are flaky on CI, only on the arm runners")
            .a("On arm runners the default tokio worker count is lower, which changes wake ordering. Pinning the test runtime to current_thread makes the result deterministic.")
            .t("$ gh run view 4411 --log | grep retries\nretries_after_backoff FAILED (attempt 2/3)")
            .u("ok, pin it and leave a comment explaining why")
            .a("Pinned with #[tokio::test(flavor = \"current_thread\")] and added a comment referencing the arm runner difference.")
            .0,
        B::new(Claude, "2b81d0aa-0c4f-4f8e-8a77-1a2b3c4d5e03", 60 * 24 * 9)
            .title("Fix flaky invoice retry tests")
            .wt("acme-api", "old-spike", API_SPIKE)
            .orphan()
            .u("spike: rewrite invoice retry as a state machine, see if the flaky test goes away")
            .a("The state machine version removes the sleep entirely; the test becomes a table of transitions. This is a larger change than the fake-clock fix.")
            .u("park this, we'll do the small fix first")
            .0,
        B::new(Claude, "c4d5e6f7-1111-4a2b-9c3d-1a2b3c4d5e04", 60 * 3)
            .title("Billing webhook signature check")
            .wt("acme-api", "billing-retry", API_BILLING)
            .u("Stripe-style webhook signatures: verify the timestamp tolerance is enforced before the HMAC comparison.")
            .a("Tolerance was checked after the comparison, so replayed payloads still paid the HMAC cost. Swapped the order and added a constant-time compare.")
            .t("$ rg 'verify_signature' -n\nsrc/webhooks/verify.rs:42")
            .u("add a test with a timestamp 6 minutes old")
            .a("Added rejects_stale_timestamp; it fails before the fix and passes after.")
            .0,
        B::new(Codex, "019a6d11-2222-7bb0-a1c2-5e6d7c8b9a05", 60 * 5)
            .wt("acme-api", "billing-retry", API_BILLING)
            .sub("/services/invoice")
            .u("why does InvoiceBatch hold a mutex across the await in flush()?")
            .a("It guards the pending queue, but holding it across the network call serializes every flush. Swap to taking the batch out of the queue first, then sending without the lock.")
            .u("do it")
            .a("Refactored flush() to drain under the lock and send outside it. Added a test with two concurrent flushes.")
            .0,
        B::new(Claude, "9e8d7c6b-3333-4c2d-8e1f-1a2b3c4d5e06", 60 * 26)
            .title("Postgres 17 upgrade plan")
            .wt("acme-api", "pg17-upgrade", API_PG)
            .u("Plan the Postgres 16 to 17 upgrade for acme-api. List extension compatibility issues first.")
            .a("pg_partman and pgcrypto are fine. The custom audit trigger uses a deprecated catalog column and needs a change before upgrade.")
            .agent("Check every migration file for uses of pg_attribute.attisdropped.", "Two migrations reference it; both are historical and do not run on fresh installs.")
            .u("write the plan into docs/pg17.md")
            .a("Wrote docs/pg17.md with a rehearsal step on a restored snapshot and a rollback section.")
            .0,
        B::new(Codex, "019a5c22-4444-7cc0-b2d3-5e6d7c8b9a07", 60 * 24 * 2)
            .title("Rate limiter for public API")
            .wt("acme-api", "main", API)
            .u("add a token bucket rate limiter on /v1/public, 100 req/min per key")
            .a("Added a Redis-backed token bucket in middleware/rate_limit.rs. Keys without a bucket get one lazily.")
            .t("$ curl -s -o /dev/null -w '%{http_code}' localhost:8080/v1/public (x101)\n429")
            .u("return Retry-After as well")
            .a("Retry-After is now set from the bucket's refill time, rounded up to whole seconds.")
            .0,
        B::new(Claude, "5a4b3c2d-5555-4d3e-9f2a-1a2b3c4d5e08", 60 * 24 * 4)
            .title("Explain ledger settlement flow")
            .wt("acme-api", "main", API)
            .sub("/crates/ledger")
            .u("walk me through how a payout gets settled in the ledger crate")
            .a("A payout is reserved, then posted in two legs: debit the merchant balance, credit the clearing account. Settlement closes the clearing entry once the bank file confirms it.")
            .u("where does the bank file get parsed?")
            .a("crates/ledger/src/bank_file.rs; it accepts fixed-width records and maps reason codes to settlement outcomes.")
            .0,
        B::new(Codex, "019a4b33-6666-7dd0-c3e4-5e6d7c8b9a09", 60 * 24 * 12)
            .title("Old spike: GraphQL gateway")
            .wt("acme-api", "old-spike", API_SPIKE)
            .orphan()
            .hidden()
            .u("try putting a GraphQL gateway in front of the REST API, just to see the shape")
            .a("Generated a schema from the OpenAPI spec. Nested resources map cleanly; pagination does not.")
            .0,
        B::new(Claude, "1f2e3d4c-7777-4e4f-8a3b-1a2b3c4d5e10", 40)
            .title("Dark theme tokens")
            .wt("inkwell", "dark-theme", INK_THEME)
            .u("Define semantic color tokens for inkwell's dark theme. Don't hardcode hex values in components.")
            .a("Added tokens.ts with surface, text, muted, accent, danger. Components now read from the token map.")
            .u("contrast on muted text is too low")
            .a("Raised muted from 3.9:1 to 4.6:1 against surface; checked against WCAG AA.")
            .0,
        B::new(Codex, "019a7a44-8888-7ee0-d4f5-5e6d7c8b9a11", 60 * 7)
            .wt("inkwell", "main", INK)
            .u("the markdown preview drops the last line when the file has no trailing newline")
            .a("The line splitter used split_terminator semantics on a stream; the final partial line was buffered and never flushed. Added a flush at end of stream.")
            .t("$ npm test -- preview\n✓ renders file without trailing newline")
            .0,
        B::new(Claude, "8d7c6b5a-9999-4f5a-9b4c-1a2b3c4d5e12", 60 * 24 * 3)
            .title("修复 日志 编码问题")
            .wt("inkwell", "main", INK)
            .u("Log files with CJK filenames show mojibake in the export panel. 日志 should render correctly.")
            .a("The export panel decoded paths as Latin-1. Switched to UTF-8 with lossless fallback and added a fixture named 日志.md.")
            .0,
        B::new(Claude, "7c6b5a4d-aaaa-4a6b-8c5d-1a2b3c4d5e13", 60 * 24 * 6)
            .title("🚀 Release checklist for 2.4")
            .wt("inkwell", "main", INK)
            .u("draft the release checklist for inkwell 2.4")
            .a("Checklist drafted: changelog, version bump, signed build, notarization, smoke test on a clean profile, announce.")
            .0,
        B::new(Codex, "019a3c55-bbbb-7ff0-e5a6-5e6d7c8b9a14", 60 * 24 * 20)
            .title("Sync engine conflict resolution")
            .wt("tidepool", "main", TIDE)
            .unknown()
            .u("design conflict resolution for the tidepool sync engine; last-writer-wins is losing edits")
            .a("Proposed per-field vector clocks with a merge function per field type. Text fields use a three-way merge against the common ancestor.")
            .u("write a property test for the merge")
            .a("Added a proptest that generates concurrent edit histories and checks convergence.")
            .0,
        B::new(Claude, "6b5a4c3d-cccc-4b7c-9d6e-1a2b3c4d5e15", 60 * 24 * 21)
            .title("Tidepool retry queue")
            .wt("tidepool", "main", TIDE)
            .unknown()
            .u("the sync retry queue grows without bound when offline")
            .a("Capped the queue and coalesce writes per document so only the latest pending write survives.")
            .0,
        B::new(Claude, "5c4d3e2f-dddd-4c8d-8e7f-1a2b3c4d5e16", 60 * 2)
            .wt("", "", DOT)
            .u("set up a zsh prompt that shows the git branch only inside repos")
            .a("Added a precmd hook using vcs_info; outside repos the segment is empty.")
            .0,
        B::new(Codex, "019a2d66-eeee-7a10-f6b7-5e6d7c8b9a17", 60 * 24)
            .folder("~/scratch/csv-cleanup")
            .u("clean up this CSV: dedupe by email, normalize phone numbers to E.164")
            .a("Wrote clean.py. 1,204 rows in, 1,131 out; 73 duplicates by lowercase email.")
            .t("$ python clean.py contacts.csv\nwrote contacts.clean.csv (1131 rows)")
            .0,
        B::new(Claude, "4d3e2f1a-ffff-4d9e-9f8a-1a2b3c4d5e18", 60 * 24 * 15)
            .folder("~/scratch/regex-tester")
            .orphan()
            .u("build a tiny regex tester page")
            .a("Single HTML file with live highlighting and a flags toggle.")
            .0,
        B::new(Claude, "3e2f1a0b-0000-4eaf-8a9b-1a2b3c4d5e19", 60 * 24 * 30)
            .folder("~/Downloads")
            .hidden()
            .u("what is in this zip?")
            .a("Three PDFs and a fonts folder.")
            .0,
        B::new(Codex, "019a1e77-1212-7b20-a7c8-5e6d7c8b9a20", 60 * 24 * 8)
            .title("Webhook retry backoff")
            .wt("acme-api", "main", API)
            .u("webhook deliveries retry immediately, add exponential backoff with jitter")
            .a("Backoff is now base 2s, factor 2, capped at 10 minutes, with full jitter.")
            .0,
    ];
    // Untitled project sessions in dotfiles use the folder form.
    for s in &mut v {
        if s.project.as_deref() == Some("") {
            s.project = None;
            s.branch = None;
        }
    }
    // One long conversation to exercise scrolling.
    let mut long = B::new(Claude, "2f1a0b9c-3434-4fb0-9bac-1a2b3c4d5e21", 60 * 30)
        .title("Invoice PDF rendering pass")
        .wt("acme-api", "billing-retry", API_BILLING);
    for i in 1..=18 {
        long = long
            .u(&format!("Step {i}: adjust the invoice PDF layout. The line-item table should wrap long descriptions and keep totals aligned to the right edge."))
            .t(&format!("$ cargo run --bin render-invoice fixtures/invoice_{i:02}.json\nwrote out/invoice_{i:02}.pdf"))
            .a(&format!("Step {i} done. Descriptions wrap at the column width; totals stay right-aligned. The retry footnote moved below the totals."));
    }
    v.push(long.0);
    v
}
