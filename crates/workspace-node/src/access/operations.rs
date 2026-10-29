//! What a remote device asked this device to do, described by the target for its own audit
//! log and approval prompts. Summaries are redacted here, before they are stored or shown:
//! they never carry file contents, message bodies, environment values or credential-shaped
//! text.
use super::{Operation, OperationDetail, Risk};
use serde_json::Value;
use std::collections::BTreeMap;

const SUMMARY: usize = 240;

/// Describes a mesh call. `route` is the matched route without the `/api/v1/mesh` prefix
/// (e.g. `/nodes/{node}/exec`); `None` means the call is not recorded on its own (an upload
/// chunk after the first, for example).
pub fn describe(
    method: &str,
    route: &str,
    query: &BTreeMap<String, String>,
    body: &Value,
) -> Option<Operation> {
    use Risk::*;
    let segments: Vec<_> = route.trim_start_matches('/').split('/').collect();
    let get = method == "GET";
    let operation = body["operation"].as_str().unwrap_or("");
    let text = |key: &str| body[key].as_str().map(redact).unwrap_or_default();
    let (kind, risk, summary) = match segments.as_slice() {
        ["jobs", _] => ("exec.status".to_owned(), Read, String::new()),
        ["jobs", _, "cancel"] => ("exec.cancel".to_owned(), Change, String::new()),
        ["nodes", _, "exec"] => {
            let (program, args) = program_args(body);
            let risk = if read_only_command(program, &args) {
                Read
            } else {
                Danger
            };
            ("exec.run".to_owned(), risk, command(body))
        }
        ["nodes", _, "files", "write"] => {
            if body["offset"].as_u64().unwrap_or(0) != 0 {
                return None;
            }
            ("files.write".to_owned(), Danger, text("path"))
        }
        ["nodes", _, "files", "read"] => {
            if body["offset"].as_u64().unwrap_or(0) != 0 {
                return None;
            }
            ("files.read".to_owned(), Danger, text("path"))
        }
        ["nodes", _, "terminal", "sessions"] => match operation {
            "list" | "" => ("terminal.list".to_owned(), Read, String::new()),
            "create" => (
                "terminal.create".to_owned(),
                Danger,
                join(&[text("name"), text("cwd")]),
            ),
            "rename" => ("terminal.rename".to_owned(), Change, text("name")),
            other => (format!("terminal.{}", word(other)), Change, text("session")),
        },
        ["nodes", _, "terminal", "ws"] => (
            "terminal.attach".to_owned(),
            Danger,
            query.get("session").map(|s| redact(s)).unwrap_or_default(),
        ),
        ["nodes", _, "desktop", ..] => ("desktop.view".to_owned(), Danger, String::new()),
        ["nodes", _, "computer", ..] => (
            format!("computer.{}", word(or(operation, "control"))),
            Danger,
            String::new(),
        ),
        ["nodes", _, "browser", ..] => (
            format!("browser.{}", word(or(operation, "control"))),
            Danger,
            body["url"].as_str().map(redact).unwrap_or_default(),
        ),
        ["nodes", _, "projects", "sync"] => ("projects.sync".to_owned(), Read, String::new()),
        ["nodes", _, "projects", "copy-status"] => {
            ("projects.copy_status".to_owned(), Read, String::new())
        }
        [
            "nodes",
            _,
            "projects",
            action @ ("export" | "import" | "transfer"),
        ] => (
            format!("projects.{action}"),
            Danger,
            join(&[text("project"), text("name"), text("path")]),
        ),
        ["nodes", _, "projects"] => match operation {
            "list" | "" => ("projects.list".to_owned(), Read, String::new()),
            "bind" => (
                "projects.bind".to_owned(),
                Danger,
                join(&[text("project"), text("path")]),
            ),
            other => (
                format!("projects.{}", word(other)),
                Change,
                join(&[text("project"), text("name")]),
            ),
        },
        ["nodes", _, "executions", ..] => (
            format!(
                "agent.{}",
                word(or(operation, if get { "status" } else { "run" }))
            ),
            if get { Read } else { Danger },
            String::new(),
        ),
        ["nodes", _, "spaces", "control"] => {
            let action = body["action"].as_str().unwrap_or("control");
            (format!("spaces.{}", word(action)), Change, text("id"))
        }
        ["nodes", _, "spaces", ..] => ("spaces.list".to_owned(), Read, String::new()),
        ["nodes", _, "update", "rollback"] => ("update.rollback".to_owned(), Danger, String::new()),
        ["nodes", _, "update", ..] if get => ("update.check".to_owned(), Read, String::new()),
        ["nodes", _, "update", ..] => ("update.install".to_owned(), Danger, text("version")),
        ["nodes", _, "activity", ..] => ("activity.read".to_owned(), Read, String::new()),
        ["nodes", _, "messages", ..] | ["nodes", _, "tasks", ..] if get => {
            (format!("{}.read", segments[2]), Read, String::new())
        }
        // Message and task bodies are the requester's words: never copied into the log.
        ["nodes", _, "messages", ..] => ("messages.send".to_owned(), Change, String::new()),
        ["nodes", _, "tasks", ..] => ("tasks.send".to_owned(), Change, String::new()),
        ["nodes", _, "agent-sessions", ..] => {
            ("agent_sessions.read".to_owned(), Read, String::new())
        }
        ["nodes", _, "llm", ..] => ("llm.use".to_owned(), Change, String::new()),
        ["nodes", _, "vault", rest @ ..] => (
            format!("vault.{}", word(rest.first().copied().unwrap_or("use"))),
            Danger,
            String::new(),
        ),
        ["nodes", _, other, ..] => (
            format!("{}.{}", word(other), if get { "read" } else { "call" }),
            if get { Read } else { Change },
            String::new(),
        ),
        _ => return None,
    };
    let confirm = !findings(&kind, body).is_empty();
    Some(Operation {
        kind,
        risk,
        summary: truncate(&summary),
        confirm,
    })
}
fn or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}
/// A short identifier taken from requester input, limited to safe characters.
fn word(value: &str) -> String {
    let word: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect();
    if word.is_empty() {
        "call".into()
    } else {
        word.replace('-', "_")
    }
}
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}
fn truncate(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.chars().count() <= SUMMARY {
        return text;
    }
    let mut short: String = text.chars().take(SUMMARY - 1).collect();
    short.push('…');
    short
}
fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || "'\"\\$`|;&<>".contains(c))
    {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}
