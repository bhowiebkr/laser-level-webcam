//! `laser report`: writes an issue report with the headings of the repository's issue form,
//! checks it, searches for duplicates and files it with the GitHub CLI (`gh`). It sends nothing
//! without `--submit`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const BUG_FORM: &str = include_str!("../../../.github/ISSUE_TEMPLATE/bug_report.yml");
const FEATURE_FORM: &str = include_str!("../../../.github/ISSUE_TEMPLATE/feature_request.yml");
/// Starts each line of the template that the reporter must replace.
const PROMPT: &str = "Replace this: ";
const FOOTER: &str = "\n\n---\nFiled with `laser report`: searched open and closed issues and ran its privacy check.\n";
/// Longer form links are cut back to the template and the title.
const MAX_URL: usize = 8000;
/// Words of the title that say nothing about the problem, left out of the duplicate search.
const COMMON: &[&str] = &[
    "after", "cannot", "does", "from", "have", "into", "should", "that", "this", "when", "with", "laser", "level",
    "webcam",
];

#[derive(Subcommand)]
pub enum Action {
    /// Write a report with the headings of the issue form. Fills in the build, the system and the cameras.
    Template { kind: Kind, report: PathBuf },
    /// Check a report, search open and closed issues, and show the account and the draft. Files
    /// it only with --submit.
    File {
        kind: Kind,
        report: PathBuf,
        /// The issue title. The form's prefix, such as "[Bug] ", is added when it is missing.
        #[arg(long)]
        title: String,
        /// File the issue with the account that is signed in to gh.
        #[arg(long)]
        submit: bool,
        /// The listed possible duplicates were read, and none is the same problem.
        #[arg(long)]
        not_duplicate: bool,
        /// A person read each private-text match.
        #[arg(long)]
        reviewed_privacy: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Kind {
    Bug,
    Feature,
}

impl Kind {
    fn form_text(self) -> &'static str {
        match self {
            Kind::Bug => BUG_FORM,
            Kind::Feature => FEATURE_FORM,
        }
    }

    fn template_file(self) -> &'static str {
        match self {
            Kind::Bug => "bug_report.yml",
            Kind::Feature => "feature_request.yml",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Bug => "bug",
            Kind::Feature => "feature",
        }
    }
}

/// Runs the action. Returns what to print, and whether it went through (false: exit 1).
pub fn run(action: Action) -> Result<(Value, bool)> {
    match action {
        Action::Template { kind, report } => {
            let facts = Facts { build: build_info(), system: system_info(), cameras: cameras_info() };
            let text = template(&Form::parse(kind.form_text()), &facts);
            write_new(&report, &text)?;
            let shown = report.display().to_string();
            let next = format!(
                "Replace each line that starts with \"{}\". Check the Cameras section for names that identify a person. Then run: laser report file {} {shown} --title \"Short summary\"",
                PROMPT.trim_end(),
                kind.name()
            );
            let out = json!({ "report": shown, "kind": kind.name(), "build": facts.build,
                "system": facts.system, "cameras": facts.cameras, "next": next });
            Ok((out, true))
        }
        Action::File { kind, report, title, submit, not_duplicate, reviewed_privacy } => {
            let text = fs::read_to_string(&report).with_context(|| format!("reading {}", report.display()))?;
            let flags = Flags { submit, not_duplicate, reviewed_privacy };
            let out = file(kind, &text, &title, flags, &repository(), &mut RealGh);
            let ok = out.ok;
            Ok((serde_json::to_value(out)?, ok))
        }
    }
}

/// `owner/name` of the repository in Cargo.toml.
fn repository() -> String {
    let url = env!("CARGO_PKG_REPOSITORY");
    url.trim_start_matches("https://github.com/").trim_end_matches('/').trim_end_matches(".git").to_string()
}

/// Creates the file and its folder; never replaces a file.
fn write_new(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("writing {} (an existing report is never replaced)", path.display()))?;
    f.write_all(text.as_bytes()).with_context(|| format!("writing {}", path.display()))
}

// ---- The issue form ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FieldKind {
    Dropdown,
    Input,
    Textarea,
}

#[derive(Debug)]
struct Field {
    kind: FieldKind,
    id: String,
    label: String,
    description: String,
    required: bool,
    options: Vec<String>,
}

#[derive(Debug)]
struct Form {
    title: String,
    labels: Vec<String>,
    fields: Vec<Field>,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let q = s.len() >= 2 && (s.starts_with('"') && s.ends_with('"') || s.starts_with('\'') && s.ends_with('\''));
    if q { s[1..s.len() - 1].to_string() } else { s.to_string() }
}

