//! What a remote device asked this device to do, described by the target for its own audit
//! log and approval prompts. Summaries are redacted here, before they are stored or shown:
//! they never carry file contents, message bodies, environment values or credential-shaped
//! text.
use super::{Operation, Risk};
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
        ["nodes", _, "exec"] => ("exec.run".to_owned(), Danger, command(body)),
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
    let confirm = match kind.as_str() {
        "exec.run" => dangerous_command(&raw_command(body)),
        "files.write" => sensitive_path(body["path"].as_str().unwrap_or("")),
        _ => false,
    };
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
fn raw_command(body: &Value) -> String {
    let mut words = vec![body["program"].as_str().unwrap_or("").to_owned()];
    words.extend(
        body["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str())
            .map(str::to_owned),
    );
    words.join(" ")
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
    let text = command.to_ascii_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "'\"`;|&()".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    let base = |w: &str| w.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    let names: Vec<String> = words.iter().map(|w| base(w)).collect();
    let has = |name: &str| {
        names
            .iter()
            .any(|n| n == name || n == &format!("{name}.exe"))
    };
    let flag = |f: &str| words.contains(&f);
    let recursive = words.iter().any(|w| {
        (w.starts_with('-') && !w.starts_with("--") && w.contains('r'))
            || *w == "--recursive"
            || *w == "/s"
            || *w == "-recurse"
    });
    if has("rm") && recursive {
        return true;
    }
    if (has("rmdir") || has("rd") || has("del") || has("erase")) && flag("/s") {
        return true;
    }
    if has("remove-item") && words.iter().any(|w| w.starts_with("-recurse")) {
        return true;
    }
    for name in [
        "mkfs",
        "fdisk",
        "sfdisk",
        "parted",
        "wipefs",
        "shred",
        "diskpart",
        "diskutil",
        "format",
        "shutdown",
        "reboot",
        "poweroff",
        "halt",
        "sudo",
        "su",
        "doas",
        "runas",
        "pkexec",
        "crontab",
        "launchctl",
        "schtasks",
        "visudo",
        "passwd",
        "useradd",
        "usermod",
        "stop-computer",
        "restart-computer",
        "invoke-expression",
        "iex",
        "killall",
        "pkill",
    ] {
        if has(name) || names.iter().any(|n| n.starts_with("mkfs.")) {
            return true;
        }
    }
    if has("dd") && words.iter().any(|w| w.starts_with("of=")) {
        return true;
    }
    if (has("chmod") || has("chown")) && recursive {
        return true;
    }
    if has("systemctl")
        && words
            .iter()
            .any(|w| matches!(*w, "enable" | "disable" | "mask" | "stop"))
    {
        return true;
    }
    if has("reg") && words.iter().any(|w| matches!(*w, "add" | "delete")) {
        return true;
    }
    if has("docker")
        && (flag("prune") || flag("rm") || flag("rmi") || (flag("volume") && flag("rm")))
    {
        return true;
    }
    // Download-and-run: `curl … | sh`, `wget -O- … | bash`.
    let fetch = has("curl") || has("wget") || has("iwr") || has("invoke-webrequest");
    let shell = [
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
    if fetch && command.contains('|') && shell.iter().any(|s| has(s)) {
        return true;
    }
    // Writes into files that grant access or run at login.
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
        ".config/autostart",
        "/etc/cron",
        "\\startup\\",
    ]
    .iter()
    .any(|p| text.contains(p))
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
}