fn program_args(body: &Value) -> (&str, Vec<&str>) {
    (
        body["program"].as_str().unwrap_or(""),
        body["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str())
            .collect(),
    )
}
/// Built-in rule findings for an operation (see `command_findings`). Any finding makes it
/// confirm-every-time; the automatic reviewer cannot clear one.
pub fn findings(kind: &str, body: &Value) -> Vec<&'static str> {
    let path = body["path"].as_str().unwrap_or("");
    match kind {
        "exec.run" => {
            let (program, args) = program_args(body);
            command_findings(program, &args)
        }
        "files.write" if sensitive_path(path) => vec!["login_files"],
        "files.read" if credential_path(path) => vec!["credential_access"],
        _ => vec![],
    }
}
/// The operation's full parameters, redacted the same way as its summary.
pub fn detail(kind: &str, query: &BTreeMap<String, String>, body: &Value) -> OperationDetail {
    let text = |key: &str| {
        body[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| truncate(&redact(s)))
    };
    let mut detail = OperationDetail::default();
    match kind {
        "exec.run" => {
            detail.argv = argv(body);
            detail.cwd = text("cwd");
            detail.env_keys = body["env"]
                .as_object()
                .map(|e| e.keys().map(|k| word(k)).collect())
                .unwrap_or_default();
            detail.timeout_secs = body["timeout_secs"].as_u64();
        }
        "files.write" | "files.read" => {
            detail.paths = text("path").into_iter().collect();
            detail.bytes = body["size"].as_u64();
        }
        "terminal.attach" => {
            detail.target = query.get("session").map(|s| truncate(&redact(s)));
        }
        k if k.starts_with("terminal.") => {
            detail.target = text("session").or_else(|| text("name"));
            detail.cwd = text("cwd");
        }
        k if k.starts_with("browser.") => detail.url = text("url"),
        k if k.starts_with("spaces.") => detail.space = text("id"),
        k if k.starts_with("projects.") => {
            detail.target = text("project").or_else(|| text("name"));
            detail.paths = text("path").into_iter().collect();
        }
        "update.install" => detail.target = text("version"),
        _ => {}
    }
    detail.findings = findings(kind, body)
        .into_iter()
        .map(str::to_owned)
        .collect();
    detail
}
/// Each argument redacted on its own; the value after a secret-named flag is masked.
fn argv(body: &Value) -> Vec<String> {
    let (program, args) = program_args(body);
    let mut out = vec![truncate(&redact(program))];
    let mut mask_next = false;
    for arg in args.into_iter().take(256) {
        if mask_next {
            out.push("***".into());
            mask_next = false;
            continue;
        }
        if arg.starts_with('-') && !arg.contains('=') && secret_name(arg) {
            mask_next = true;
        }
        out.push(clip(&redact(arg), 2000));
    }
    out
}
fn clip(text: &str, max: usize) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    if text.chars().count() <= max {
        return text;
    }
    let mut short: String = text.chars().take(max - 1).collect();
    short.push('…');
    short
}
/// The last `max` characters of command output, redacted, for the log.
pub fn tail(output: &[u8], max: usize) -> Option<String> {
    if output.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(output);
    let start = text
        .char_indices()
        .rev()
        .nth(max.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    // Start at a whitespace boundary so a token is never half-redacted.
    let piece = &text[start..];
    let piece = if start > 0 {
        piece
            .split_once(char::is_whitespace)
            .map_or(piece, |(_, r)| r)
    } else {
        piece
    };
    let redacted = redact(piece);
    let cleaned: String = redacted
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    Some(cleaned.trim_end().to_owned()).filter(|s| !s.is_empty())
}
/// The command line with secrets masked, then where it runs and which environment
/// variable names were set (never their values).
fn command(body: &Value) -> String {
    let program = body["program"].as_str().unwrap_or("");
    let args: Vec<&str> = body["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str())
        .collect();
    let mut out = vec![quote(program)];
    let mut mask_next = false;
    for arg in args {
        if mask_next {
            out.push("***".into());
            mask_next = false;
            continue;
        }
        if arg.starts_with('-') && !arg.contains('=') && secret_name(arg) {
            mask_next = true;
        }
        out.push(quote(&redact(arg)));
    }
    let mut summary = out.join(" ");
    if let Some(cwd) = body["cwd"].as_str().filter(|c| !c.is_empty()) {
        summary.push_str(&format!("  (cwd {})", redact(cwd)));
    }
    if let Some(env) = body["env"].as_object().filter(|e| !e.is_empty()) {
        let names: Vec<_> = env.keys().map(|k| word(k)).collect();
        summary.push_str(&format!("  (env {})", names.join(", ")));
    }
    summary
}
fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "pass",
        "pwd",
        "token",
        "secret",
        "key",
        "auth",
        "credential",
        "cookie",
        "session",
        "signature",
        "private",
    ]
    .iter()
    .any(|w| name.contains(w))
}
const PREFIXES: [&str; 12] = [
    "sk-",
    "sk_",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "akia",
    "aiza",
    "eyj",
];
/// Masks credential-shaped text inside free text such as `bash -c "..."` scripts.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut mask_next = false;
    for (index, piece) in text.split_inclusive(char::is_whitespace).enumerate() {
        let token = piece.trim_end();
        let space = &piece[token.len()..];
        let lower = token.to_ascii_lowercase();
        let bare = lower.trim_matches(|c: char| "'\"".contains(c));
        let scheme = matches!(bare, "bearer" | "basic" | "token");
        if mask_next && !token.is_empty() && !scheme {
            mask_next = false;
            out.push_str("***");
            out.push_str(space);
            continue;
        }
        if scheme && index > 0 || bare.ends_with("authorization:") {
            mask_next = true;
        }
        if (bare.starts_with("--") || bare.starts_with('-'))
            && !bare.contains('=')
            && secret_name(bare)
        {
            mask_next = true;
        }
        out.push_str(&mask_token(token));
        out.push_str(space);
    }
    out
}
fn mask_token(token: &str) -> String {
    // `scheme://user:password@host` keeps only the host.
    if let Some(scheme) = token.find("://") {
        let rest = &token[scheme + 3..];
        if let Some(at) = rest.find('@')
            && !rest[..at].contains('/')
        {
            return format!("{}://***@{}", &token[..scheme], mask_token(&rest[at + 1..]));
        }
    }
    // `NAME=value` / `--name=value` / `name:value` with a secret-looking name.
    for separator in ['=', ':'] {
        if let Some(position) = token.find(separator) {
            let (name, value) = (&token[..position], &token[position + 1..]);
            if !value.is_empty() && !value.starts_with("//") && secret_name(name) {
                return format!("{name}{separator}***");
            }
        }
    }
    let lower = token
        .trim_matches(|c: char| "'\"".contains(c))
        .to_ascii_lowercase();
    if PREFIXES
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() >= p.len() + 12)
    {
        return "***".into();
    }
    if opaque(token) {
        return "***".into();
    }
    token.to_owned()
}
/// A long run of key-like characters mixing letters and digits (API keys, hex secrets,
/// base64 blobs). Paths and ordinary words are left alone.
fn opaque(token: &str) -> bool {
    let core = token.trim_matches(|c: char| "'\"()[]{},;".contains(c));
    core.len() >= 32
        && core
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+=_".contains(c))
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

/// Commands that destroy data, take over the machine, persist access or fetch and run
/// code. They always need a one-time approval on the target, even when Exec is set to
/// Allow. This is a speed bump for mistakes and obvious attacks, not a security boundary:
/// anyone allowed to run commands can disguise one.
pub fn dangerous_command(command: &str) -> bool {
    !script_findings(command).is_empty()
}
/// Rule findings for an exec request. Each is a stable code (`recursive_delete`, …) the
/// UI translates; any finding makes the operation confirm-every-time.
pub fn command_findings(program: &str, args: &[&str]) -> Vec<&'static str> {
    let mut argv = vec![program.to_owned()];
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    let mut found = Vec::new();
    argv_findings(&argv, &mut found, 0);
    found.sort_unstable();
    found.dedup();
    found
}
fn script_findings(script: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    script_into(script, &mut found, 0);
    found.sort_unstable();
    found.dedup();
    found
}
fn base(word: &str) -> String {
    let name = word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}