impl Form {
    /// Reads the fields that a report fills in (dropdowns, inputs and text areas) from a GitHub
    /// issue form, as the forms in this repository lay them out.
    fn parse(text: &str) -> Form {
        let mut form = Form { title: String::new(), labels: Vec::new(), fields: Vec::new() };
        let mut current: Option<(Option<FieldKind>, Field)> = None;
        let finish = |current: &mut Option<(Option<FieldKind>, Field)>, fields: &mut Vec<Field>| {
            if let Some((Some(kind), mut f)) = current.take() {
                f.kind = kind;
                fields.push(f);
            }
        };
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("title: ") {
                form.title = unquote(v);
            } else if let Some(v) = line.strip_prefix("labels: ") {
                let v = v.trim().trim_start_matches('[').trim_end_matches(']');
                form.labels = v.split(',').map(unquote).filter(|l| !l.is_empty()).collect();
            } else if let Some(v) = line.strip_prefix("  - type: ") {
                finish(&mut current, &mut form.fields);
                let kind = match v.trim() {
                    "dropdown" => Some(FieldKind::Dropdown),
                    "input" => Some(FieldKind::Input),
                    "textarea" => Some(FieldKind::Textarea),
                    _ => None,
                };
                let f = Field {
                    kind: FieldKind::Input,
                    id: String::new(),
                    label: String::new(),
                    description: String::new(),
                    required: false,
                    options: Vec::new(),
                };
                current = Some((kind, f));
            } else if let Some((_, f)) = current.as_mut() {
                if let Some(v) = line.strip_prefix("    id: ") {
                    f.id = unquote(v);
                } else if let Some(v) = line.strip_prefix("      label: ") {
                    f.label = unquote(v);
                } else if let Some(v) = line.strip_prefix("      description: ") {
                    f.description = unquote(v);
                } else if line == "      required: true" {
                    f.required = true;
                } else if let Some(v) = line.strip_prefix("        - ") {
                    f.options.push(unquote(v));
                }
            }
        }
        finish(&mut current, &mut form.fields);
        form
    }
}

// ---- The template ----

/// What the template fills in, read from this computer.
struct Facts {
    build: String,
    system: String,
    cameras: String,
}

fn template(form: &Form, facts: &Facts) -> String {
    let mut out = String::new();
    for f in &form.fields {
        let known = match f.id.as_str() {
            "build" => Some(&facts.build),
            "system" => Some(&facts.system),
            "cameras" => Some(&facts.cameras),
            _ => None,
        };
        let value = match (known, f.kind) {
            (Some(v), _) => v.clone(),
            (None, FieldKind::Dropdown) => format!("{PROMPT}one of: {}", f.options.join(" | ")),
            (None, _) if f.description.is_empty() => format!("{PROMPT}{}", f.label),
            (None, _) => format!("{PROMPT}{}", f.description),
        };
        let optional =
            if f.required || !value.starts_with(PROMPT) { "" } else { " (optional: you can remove this section)" };
        out.push_str(&format!("### {}\n\n{value}{optional}\n\n", f.label));
    }
    out
}

// ---- Checking the report ----