const SHELLS: [&str; 7] = ["sh", "bash", "zsh", "dash", "ksh", "fish", "busybox"];
const INTERPRETERS: [&str; 10] = [
    "sh",
    "bash",
    "zsh",
    "python",
    "python3",
    "node",
    "perl",
    "ruby",
    "pwsh",
    "powershell",
];
/// A shell script: split into command segments, each checked as its own argv. Quoting
/// is approximated; when in doubt a segment is checked anyway (more confirmations, never
/// fewer).
fn script_into(script: &str, found: &mut Vec<&'static str>, depth: usize) {
    if depth > 4 {
        found.push("obfuscated");
        return;
    }
    let lower = script.to_ascii_lowercase();
    let segments: Vec<Vec<String>> = lower
        .split(|c: char| ";|&\n`()".contains(c))
        .map(|segment| {
            segment
                .split(|c: char| c.is_whitespace())
                .map(|w| w.trim_matches(|c: char| "'\"$<>{}".contains(c)).to_owned())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect();
    let mut fetch = false;
    let mut runs = false;
    let mut decode = false;
    for words in &segments {
        let name = command_name(words).map(|i| base(&words[i]));
        decode |= name.as_deref().is_some_and(|n| {
            (n == "base64"
                && words
                    .iter()
                    .any(|w| w == "-d" || w == "--decode" || w == "-D"))
                || (n == "xxd" && words.iter().any(|w| w == "-r"))
        });
        fetch |= name
            .as_deref()
            .is_some_and(|n| matches!(n, "curl" | "wget" | "iwr" | "invoke-webrequest" | "irm"));
        runs |= name.as_deref().is_some_and(|n| INTERPRETERS.contains(&n));
        argv_findings(words, found, depth + 1);
    }
    if fetch && runs {
        found.push("remote_code");
    }
    // Decoding something and running it hides what runs.
    if decode && runs {
        found.push("obfuscated");
    }
    // `… > ~/.bashrc`, `>> authorized_keys`: redirection into login or key files.
    for (position, _) in lower.match_indices('>') {
        let target: String = lower[position..]
            .trim_start_matches('>')
            .trim_start()
            .split(|c: char| c.is_whitespace() || ";|&".contains(c))
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| "'\"".contains(c))
            .to_owned();
        if persistence_path(&target) {
            found.push("persistence");
        }
    }
}
/// Index of the actual command after environment assignments and wrappers.
fn command_name(words: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        let name = base(word);
        if word.contains('=') && !word.starts_with('-') && i == 0
            || (word.contains('=') && !word.starts_with('-') && i > 0 && {
                let prev = base(&words[i - 1]);
                prev == "env" || words[..i].iter().all(|w| w.contains('='))
            })
        {
            i += 1;
            continue;
        }
        match name.as_str() {
            "env" | "nohup" | "time" | "nice" | "ionice" | "exec" | "command" | "builtin"
            | "stdbuf" | "xargs" | "sudo" | "doas" | "pkexec" | "runas" | "caffeinate" => {
                i += 1;
                while i < words.len() && words[i].starts_with('-') {
                    // `sudo -u user`, `nice -n 5`, `xargs -I {}`: the option's value.
                    let takes = matches!(words[i].as_str(), "-u" | "-g" | "-n" | "-i" | "-p");
                    i += if takes { 2 } else { 1 };
                }
            }
            "timeout" => {
                i += 1;
                while i < words.len() && words[i].starts_with('-') {
                    i += 1;
                }
                i += 1; // duration
            }
            _ => return Some(i),
        }
    }
    None
}
fn argv_findings(argv: &[String], found: &mut Vec<&'static str>, depth: usize) {
    let words: Vec<String> = argv.iter().map(|w| w.to_ascii_lowercase()).collect();
    let Some(start) = command_name(&words) else {
        if words.iter().any(|w| is_privilege(&base(w))) {
            found.push("privilege");
        }
        return;
    };
    if words[..start].iter().any(|w| is_privilege(&base(w))) {
        found.push("privilege");
    }
    let name = base(&words[start]);
    let args = &argv[start + 1..];
    let lower = &words[start + 1..];
    let flag = |f: &str| lower.iter().any(|w| w == f);
    // Credential files are flagged whatever the command (reading them is exfiltration).
    if lower.iter().any(|w| credential_path(w)) {
        found.push("credential_access");
    }
    // Shells and interpreters: check the inline script as commands of its own.
    if SHELLS.contains(&name.as_str()) {
        if let Some(position) = lower
            .iter()
            .position(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('c'))
        {
            let script = args[position + 1..].join(" ");
            script_into(&script, found, depth);
        }
        return;
    }
    if name == "cmd" {
        if let Some(position) = lower.iter().position(|w| w == "/c" || w == "/k") {
            script_into(&args[position + 1..].join(" "), found, depth);
        }
        return;
    }
    if name == "powershell" || name == "pwsh" {
        if lower.iter().any(|w| w.starts_with("-e") && w.len() > 2) || flag("-e") {
            found.push("obfuscated");
        }
        if let Some(position) = lower.iter().position(|w| w == "-c" || w == "-command") {
            script_into(&args[position + 1..].join(" "), found, depth);
        }
        return;
    }
    if matches!(
        name.as_str(),
        "python" | "python3" | "node" | "perl" | "ruby" | "php" | "osascript"
    ) && lower.iter().any(|w| w == "-c" || w == "-e")
    {
        // Inline code: no parser, so look for the dangerous names anywhere in it.
        let code = args.join(" ");
        if inline_code_dangerous(&code) {
            found.push("inline_code");
        }
        return;
    }
    let recursive = lower.iter().any(|w| {
        (w.starts_with('-') && !w.starts_with("--") && w.contains('r'))
            || w == "--recursive"
            || w == "/s"
            || w.starts_with("-recurse")
    });
    match name.as_str() {
        "rm" if recursive => found.push("recursive_delete"),
        "rmdir" | "rd" | "del" | "erase" if flag("/s") => found.push("recursive_delete"),
        "remove-item" | "ri" if lower.iter().any(|w| w.starts_with("-recurse")) => {
            found.push("recursive_delete")
        }
        "find"
            if lower
                .iter()
                .any(|w| matches!(w.as_str(), "-delete" | "-exec" | "-execdir" | "-ok")) =>
        {
            if lower.iter().any(|w| w == "-delete") || lower.iter().any(|w| base(w) == "rm") {
                found.push("recursive_delete");
            }
        }
        "git" if flag("clean") && lower.iter().any(|w| w.starts_with('-') && w.contains('f')) => {
            found.push("recursive_delete")
        }
        "mkfs" | "fdisk" | "sfdisk" | "parted" | "wipefs" | "shred" | "diskpart" | "diskutil"
        | "format" => found.push("disk"),
        n if n.starts_with("mkfs.") => found.push("disk"),
        "dd" if lower.iter().any(|w| w.starts_with("of=")) => found.push("disk"),
        "shutdown" | "reboot" | "poweroff" | "halt" | "stop-computer" | "restart-computer" => {
            found.push("power")
        }
        "su" | "visudo" => found.push("privilege"),
        "passwd" | "useradd" | "usermod" | "userdel" | "chpasswd" => found.push("account"),
        "net" if flag("user") || flag("localgroup") => found.push("account"),
        "crontab" | "launchctl" | "schtasks" | "at" => found.push("persistence"),
        "systemctl"
            if lower
                .iter()
                .any(|w| matches!(w.as_str(), "enable" | "disable" | "mask" | "stop")) =>
        {
            found.push("service")
        }
        "reg" if flag("add") || flag("delete") => found.push("registry"),
        "chmod" | "chown" | "icacls" if recursive => found.push("permissions"),
        "docker" | "podman"
            if flag("prune") || flag("rm") || flag("rmi") || (flag("volume") && flag("rm")) =>
        {
            found.push("container_delete")
        }
        "killall" | "pkill" | "taskkill" => found.push("kill"),
        "invoke-expression" | "iex" | "eval" => found.push("eval"),
        _ => {}
    }
    // Copies, links or edits into files that grant access or run at login.
    if matches!(
        name.as_str(),
        "tee"
            | "cp"
            | "mv"
            | "install"
            | "ln"
            | "sed"
            | "rsync"
            | "scp"
            | "truncate"
            | "touch"
            | "copy"
            | "move"
            | "copy-item"
            | "set-content"
            | "add-content"
            | "out-file"
    ) && lower.iter().any(|w| persistence_path(w))
    {
        found.push("persistence");
    }
}
fn is_privilege(name: &str) -> bool {
    matches!(name, "sudo" | "doas" | "pkexec" | "runas" | "su")
}
fn inline_code_dangerous(code: &str) -> bool {
    let code = code.to_ascii_lowercase();
    [
        "rmtree",
        "rm -rf",
        "rm -fr",
        "os.remove",
        "unlink",
        "rmsync",
        "rimraf",
        "sudo ",
        "authorized_keys",
        "/etc/",
        ".ssh",
        "mkfs",
        "format c:",
    ]
    .iter()
    .any(|p| code.contains(p))
        || !script_findings(&code).is_empty() && code.contains("system")
}
fn persistence_path(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    [
        "authorized_keys",
        ".bashrc",
        ".bash_profile",
        ".zshrc",
        ".zprofile",
        ".profile",
        "/etc/sudoers",
        "/etc/passwd",
        "/etc/shadow",
        "launchagents",
        "launchdaemons",
        "/etc/systemd",
        ".config/systemd",
        ".config/autostart",
        "/etc/cron",
        "/start menu/programs/startup",
    ]
    .iter()
    .any(|p| path.contains(p))
}
/// Files whose contents are credentials. Reading one, by command or file transfer,
/// always needs a one-time approval.
pub fn credential_path(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    [
        ".ssh/id_",
        ".ssh/identity",
        "/.ssh/",
        ".aws/credentials",
        ".gnupg",
        ".netrc",
        ".docker/config.json",
        ".kube/config",
        ".config/gh/hosts.yml",
        ".git-credentials",
        ".password-store",
        "/.cranibox",
        "/.config/cranibox",
        "/etc/shadow",
        "keychains/",
        "login data",
        "/cookies",
        "key4.db",
        "logins.json",
    ]
    .iter()
    .any(|p| path.contains(p))
        || path.ends_with("/.ssh")
}
/// Plain commands that only look: no shell, no redirection, nothing flagged. Logged as
/// read-only and not sent to the automatic reviewer.
pub fn read_only_command(program: &str, args: &[&str]) -> bool {
    if !command_findings(program, args).is_empty() {
        return false;
    }
    let name = base(program);
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    if lower
        .iter()
        .any(|a| a.contains('>') || a.contains('|') || a.contains('`') || a.contains("$("))
    {
        return false;
    }
    match name.as_str() {
        "ls" | "pwd" | "whoami" | "id" | "uname" | "hostname" | "date" | "df" | "du" | "free"
        | "uptime" | "ps" | "cat" | "head" | "tail" | "wc" | "stat" | "file" | "which"
        | "where" | "echo" | "grep" | "rg" | "tree" | "lsblk" | "nproc" | "sw_vers" | "dir"
        | "tasklist" | "realpath" | "readlink" | "basename" | "dirname" | "md5sum"
        | "sha256sum" | "shasum" | "less" | "true" => true,
        "find" => !lower.iter().any(|a| {
            matches!(
                a.as_str(),
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprintf"
                    | "-fls"
                    | "-fprint0"
            )
        }),
        "git" => lower
            .iter()
            .find(|a| !a.starts_with('-'))
            .is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "status"
                        | "log"
                        | "diff"
                        | "show"
                        | "rev-parse"
                        | "describe"
                        | "ls-files"
                        | "blame"
                        | "shortlog"
                )
            }),
        _ => false,
    }
}
/// Upload destinations that grant access or run code at login.
pub fn sensitive_path(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    [
        "/.ssh/",
        "/.bashrc",
        "/.bash_profile",
        "/.zshrc",
        "/.zprofile",
        "/.profile",
        "/library/launchagents/",
        "/library/launchdaemons/",
        "/.config/autostart/",
        "/.config/systemd/",
        "/etc/",
        "/start menu/programs/startup/",
        "/.gnupg/",
        "/.cranibox/",
        "/usr/local/bin/",
        "/usr/bin/",
    ]
    .iter()
    .any(|p| path.contains(p) || path.starts_with(&p[1..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn op(method: &str, route: &str, body: Value) -> Operation {
        describe(method, route, &BTreeMap::new(), &body).unwrap()
    }
    #[test]
    fn commands_are_summarised_without_secrets() {
        let o = op(
            "POST",
            "/nodes/{node}/exec",
            json!({"program":"curl","args":["-H","Authorization: Bearer abc.def.ghi","--token","hunter2","https://bob:pw@example.com/x?api_key=zzz","API_KEY=sk-1234567890abcdefghij"],"cwd":"/home/u","env":{"OPENAI_API_KEY":"sk-live-secret"}}),
        );
        assert_eq!(o.kind, "exec.run");
        assert_eq!(o.risk, Risk::Danger);
        for secret in [
            "abc.def.ghi",
            "hunter2",
            "bob:pw",
            "sk-1234567890",
            "sk-live-secret",
        ] {
            assert!(
                !o.summary.contains(secret),
                "{secret} leaked: {}",
                o.summary
            );
        }
        assert!(o.summary.contains("example.com"), "{}", o.summary);
        assert!(
            o.summary.contains("OPENAI_API_KEY"),
            "env names stay visible"
        );
        assert!(o.summary.contains("(cwd /home/u)"));
        let o = op(
            "POST",
            "/nodes/{node}/exec",
            json!({"program":"bash","args":["-c","export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789 && git push"]}),
        );
        assert!(!o.summary.contains("ghp_abc"), "{}", o.summary);
        assert!(o.summary.contains("git push"));
        let long = "x".repeat(1000);
        assert!(
            op(
                "POST",
                "/nodes/{node}/exec",
                json!({"program":"echo","args":[long]})
            )
            .summary
            .chars()
            .count()
                <= SUMMARY
        );
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(
            !op(
                "POST",
                "/nodes/{node}/exec",
                json!({"program":"echo","args":[hex]})
            )
            .summary
            .contains(hex)
        );
        assert!(
            op(
                "POST",
                "/nodes/{node}/exec",
                json!({"program":"ls","args":["/var/lib/docker/overlay2"]})
            )
            .summary
            .contains("/var/lib/docker/overlay2")
        );
    }
    #[test]
    fn files_messages_and_spaces_never_log_content() {
        let o = op(
            "POST",
            "/nodes/{node}/files/write",
            json!({"path":"/tmp/report.txt","offset":0,"data":"c2VjcmV0","size":6,"sha256":"00"}),
        );
        assert_eq!(
            (o.kind.as_str(), o.summary.as_str()),
            ("files.write", "/tmp/report.txt")
        );
        assert!(
            describe(
                "POST",
                "/nodes/{node}/files/write",
                &BTreeMap::new(),
                &json!({"path":"/tmp/a","offset":4096})
            )
            .is_none()
        );
        let o = op(
            "POST",
            "/nodes/{node}/messages",
            json!({"text":"my password is hunter2"}),
        );
        assert_eq!(
            (o.kind.as_str(), o.summary.as_str(), o.risk),
            ("messages.send", "", Risk::Change)
        );
        let o = op(
            "POST",
            "/nodes/{node}/spaces/control",
            json!({"id":"5f8c","action":"stop"}),
        );
        assert_eq!((o.kind.as_str(), o.risk), ("spaces.stop", Risk::Change));
        assert_eq!(
            op("GET", "/nodes/{node}/spaces", Value::Null).risk,
            Risk::Read
        );
        let o = op(
            "POST",
            "/nodes/{node}/computer",
            json!({"operation":"click; rm -rf"}),
        );
        assert_eq!(o.kind, "computer.clickrm_rf");
    }
    #[test]
    fn destructive_and_persistence_commands_always_need_confirmation() {
        let confirm = |program: &str, args: &[&str]| {
            op(
                "POST",
                "/nodes/{node}/exec",
                json!({"program":program,"args":args}),
            )
            .confirm
        };
        assert!(confirm("rm", &["-rf", "/home/u/project"]));
        assert!(confirm("/bin/bash", &["-lc", "cd /tmp && rm -fr build"]));
        assert!(confirm("sudo", &["apt-get", "install", "x"]));
        assert!(confirm("sh", &["-c", "curl -fsSL https://x.sh | sh"]));
        assert!(confirm(
            "bash",
            &["-c", "echo key >> ~/.ssh/authorized_keys"]
        ));
        assert!(confirm("cmd", &["/c", "rd", "/s", "/q", "C:\\Users"]));
        assert!(confirm("dd", &["if=/dev/zero", "of=/dev/sda"]));
        assert!(confirm("shutdown", &["-h", "now"]));
        assert!(!confirm("ls", &["-la"]));
        assert!(!confirm("git", &["status"]));
        assert!(!confirm("rm", &["build.log"]));
        assert!(!confirm("cargo", &["test", "--release"]));
        let write = |path: &str| {
            op(
                "POST",
                "/nodes/{node}/files/write",
                json!({"path":path,"offset":0}),
            )
            .confirm
        };
        assert!(write("/home/u/.ssh/authorized_keys"));
        assert!(write("/Users/u/Library/LaunchAgents/evil.plist"));
        assert!(write(
            "C:\\Users\\u\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\x.bat"
        ));
        assert!(!write("/home/u/Downloads/report.pdf"));
    }
    #[test]
    fn words_inside_arguments_are_not_commands() {
        let findings = |program: &str, args: &[&str]| command_findings(program, args);
        assert!(findings("git", &["commit", "-m", "sudo docs and rm -rf notes"]).is_empty());
        assert!(findings("grep", &["-r", "shutdown", "."]).is_empty());
        assert!(findings("echo", &["chmod 777 /"]).is_empty());
        assert_eq!(findings("sudo", &["-u", "root", "ls"]), ["privilege"]);
        assert!(findings("env", &["FOO=1", "rm", "-rf", "/srv/x"]).contains(&"recursive_delete"));
        assert!(
            findings("bash", &["-c", "git status; sudo reboot"]).contains(&"privilege"),
            "each segment of a shell script is checked"
        );
        assert!(findings("sh", &["-c", "echo aGkK | base64 -d | sh"]).contains(&"obfuscated"));
        assert!(findings("cat", &["/home/u/.ssh/id_ed25519"]).contains(&"credential_access"));
        assert!(findings("cat", &["/home/u/notes.txt"]).is_empty());
    }
    #[test]
    fn read_only_commands_are_reads_and_anything_else_is_danger() {
        let risk = |program: &str, args: &[&str]| {
            op(
                "POST",
                "/nodes/{node}/exec",
                json!({"program":program,"args":args}),
            )
            .risk
        };
        assert_eq!(risk("ls", &["-la"]), Risk::Read);
        assert_eq!(risk("git", &["status"]), Risk::Read);
        assert_eq!(risk("git", &["log", "--oneline", "-5"]), Risk::Read);
        assert_eq!(risk("find", &[".", "-name", "*.rs"]), Risk::Read);
        assert_eq!(risk("find", &[".", "-delete"]), Risk::Danger);
        assert_eq!(risk("git", &["push"]), Risk::Danger);
        assert_eq!(risk("cat", &["/home/u/.aws/credentials"]), Risk::Danger);
        assert_eq!(risk("sh", &["-c", "ls > out.txt"]), Risk::Danger);
        assert_eq!(risk("cargo", &["build"]), Risk::Danger);
    }
    #[test]
    fn reading_a_credential_file_needs_confirmation() {
        let read = |path: &str| {
            op(
                "POST",
                "/nodes/{node}/files/read",
                json!({"path":path,"offset":0}),
            )
            .confirm
        };
        assert!(read("/home/u/.ssh/id_rsa"));
        assert!(read("/home/u/.aws/credentials"));
        assert!(read("C:\\Users\\u\\.kube\\config"));
        assert!(!read("/home/u/project/README.md"));
    }
    #[test]
    fn details_keep_arguments_and_names_but_not_secrets() {
        let body = json!({
            "program": "/usr/bin/curl",
            "args": ["-H", "Authorization: Bearer sk-live-0123456789abcdef", "--password", "hunter2", "https://example.com"],
            "cwd": "/home/u/project",
            "env": {"API_KEY": "sk-secret-value", "LANG": "C"},
            "timeout_secs": 30
        });
        let d = detail("exec.run", &BTreeMap::new(), &body);
        assert_eq!(d.argv[0], "/usr/bin/curl");
        assert_eq!(d.argv[4], "***", "the value after a secret flag is masked");
        assert!(!d.argv.join(" ").contains("sk-live"));
        assert_eq!(d.cwd.as_deref(), Some("/home/u/project"));
        assert_eq!(d.env_keys, ["API_KEY", "LANG"]);
        assert_eq!(d.timeout_secs, Some(30));
        let rm = detail(
            "exec.run",
            &BTreeMap::new(),
            &json!({"program":"rm","args":["-rf","/home/u/project"]}),
        );
        assert!(!rm.findings.is_empty(), "matched rules are recorded");
        assert!(d.findings.is_empty());
        let text = serde_json::to_string(&d).unwrap();
        assert!(!text.contains("sk-secret") && !text.contains("hunter2"));
        let files = detail(
            "files.write",
            &BTreeMap::new(),
            &json!({"path":"/tmp/a.txt","size":42,"data":"c2VjcmV0"}),
        );
        assert_eq!(
            (files.paths, files.bytes),
            (vec!["/tmp/a.txt".to_owned()], Some(42))
        );
        let tail = tail(
            b"build ok\ntoken=ghp_0123456789abcdefghijklmnopqrstuvwxyz\ndone",
            400,
        )
        .unwrap();
        assert!(tail.contains("build ok") && tail.contains("done"));
        assert!(!tail.contains("ghp_0123456789"), "{tail}");
        let long = "x ".repeat(1000);
        assert!(super::tail(long.as_bytes(), 100).unwrap().chars().count() <= 100);
    }
}