/// The `### Heading` sections of a report, in order. Text before the first heading is ignored.
fn sections(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            out.push((h.trim().to_string(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    for (_, body) in &mut out {
        *body = body.trim().to_string();
    }
    out
}

fn check(form: &Form, title: &str, secs: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    if title.trim().is_empty() {
        problems.push("The title is empty.".to_string());
    }
    for (heading, body) in secs {
        let Some(f) = form.fields.iter().find(|f| &f.label == heading) else {
            problems.push(format!("\"{heading}\" is not a section of the form."));
            continue;
        };
        if body.contains(PROMPT.trim_end()) {
            problems.push(format!("Replace the \"{}\" line in {heading}.", PROMPT.trim_end()));
        } else if f.kind == FieldKind::Dropdown && !body.is_empty() && !f.options.iter().any(|o| o == body) {
            problems.push(format!("{heading} must be one of: {}.", f.options.join(" | ")));
        }
    }
    for f in form.fields.iter().filter(|f| f.required) {
        match secs.iter().find(|(h, _)| h == &f.label) {
            None => problems.push(format!("The section {} is missing.", f.label)),
            Some((_, b)) if b.is_empty() => {
                problems.push(format!("{} is empty. Write NOT_ENOUGH_INFO and the reason if you do not know.", f.label))
            }
            _ => {}
        }
    }
    problems
}

/// The issue body, in the order of the form. An empty or left-out optional section says
/// "_No response_", as GitHub writes it.
fn body(form: &Form, secs: &[(String, String)]) -> String {
    let mut out = String::new();
    for f in &form.fields {
        let value = secs.iter().find(|(h, _)| h == &f.label).map_or("", |(_, b)| b.as_str());
        let value = if value.is_empty() { "_No response_" } else { value };
        out.push_str(&format!("### {}\n\n{value}\n\n", f.label));
    }
    out.trim_end().to_string()
}

// ---- Private text ----

#[derive(Serialize, Debug, PartialEq)]
pub struct PrivateMatch {
    /// "title", or "line N" of the report.
    place: String,
    kind: &'static str,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The character before byte `i` of `s`, if any.
fn before(s: &str, i: usize) -> Option<char> {
    s[..i].chars().next_back()
}

/// Byte offsets where `pat` starts in `s`, with no word character just before it.
fn starts<'a>(s: &'a str, pat: &'a str) -> impl Iterator<Item = usize> + 'a {
    s.match_indices(pat).map(|(i, _)| i).filter(move |&i| !before(s, i).is_some_and(is_word))
}

/// Length of the run at the start of `s` of characters that pass `f`.
fn run_len(s: &str, f: impl Fn(char) -> bool) -> usize {
    s.chars().take_while(|&c| f(c)).count()
}

fn absolute_path(line: &str) -> bool {
    let b = line.as_bytes();
    let drive = (0..b.len().saturating_sub(2)).any(|i| {
        b[i].is_ascii_alphabetic()
            && b[i + 1] == b':'
            && (b[i + 2] == b'\\' || b[i + 2] == b'/')
            && !(i > 0 && (is_word(b[i - 1] as char) || b[i - 1] == b'/'))
    });
    let home = ["/home/", "/Users/"].iter().any(|p| {
        line.match_indices(p).any(|(i, _)| {
            !before(line, i).is_some_and(|c| is_word(c) || c == '.')
                && line[i + p.len()..].chars().next().is_some_and(is_word)
        })
    });
    let unc = line.match_indices("\\\\").any(|(i, _)| {
        before(line, i) != Some('\\')
            && line[i + 2..].chars().next().is_some_and(|c| is_word(c) || c == '?' || c == '.')
    });
    drive || home || unc
}

fn credential(line: &str) -> bool {
    let token_char = |c: char| is_word(c) || c == '-';
    let gh = starts(line, "gh").any(|i| {
        let rest = &line[i + 2..];
        let mut cs = rest.chars();
        matches!(cs.next(), Some('p' | 'o' | 'u' | 's' | 'r'))
            && cs.next() == Some('_')
            && run_len(&rest[2..], is_word) >= 20
    });
    let pat = starts(line, "github_pat_").any(|i| run_len(&line[i + 11..], is_word) >= 20);
    let sk = starts(line, "sk-").any(|i| run_len(&line[i + 3..], token_char) >= 20);
    let aws =
        starts(line, "AKIA").any(|i| run_len(&line[i + 4..], |c| c.is_ascii_uppercase() || c.is_ascii_digit()) >= 16);
    let slack = starts(line, "xox").any(|i| {
        let mut cs = line[i + 3..].chars();
        matches!(cs.next(), Some('a' | 'b' | 'p' | 'r' | 's')) && cs.next() == Some('-')
    });
    let key = line.contains("-----BEGIN ") && line.contains("PRIVATE KEY");
    let lower = line.to_ascii_lowercase();
    let assigned = ["api_key", "api-key", "apikey", "password", "secret", "token"].iter().any(|k| {
        lower.match_indices(k).any(|(i, _)| {
            let rest = lower[i + k.len()..].trim_start_matches(['"', '\'', ' ']);
            let Some(rest) = rest.strip_prefix([':', '=']) else { return false };
            let rest = rest.trim_start_matches(['"', '\'', ' ']);
            run_len(rest, |c| !c.is_whitespace() && c != '"' && c != '\'') >= 8
        })
    });
    gh || pat || sk || aws || slack || key || assigned
}

fn email(line: &str) -> bool {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    line.match_indices('@').any(|(i, _)| {
        let left = line[..i].chars().rev().take_while(|&c| local(c)).count();
        let domain: String =
            line[i + 1..].chars().take_while(|&c| c.is_ascii_alphanumeric() || c == '.' || c == '-').collect();
        let domain = domain.trim_end_matches('.');
        let tld = domain.rsplit_once('.').map_or("", |(_, t)| t);
        left > 0 && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
    })
}

fn local_file(line: &str) -> bool {
    let ends_name = |rest: &str| !rest.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    let dot_local =
        line.match_indices(".local").any(|(i, _)| before(line, i).is_some_and(is_word) && ends_name(&line[i + 6..]));
    let env = line
        .match_indices(".env")
        .any(|(i, _)| !before(line, i).is_some_and(|c| c.is_ascii_alphanumeric()) && ends_name(&line[i + 4..]));
    dot_local || env
}

/// A kind of private text, and the test for one line.
type Check = (&'static str, fn(&str) -> bool);

/// Places in the title and the report that look private. Only the place and the kind are given,
/// never the text, so the output can be shown safely.
fn private_text(title: &str, text: &str) -> Vec<PrivateMatch> {
    let checks: [Check; 5] = [
        ("absolute path", absolute_path),
        ("credential", credential),
        ("email address", email),
        ("local-only file", local_file),
        ("hidden HTML comment", |l: &str| l.contains("<!--")),
    ];
    let lines = std::iter::once(("title".to_string(), title))
        .chain(text.lines().enumerate().map(|(n, l)| (format!("line {}", n + 1), l)));
    let mut out = Vec::new();
    for (place, line) in lines {
        for (kind, f) in checks {
            if f(line) {
                out.push(PrivateMatch { place: place.clone(), kind });
            }
        }
    }
    out
}

// ---- GitHub ----

pub struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `gh`. `None` when it could not start.
pub trait Gh {
    fn run(&mut self, args: &[&str], stdin: Option<&str>) -> Option<Output>;
}

struct RealGh;

impl Gh for RealGh {
    fn run(&mut self, args: &[&str], stdin: Option<&str>) -> Option<Output> {
        let mut cmd = Command::new("gh");
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child = cmd.spawn().ok()?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes()).ok()?;
        }
        let o = child.wait_with_output().ok()?;
        Some(Output {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Issue {
    number: u64,
    title: String,
    state: String,
    url: String,
}

/// Up to five words of the title that can find the same problem.
fn search_words(title: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for w in title.split(|c: char| !c.is_alphabetic()) {
        let w = w.to_lowercase();
        if w.chars().count() >= 4 && !COMMON.contains(&w.as_str()) && !words.contains(&w) {
            words.push(w);
        }
    }
    words.truncate(5);
    words
}

fn duplicates(gh: &mut dyn Gh, repo: &str, title: &str) -> Vec<Issue> {
    let words = search_words(title);
    if words.is_empty() {
        return Vec::new();
    }
    let query = words.join(" OR ");
    let args = [
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        "all",
        "--search",
        &query,
        "--json",
        "number,title,state,url",
        "--limit",
        "10",
    ];
    match gh.run(&args, None) {
        Some(o) if o.code == 0 => serde_json::from_str(&o.stdout).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn percent(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A link to the issue form with the title and each section filled in, for a person to submit.
fn form_url(kind: Kind, form: &Form, repo: &str, title: &str, secs: &[(String, String)]) -> String {
    let base =
        format!("https://github.com/{repo}/issues/new?template={}&title={}", kind.template_file(), percent(title));
    let mut url = base.clone();
    for f in &form.fields {
        if let Some((_, b)) = secs.iter().find(|(h, _)| h == &f.label).filter(|(_, b)| !b.is_empty()) {
            url.push_str(&format!("&{}={}", percent(&f.id), percent(b)));
        }
    }
    if url.len() > MAX_URL { base } else { url }
}

#[derive(Clone, Copy, Default)]
pub struct Flags {
    submit: bool,
    not_duplicate: bool,
    reviewed_privacy: bool,
}

#[derive(Serialize, Debug)]
pub struct Filed {
    ok: bool,
    problems: Vec<String>,
    private: Vec<PrivateMatch>,
    repository: String,
    /// The account signed in to gh, which files the issue.
    account: Option<String>,
    duplicates: Vec<Issue>,
    title: String,
    body: String,
    /// Only when gh cannot be used: the filled-in form, for a person to submit.
    form_url: Option<String>,
    sent: bool,
    url: Option<String>,
    next: String,
}

pub fn file(kind: Kind, text: &str, title: &str, flags: Flags, repo: &str, gh: &mut dyn Gh) -> Filed {
    let form = Form::parse(kind.form_text());
    let short = title.trim().strip_prefix(form.title.trim()).unwrap_or(title.trim()).trim();
    let full = format!("{}{short}", form.title);
    let secs = sections(text);
    let problems = check(&form, short, &secs);
    let private = private_text(&full, text);
    let body = format!("{}{FOOTER}", body(&form, &secs));
    let account = gh
        .run(&["api", "user", "--jq", ".login"], None)
        .filter(|o| o.code == 0)
        .map(|o| o.stdout.trim().to_string())
        .filter(|a| !a.is_empty());
    let dups = if account.is_some() { duplicates(gh, repo, short) } else { Vec::new() };
    let form_url = account.is_none().then(|| form_url(kind, &form, repo, &full, &secs));
    let mut out = Filed {
        ok: false,
        problems,
        private,
        repository: repo.to_string(),
        account,
        duplicates: dups,
        title: full,
        body,
        form_url,
        sent: false,
        url: None,
        next: String::new(),
    };
    out.next = if !out.problems.is_empty() {
        "Fix the problems in the report, then run this again.".into()
    } else if !out.private.is_empty() && !flags.reviewed_privacy {
        "A person reads each private-text match. Remove the private text, or run again with --reviewed-privacy.".into()
    } else if !out.duplicates.is_empty() && !flags.not_duplicate {
        "Read the possible duplicates. If none is the same problem, run again with --not-duplicate.".into()
    } else if !flags.submit {
        out.ok = true;
        match &out.account {
            Some(a) => format!(
                "Show the draft and the account ({a}) to the person. With their approval, run again with --submit."
            ),
            None => {
                "gh is not installed or not signed in. A person opens form_url, checks the form and submits it.".into()
            }
        }
    } else if out.account.is_none() {
        "gh is not installed or not signed in, so nothing was sent. A person opens form_url, checks the form and submits it.".into()
    } else {
        match create(gh, repo, &out.title, &out.body, form.labels.first().map(String::as_str)) {
            Ok(url) => {
                out.ok = true;
                out.sent = true;
                out.next = format!("Filed: {url}");
                out.url = Some(url);
                out.next.clone()
            }
            Err(e) => format!("gh could not file the issue: {e}"),
        }
    };
    out
}

/// Files the issue. If the repository refuses the label, files it again without the label.
fn create(gh: &mut dyn Gh, repo: &str, title: &str, body: &str, label: Option<&str>) -> Result<String, String> {
    let mut args = vec!["issue", "create", "--repo", repo, "--title", title, "--body-file", "-"];
    if let Some(l) = label {
        args.extend(["--label", l]);
    }
    let mut o = gh.run(&args, Some(body)).ok_or("gh did not start")?;
    if o.code != 0 && label.is_some() && o.stderr.to_lowercase().contains("label") {
        args.truncate(args.len() - 2);
        o = gh.run(&args, Some(body)).ok_or("gh did not start")?;
    }
    if o.code == 0 { Ok(o.stdout.trim().to_string()) } else { Err(o.stderr.trim().to_string()) }
}

// ---- Facts about this computer ----

fn build_info() -> String {
    let commit = option_env!("LASER_COMMIT").unwrap_or("commit unknown");
    let target = option_env!("LASER_TARGET").unwrap_or(std::env::consts::ARCH);
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    format!("laser {} ({commit}, {target}, {profile})", env!("CARGO_PKG_VERSION"))
}

/// The first line of a command's output, if it ran and printed one.
fn first_line(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let o = cmd.output().ok().filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&o.stdout);
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

#[cfg(windows)]
fn os_info() -> String {
    const PS: &str = "$o = Get-CimInstance Win32_OperatingSystem; \
        $g = (Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + ' (driver ' + $_.DriverVersion + ')' }) -join ', '; \
        $o.Caption + ' ' + $o.Version + '; GPU: ' + $g";
    let os = first_line("powershell", &["-NoProfile", "-NonInteractive", "-Command", PS])
        .or_else(|| first_line("cmd", &["/C", "ver"]))
        .unwrap_or_else(|| "Windows, version unknown".into());
    let wine = first_line("reg", &["query", r"HKLM\Software\Wine"]).is_some();
    if wine { format!("{os}; Wine") } else { os }
}

#[cfg(target_os = "macos")]
fn os_info() -> String {
    let version = first_line("sw_vers", &["-productVersion"]).unwrap_or_else(|| "version unknown".into());
    let machine = first_line("uname", &["-m"]).unwrap_or_default();
    let rosetta = first_line("sysctl", &["-n", "sysctl.proc_translated"]).as_deref() == Some("1");
    format!("macOS {version} {machine}{}", if rosetta { "; Rosetta" } else { "" })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_info() -> String {
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let name =
        release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map_or_else(|| "Linux".to_string(), unquote);
    let kernel = first_line("uname", &["-r"]).unwrap_or_default();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "session unknown".into());
    format!("{name}; kernel {kernel}; {session}")
}

fn system_info() -> String {
    format!("{}; laser built for {} {}", os_info(), std::env::consts::OS, std::env::consts::ARCH)
}

/// Each camera as "name (backend, USB vendor:product)". The device id is left out: it can hold
/// a serial number or a system path.
fn cameras_info() -> String {
    let cams: Vec<String> = laser_camera::devices()
        .into_iter()
        .filter(|d| d.backend != laser_camera::Backend::Synthetic)
        .map(|d| match laser_camera::usb(&d) {
            Ok(u) => format!("- {} ({:?}, USB {:04x}:{:04x})", d.name, d.backend, u.vendor_id, u.product_id),
            Err(_) => format!("- {} ({:?})", d.name, d.backend),
        })
        .collect();
    if cams.is_empty() { "No camera found.".into() } else { cams.join("\n") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Answers each gh call from a list, and keeps the calls.
    #[derive(Default)]
    struct FakeGh {
        login: Option<&'static str>,
        issues: &'static str,
        create: VecDeque<Output>,
        calls: Vec<(Vec<String>, Option<String>)>,
        missing: bool,
    }

    fn ok(stdout: &str) -> Output {
        Output { code: 0, stdout: stdout.into(), stderr: String::new() }
    }

    fn fail(stderr: &str) -> Output {
        Output { code: 1, stdout: String::new(), stderr: stderr.into() }
    }

    impl Gh for FakeGh {
        fn run(&mut self, args: &[&str], stdin: Option<&str>) -> Option<Output> {
            if self.missing {
                return None;
            }
            self.calls.push((args.iter().map(|s| s.to_string()).collect(), stdin.map(str::to_string)));
            match args {
                ["api", "user", ..] => Some(self.login.map_or_else(|| fail("not logged in"), ok)),
                ["issue", "list", ..] => Some(ok(if self.issues.is_empty() { "[]" } else { self.issues })),
                ["issue", "create", ..] => self.create.pop_front(),
                _ => None,
            }
        }
    }

    impl FakeGh {
        fn created(&self) -> Vec<&Vec<String>> {
            self.calls.iter().map(|(a, _)| a).filter(|a| a[1] == "create").collect()
        }
    }

    fn facts() -> Facts {
        Facts { build: "laser 2.0.0 (abc, x, debug)".into(), system: "Test OS".into(), cameras: "- Cam (V4l2)".into() }
    }

    /// The template with each prompt replaced by a valid value.
    fn filled(kind: Kind) -> String {
        let form = Form::parse(kind.form_text());
        let mut text = template(&form, &facts());
        for f in &form.fields {
            let value =
                if f.kind == FieldKind::Dropdown { f.options[0].clone() } else { format!("Text for {}.", f.id) };
            let line =
                text.lines().find(|l| l.starts_with(PROMPT) && text.contains(&format!("### {}\n\n{l}", f.label)));
            if let Some(line) = line.map(str::to_string) {
                text = text.replacen(&line, &value, 1);
            }
        }
        text
    }

    #[test]
    fn reads_both_forms() {
        let bug = Form::parse(BUG_FORM);
        assert_eq!(bug.title, "[Bug] ");
        assert_eq!(bug.labels, ["bug"]);
        let ids: Vec<&str> = bug.fields.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "program",
                "area",
                "observed-behavior",
                "reproduction",
                "expected-behavior",
                "impact",
                "build",
                "system",
                "cameras",
                "evidence",
                "reporter"
            ]
        );
        let program = &bug.fields[0];
        assert_eq!(program.kind, FieldKind::Dropdown);
        assert!(program.required);
        assert!(program.options.contains(&"Mac app".to_string()));
        let cameras = bug.fields.iter().find(|f| f.id == "cameras").unwrap();
        assert!(!cameras.required && cameras.kind == FieldKind::Textarea);
        let evidence = bug.fields.iter().find(|f| f.id == "evidence").unwrap();
        assert!(evidence.description.starts_with("`laser` output"), "{}", evidence.description);

        let feature = Form::parse(FEATURE_FORM);
        assert_eq!(feature.title, "[Feature] ");
        assert_eq!(feature.labels, ["enhancement"]);
        assert_eq!(feature.fields.len(), 7);
    }

    #[test]
    fn template_fills_in_the_facts_and_prompts_the_rest() {
        let text = template(&Form::parse(BUG_FORM), &facts());
        assert!(text.contains("### Build\n\nlaser 2.0.0 (abc, x, debug)\n"));
        assert!(text.contains("### System\n\nTest OS\n"));
        assert!(text.contains("### Cameras\n\n- Cam (V4l2)\n\n### Evidence\n\nReplace this: `laser` output"));
        assert!(text.contains("note what still works. (optional: you can remove this section)"));
        assert!(text.contains("### Program\n\nReplace this: one of: Windows app | Mac app"));
        let problems = check(&Form::parse(BUG_FORM), "Title", &sections(&text));
        assert!(problems.iter().any(|p| p.contains("Observed behavior")), "{problems:?}");
    }

    #[test]
    fn a_filled_template_has_no_problems() {
        for kind in [Kind::Bug, Kind::Feature] {
            let text = filled(kind);
            let problems = check(&Form::parse(kind.form_text()), "A title", &sections(&text));
            assert!(problems.is_empty(), "{kind:?}: {problems:?}\n{text}");
        }
    }

    #[test]
    fn finds_problems() {
        let form = Form::parse(BUG_FORM);
        let text = filled(Kind::Bug)
            .replace("### Impact", "### Impakt")
            .replacen("Windows app", "Amiga app", 1)
            .replace("Text for reproduction.", "");
        let problems = check(&form, " ", &sections(&text));
        let has = |s: &str| problems.iter().any(|p| p.contains(s));
        assert!(has("title is empty"), "{problems:?}");
        assert!(has("\"Impakt\" is not a section"), "{problems:?}");
        assert!(has("section Impact is missing"), "{problems:?}");
        assert!(has("Program must be one of"), "{problems:?}");
        assert!(has("Reproduction is empty"), "{problems:?}");
    }

    #[test]
    fn body_follows_the_form() {
        let form = Form::parse(BUG_FORM);
        let text = filled(Kind::Bug).replace("- Cam (V4l2)", "");
        let b = body(&form, &sections(&text));
        assert!(b.starts_with("### Program\n\nWindows app\n\n### Area"));
        assert!(b.contains("### Cameras\n\n_No response_\n\n### Evidence"));
    }

    #[test]
    fn finds_each_kind_of_private_text() {
        let cases = [
            (r"Saved to C:\Users\me\frame.pgm", "absolute path"),
            ("see D:/data/x", "absolute path"),
            ("in /home/alice/frames", "absolute path"),
            ("on /Users/bob/Desktop", "absolute path"),
            (r"share \\server\camera", "absolute path"),
            (r"id \\?\usb#vid_046d", "absolute path"),
            ("token ghp_abcdefghijklmnopqrstuvwxyz0123", "credential"),
            ("github_pat_11ABCDEFG0123456789_abcdefghij", "credential"),
            ("key sk-abcdefghijklmnopqrstuvwx", "credential"),
            ("AKIAABCDEFGHIJKLMNOP", "credential"),
            ("xoxb-123-456", "credential"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "credential"),
            ("password = hunter2hunter2", "credential"),
            ("API_KEY: \"abcdefgh1234\"", "credential"),
            ("mail me at someone@example.com", "email address"),
            ("my AGENTS.local.md says", "local-only file"),
            ("settings.local.json", "local-only file"),
            ("read .env first", "local-only file"),
            ("<!-- hidden -->", "hidden HTML comment"),
        ];
        for (line, kind) in cases {
            let found = private_text("Title", line);
            assert!(found.iter().any(|m| m.kind == kind && m.place == "line 1"), "{line:?} → {found:?}");
        }
        assert_eq!(
            private_text("ask someone@example.com", ""),
            [PrivateMatch { place: "title".into(), kind: "email address" }]
        );
    }

    #[test]
    fn plain_text_is_not_private() {
        let text = "\
The Wait step reads 0.3 µm at 10:30, on Windows 11 and macOS 14.
See https://github.com/bhowiebkr/laser-level-webcam/issues/3 and docs/wiki.
A task-based run used 20 frames; the tokens: none. Ask @someone on GitHub.
It ran on localhost and in the environment of the app. git@github version.
Mean 103/255 at 1920x1080, ratio 3:2.";
        let found = private_text("[Bug] Wait never reaches Ready", text);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn search_words_drop_short_and_common_words() {
        assert_eq!(
            search_words("The Mac app does not show the camera list when the laser is on"),
            ["show", "camera", "list"]
        );
        assert_eq!(search_words("one two six"), Vec::<String>::new());
        assert_eq!(search_words("alpha bravo charlie delta echoes foxtrot golf").len(), 5);
    }

    #[test]
    fn file_without_submit_shows_the_draft_and_sends_nothing() {
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        let out = file(Kind::Bug, &filled(Kind::Bug), "Camera list is empty", Flags::default(), "o/r", &mut gh);
        assert!(out.ok, "{out:?}");
        assert!(!out.sent);
        assert_eq!(out.account.as_deref(), Some("tester"));
        assert_eq!(out.title, "[Bug] Camera list is empty");
        assert!(out.body.ends_with("ran its privacy check.\n"));
        assert!(out.form_url.is_none());
        assert!(out.next.contains("tester") && out.next.contains("--submit"));
        assert!(gh.created().is_empty());
        let search = gh.calls.iter().find(|(a, _)| a[1] == "list").unwrap();
        assert!(search.0.contains(&"camera OR list OR empty".to_string()), "{search:?}");
    }

    #[test]
    fn the_title_prefix_is_not_added_twice() {
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        let out = file(Kind::Bug, &filled(Kind::Bug), "[Bug] Camera list is empty", Flags::default(), "o/r", &mut gh);
        assert_eq!(out.title, "[Bug] Camera list is empty");
    }

    #[test]
    fn duplicates_block_until_read() {
        let issues = r#"[{"number":7,"title":"Camera list empty on Mac","state":"OPEN","url":"https://github.com/o/r/issues/7"}]"#;
        let mut gh = FakeGh { login: Some("tester"), issues, ..FakeGh::default() };
        let submit = Flags { submit: true, ..Flags::default() };
        let out = file(Kind::Bug, &filled(Kind::Bug), "Camera list is empty", submit, "o/r", &mut gh);
        assert!(!out.ok && !out.sent);
        assert_eq!(out.duplicates[0].number, 7);
        assert!(out.next.contains("--not-duplicate"));
        assert!(gh.created().is_empty());

        gh.create.push_back(ok("https://github.com/o/r/issues/8\n"));
        let flags = Flags { submit: true, not_duplicate: true, ..Flags::default() };
        let out = file(Kind::Bug, &filled(Kind::Bug), "Camera list is empty", flags, "o/r", &mut gh);
        assert!(out.ok && out.sent, "{out:?}");
        assert_eq!(out.url.as_deref(), Some("https://github.com/o/r/issues/8"));
        let created = gh.created();
        assert_eq!(created.len(), 1);
        assert!(created[0].windows(2).any(|w| w == ["--label", "bug"]));
        let sent_body = gh.calls.iter().find(|(a, _)| a[1] == "create").and_then(|(_, s)| s.clone()).unwrap();
        assert_eq!(sent_body, out.body);
    }

    #[test]
    fn a_refused_label_is_left_out() {
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        gh.create.push_back(fail("could not add label: 'enhancement' not found"));
        gh.create.push_back(ok("https://github.com/o/r/issues/9"));
        let flags = Flags { submit: true, ..Flags::default() };
        let out = file(Kind::Feature, &filled(Kind::Feature), "Gain on the Mac", flags, "o/r", &mut gh);
        assert!(out.sent, "{out:?}");
        let created = gh.created();
        assert_eq!(created.len(), 2);
        assert!(created[0].contains(&"--label".to_string()));
        assert!(!created[1].contains(&"--label".to_string()));
    }

    #[test]
    fn other_create_errors_are_not_retried() {
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        gh.create.push_back(fail("HTTP 502"));
        let flags = Flags { submit: true, ..Flags::default() };
        let out = file(Kind::Bug, &filled(Kind::Bug), "Camera list is empty", flags, "o/r", &mut gh);
        assert!(!out.ok && !out.sent);
        assert!(out.next.contains("HTTP 502"));
        assert_eq!(gh.created().len(), 1);
    }

    #[test]
    fn without_gh_a_filled_form_link_is_given() {
        for mut gh in [FakeGh { missing: true, ..FakeGh::default() }, FakeGh::default()] {
            let flags = Flags { submit: true, ..Flags::default() };
            let out = file(Kind::Bug, &filled(Kind::Bug), "Camera list is empty", flags, "o/r", &mut gh);
            assert!(!out.ok && !out.sent && out.account.is_none());
            let url = out.form_url.unwrap();
            assert!(url.starts_with("https://github.com/o/r/issues/new?template=bug_report.yml&title=%5BBug%5D%20Camera%20list%20is%20empty&"), "{url}");
            assert!(url.contains("&program=Windows%20app"), "{url}");
            assert!(gh.created().is_empty());
        }
    }

    #[test]
    fn a_long_form_link_is_cut_back_to_the_title() {
        let text = filled(Kind::Bug).replace("Text for evidence.", &"frame ".repeat(2000));
        let mut gh = FakeGh { missing: true, ..FakeGh::default() };
        let out = file(Kind::Bug, &text, "Camera list is empty", Flags::default(), "o/r", &mut gh);
        let url = out.form_url.unwrap();
        assert!(url.ends_with("&title=%5BBug%5D%20Camera%20list%20is%20empty"), "{url}");
        assert!(url.len() < MAX_URL);
    }

    #[test]
    fn private_text_blocks_until_reviewed() {
        let text = filled(Kind::Bug).replace("Text for evidence.", r"Frame saved to C:\Users\me\f.pgm");
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        let submit = Flags { submit: true, ..Flags::default() };
        let out = file(Kind::Bug, &text, "Camera list is empty", submit, "o/r", &mut gh);
        assert!(!out.ok && !out.sent);
        assert_eq!(out.private.len(), 1);
        assert_eq!(out.private[0].kind, "absolute path");
        assert!(out.next.contains("--reviewed-privacy"));
        let json = serde_json::to_string(&out.private).unwrap();
        assert!(!json.contains("Users"), "the match must not repeat the text: {json}");

        gh.create.push_back(ok("https://github.com/o/r/issues/10"));
        let flags = Flags { submit: true, reviewed_privacy: true, ..Flags::default() };
        let out = file(Kind::Bug, &text, "Camera list is empty", flags, "o/r", &mut gh);
        assert!(out.sent, "{out:?}");
    }

    #[test]
    fn problems_block_filing() {
        let mut gh = FakeGh { login: Some("tester"), ..FakeGh::default() };
        let flags = Flags { submit: true, not_duplicate: true, reviewed_privacy: true };
        let text = template(&Form::parse(BUG_FORM), &facts());
        let out = file(Kind::Bug, &text, "Camera list is empty", flags, "o/r", &mut gh);
        assert!(!out.ok && !out.sent && !out.problems.is_empty());
        assert!(gh.created().is_empty());
    }

    #[test]
    fn the_template_is_never_written_over() {
        let dir = std::env::temp_dir().join(format!("laser-report-test-{}", std::process::id()));
        let path = dir.join("sub").join("report.md");
        write_new(&path, "one").unwrap();
        assert!(write_new(&path, "two").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn repository_comes_from_cargo_toml() {
        assert_eq!(repository(), "bhowiebkr/laser-level-webcam");
    }
}
