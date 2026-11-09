//! `lain service install|uninstall|start|stop|status|logs` (D-164),
//! wrapping each platform's own manager with nothing but its CLI:
//!
//! - **Linux:** a systemd *user* unit in `~/.config/systemd/user/`
//!   (`loginctl enable-linger` with `--linger` for headless boxes).
//!   `--system` writes `/etc/systemd/system/` instead (run as root).
//! - **macOS:** a LaunchAgent plist in `~/Library/LaunchAgents/`;
//!   `--system` writes a LaunchDaemon in `/Library/LaunchDaemons/`.
//! - **Windows:** a per-user *logon* task (Task Scheduler), not a
//!   LocalSystem service: a service runs in session 0 and cannot see
//!   the user's mapped drives (the owner's media is on `V:`) or
//!   per-user paths. `--system` registers a boot task running as
//!   SYSTEM for headless machines, with that same drive caveat; a real
//!   SCM service needs the service-control dispatcher and is a later
//!   step.
//!
//! Everything is planned first ([`plan`]: files to write, commands to
//! run), so each platform's artifacts and commands are unit-tested on
//! any host and `--dry-run` prints them.

use std::path::{Path, PathBuf};

use crate::datadir::Os;

/// What to install.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Unit / agent / task name.
    pub name: String,
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub bind: String,
    pub system: bool,
    /// Run `loginctl enable-linger` (Linux user units).
    pub linger: bool,
}

impl Spec {
    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("logs").join(format!("{}.log", self.name))
    }

    fn serve_args(&self, with_log_file: bool) -> Vec<String> {
        let mut a = vec![
            "serve".to_string(),
            "--data-dir".into(),
            self.data_dir.to_string_lossy().into_owned(),
            "--port".into(),
            self.port.to_string(),
            "--bind".into(),
            self.bind.clone(),
        ];
        if with_log_file {
            a.push("--log-file".into());
            a.push(self.log_file().to_string_lossy().into_owned());
        }
        a
    }
}

/// A service subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
    Logs { follow: bool, lines: usize },
}

impl Action {
    pub fn parse(s: &str, follow: bool, lines: usize) -> Option<Action> {
        Some(match s {
            "install" => Action::Install,
            "uninstall" => Action::Uninstall,
            "start" => Action::Start,
            "stop" => Action::Stop,
            "status" => Action::Status,
            "logs" => Action::Logs { follow, lines },
            _ => return None,
        })
    }
}

/// One command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    /// A failure is reported but does not stop the plan (stop before
    /// uninstall; `status` of a stopped unit exits non-zero).
    pub allow_fail: bool,
}

/// What an action does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub write: Vec<(PathBuf, Vec<u8>)>,
    pub mkdir: Vec<PathBuf>,
    pub remove: Vec<PathBuf>,
    pub commands: Vec<Cmd>,
    /// Run after `remove` (a daemon-reload once the unit is gone).
    pub post: Vec<Cmd>,
    /// Print this file's tail (logs where the manager has no journal).
    pub tail: Option<(PathBuf, usize, bool)>,
    pub notes: Vec<String>,
}

fn cmd(argv: &[&str]) -> Cmd {
    Cmd { argv: argv.iter().map(|s| s.to_string()).collect(), allow_fail: false }
}

fn lenient(argv: &[&str]) -> Cmd {
    Cmd { allow_fail: true, ..cmd(argv) }
}

/// The host facts a plan needs.
#[derive(Debug, Clone, Default)]
pub struct Host {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` when set.
    pub xdg_config: Option<PathBuf>,
    pub user: String,
    /// Numeric uid (macOS launchctl domains).
    pub uid: String,
    /// `DOMAIN\user` (Windows logon trigger).
    pub windows_user: String,
}

impl Host {
    pub fn detect() -> Host {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        let user = env("USER").or_else(|| env("USERNAME")).unwrap_or_default();
        let uid = if cfg!(unix) {
            std::process::Command::new("id")
                .arg("-u")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let windows_user = match (env("USERDOMAIN"), env("USERNAME")) {
            (Some(d), Some(u)) => format!("{d}\\{u}"),
            (None, Some(u)) => u,
            _ => String::new(),
        };
        Host { home, xdg_config: env("XDG_CONFIG_HOME").map(PathBuf::from), user, uid, windows_user }
    }
}

// ---- artifacts -------------------------------------------------------------

/// Quotes one systemd `ExecStart` word: double quotes, with `\`, `"`
/// escaped and `%` doubled (unit specifiers).
fn systemd_word(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    if escaped.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\' || c == ';') || escaped.is_empty() {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// The systemd unit.
pub fn systemd_unit(spec: &Spec, host: &Host) -> String {
    let mut exec = vec![systemd_word(&spec.exe.to_string_lossy())];
    exec.extend(spec.serve_args(false).iter().map(|a| systemd_word(a)));
    let home = format!("LAIN_HOME={}", spec.data_dir.to_string_lossy());
    let mut s = String::new();
    s.push_str("[Unit]\n");
    s.push_str("Description=Lain media server\n");
    s.push_str("Documentation=https://github.com/enrell/lain\n");
    s.push_str("After=network-online.target\n");
    s.push_str("Wants=network-online.target\n\n");
    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    s.push_str(&format!("ExecStart={}\n", exec.join(" ")));
    s.push_str(&format!("Environment={}\n", systemd_word(&home)));
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartSec=3\n");
    s.push_str("TimeoutStopSec=20\n");
    if spec.system && !host.user.is_empty() && host.user != "root" {
        s.push_str(&format!("User={}\n", host.user));
    }
    s.push_str("\n[Install]\n");
    s.push_str(if spec.system { "WantedBy=multi-user.target\n" } else { "WantedBy=default.target\n" });
    s
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn launchd_label(spec: &Spec) -> String {
    format!("dev.lain.{}", spec.name)
}

/// The LaunchAgent / LaunchDaemon plist.
pub fn launchd_plist(spec: &Spec) -> String {
    let mut args = vec![spec.exe.to_string_lossy().into_owned()];
    args.extend(spec.serve_args(false));
    let log = xml(&spec.log_file().to_string_lossy());
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    s.push_str("<plist version=\"1.0\">\n<dict>\n");
    s.push_str(&format!("  <key>Label</key>\n  <string>{}</string>\n", xml(&launchd_label(spec))));
    s.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in &args {
        s.push_str(&format!("    <string>{}</string>\n", xml(a)));
    }
    s.push_str("  </array>\n");
    s.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
    s.push_str(&format!("    <key>LAIN_HOME</key>\n    <string>{}</string>\n", xml(&spec.data_dir.to_string_lossy())));
    s.push_str("  </dict>\n");
    s.push_str(&format!(
        "  <key>WorkingDirectory</key>\n  <string>{}</string>\n",
        xml(&spec.data_dir.to_string_lossy())
    ));
    s.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    s.push_str("  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n");
    s.push_str("  <key>ThrottleInterval</key>\n  <integer>5</integer>\n");
    s.push_str(&format!("  <key>StandardOutPath</key>\n  <string>{log}</string>\n"));
    s.push_str(&format!("  <key>StandardErrorPath</key>\n  <string>{log}</string>\n"));
    s.push_str("</dict>\n</plist>\n");
    s
}

/// Quotes one Windows command-line argument (CommandLineToArgvW rules).
fn win_arg(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The Task Scheduler definition (schema 1.2, Windows 7+).
pub fn task_xml(spec: &Spec, host: &Host) -> String {
    let args: Vec<String> = spec.serve_args(true).iter().map(|a| win_arg(a)).collect();
    let (trigger, principal) = if spec.system {
        (
            "    <BootTrigger>\n      <Enabled>true</Enabled>\n    </BootTrigger>\n".to_string(),
            "    <Principal id=\"Author\">\n      <UserId>S-1-5-18</UserId>\n      <RunLevel>HighestAvailable</RunLevel>\n    </Principal>\n".to_string(),
        )
    } else {
        (
            format!(
                "    <LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>{}</UserId>\n    </LogonTrigger>\n",
                xml(&host.windows_user)
            ),
            format!(
                "    <Principal id=\"Author\">\n      <UserId>{}</UserId>\n      <LogonType>InteractiveToken</LogonType>\n      <RunLevel>LeastPrivilege</RunLevel>\n    </Principal>\n",
                xml(&host.windows_user)
            ),
        )
    };
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n");
    s.push_str("<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n");
    s.push_str("  <RegistrationInfo>\n    <Description>Lain media server</Description>\n    <URI>\\");
    s.push_str(&xml(&spec.name));
    s.push_str("</URI>\n  </RegistrationInfo>\n");
    s.push_str("  <Triggers>\n");
    s.push_str(&trigger);
    s.push_str("  </Triggers>\n  <Principals>\n");
    s.push_str(&principal);
    s.push_str("  </Principals>\n");
    s.push_str("  <Settings>\n");
    s.push_str("    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n");
    s.push_str("    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n");
    s.push_str("    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n");
    s.push_str("    <AllowHardTerminate>true</AllowHardTerminate>\n");
    s.push_str("    <StartWhenAvailable>true</StartWhenAvailable>\n");
    s.push_str("    <AllowStartOnDemand>true</AllowStartOnDemand>\n");
    s.push_str("    <Enabled>true</Enabled>\n");
    s.push_str("    <Hidden>true</Hidden>\n");
    s.push_str("    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n");
    s.push_str(
        "    <RestartOnFailure>\n      <Interval>PT1M</Interval>\n      <Count>999</Count>\n    </RestartOnFailure>\n",
    );
    s.push_str("  </Settings>\n");
    s.push_str("  <Actions Context=\"Author\">\n    <Exec>\n");
    s.push_str(&format!("      <Command>{}</Command>\n", xml(&spec.exe.to_string_lossy())));
    s.push_str(&format!("      <Arguments>{}</Arguments>\n", xml(&args.join(" "))));
    s.push_str(&format!("      <WorkingDirectory>{}</WorkingDirectory>\n", xml(&spec.data_dir.to_string_lossy())));
    s.push_str("    </Exec>\n  </Actions>\n</Task>\n");
    s
}

/// UTF-16LE with a BOM: what `schtasks /XML` reliably accepts.
pub fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.replace('\n', "\r\n").encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

// ---- plans -----------------------------------------------------------------

pub fn unit_path(spec: &Spec, host: &Host) -> PathBuf {
    if spec.system {
        PathBuf::from("/etc/systemd/system").join(format!("{}.service", spec.name))
    } else {
        host.xdg_config
            .clone()
            .unwrap_or_else(|| host.home.join(".config"))
            .join("systemd")
            .join("user")
            .join(format!("{}.service", spec.name))
    }
}

pub fn plist_path(spec: &Spec, host: &Host) -> PathBuf {
    let file = format!("{}.plist", launchd_label(spec));
    if spec.system {
        PathBuf::from("/Library/LaunchDaemons").join(file)
    } else {
        host.home.join("Library").join("LaunchAgents").join(file)
    }
}

pub fn task_file(spec: &Spec) -> PathBuf {
    spec.data_dir.join("service").join(format!("{}.xml", spec.name))
}

/// What `action` does on `os`.
pub fn plan(os: Os, action: &Action, spec: &Spec, host: &Host) -> Result<Plan, String> {
    if spec.name.is_empty() || spec.name.contains(['/', '\\', ' ']) {
        return Err("service name must be a plain word".into());
    }
    let mut p = Plan::default();
    match os {
        Os::Linux => {
            let unit = format!("{}.service", spec.name);
            let sc: Vec<&str> = if spec.system { vec!["systemctl"] } else { vec!["systemctl", "--user"] };
            let with =
                |extra: &[&str]| -> Vec<String> { sc.iter().chain(extra.iter()).map(|s| s.to_string()).collect() };
            match action {
                Action::Install => {
                    let path = unit_path(spec, host);
                    p.mkdir.push(path.parent().unwrap_or(Path::new("/")).to_path_buf());
                    p.mkdir.push(spec.data_dir.clone());
                    p.write.push((path, systemd_unit(spec, host).into_bytes()));
                    p.commands.push(Cmd { argv: with(&["daemon-reload"]), allow_fail: false });
                    p.commands.push(Cmd { argv: with(&["enable", &unit]), allow_fail: false });
                    if spec.linger && !spec.system {
                        p.commands.push(cmd(&["loginctl", "enable-linger", &host.user]));
                    }
                    p.notes.push(format!("installed {unit}; start it with `lain service start --name {}`", spec.name));
                    if !spec.system && !spec.linger {
                        p.notes.push("a user unit runs while you are logged in; for a headless box add --linger (loginctl enable-linger)".into());
                    }
                }
                Action::Uninstall => {
                    p.commands.push(Cmd { argv: with(&["disable", "--now", &unit]), allow_fail: true });
                    p.remove.push(unit_path(spec, host));
                    p.post.push(Cmd { argv: with(&["daemon-reload"]), allow_fail: false });
                }
                Action::Start => p.commands.push(Cmd { argv: with(&["start", &unit]), allow_fail: false }),
                Action::Stop => p.commands.push(Cmd { argv: with(&["stop", &unit]), allow_fail: false }),
                Action::Status => {
                    p.commands.push(Cmd { argv: with(&["status", "--no-pager", &unit]), allow_fail: true })
                }
                Action::Logs { follow, lines } => {
                    let mut argv = vec!["journalctl".to_string()];
                    if !spec.system {
                        argv.push("--user".into());
                    }
                    argv.extend(["-u".into(), unit.clone(), "-n".into(), lines.to_string(), "--no-pager".into()]);
                    if *follow {
                        argv.push("-f".into());
                    }
                    p.commands.push(Cmd { argv, allow_fail: true });
                }
            }
        }
        Os::MacOs => {
            let label = launchd_label(spec);
            let domain = if spec.system { "system".to_string() } else { format!("gui/{}", host.uid) };
            let target = format!("{domain}/{label}");
            let plist = plist_path(spec, host);
            match action {
                Action::Install => {
                    p.mkdir.push(plist.parent().unwrap_or(Path::new("/")).to_path_buf());
                    p.mkdir.push(spec.data_dir.join("logs"));
                    p.write.push((plist.clone(), launchd_plist(spec).into_bytes()));
                    // Re-installing replaces a loaded agent.
                    p.commands.push(lenient(&["launchctl", "bootout", &target]));
                    p.commands.push(cmd(&["launchctl", "bootstrap", &domain, &plist.to_string_lossy()]));
                    p.notes.push(format!("loaded {label}; it starts at login (RunAtLoad) and now"));
                }
                Action::Uninstall => {
                    p.commands.push(lenient(&["launchctl", "bootout", &target]));
                    p.remove.push(plist);
                }
                Action::Start => p.commands.push(cmd(&["launchctl", "kickstart", &target])),
                Action::Stop => p.commands.push(cmd(&["launchctl", "kill", "SIGTERM", &target])),
                Action::Status => p.commands.push(lenient(&["launchctl", "print", &target])),
                Action::Logs { follow, lines } => p.tail = Some((spec.log_file(), *lines, *follow)),
            }
        }
        Os::Windows => {
            let name = spec.name.as_str();
            match action {
                Action::Install => {
                    let file = task_file(spec);
                    p.mkdir.push(spec.data_dir.join("service"));
                    p.mkdir.push(spec.data_dir.join("logs"));
                    p.write.push((file.clone(), utf16le_bom(&task_xml(spec, host))));
                    p.commands.push(cmd(&["schtasks", "/Create", "/TN", name, "/XML", &file.to_string_lossy(), "/F"]));
                    if spec.system {
                        p.notes.push(
                            "--system: the task runs as SYSTEM at boot, in session 0, and cannot see mapped drives (V:); use UNC paths for libraries"
                                .into(),
                        );
                    } else {
                        p.notes.push(format!(
                            "the task starts at your logon; start it now with `lain service start --name {name}`"
                        ));
                    }
                }
                Action::Uninstall => {
                    p.commands.push(lenient(&["schtasks", "/End", "/TN", name]));
                    p.commands.push(cmd(&["schtasks", "/Delete", "/TN", name, "/F"]));
                    p.remove.push(task_file(spec));
                }
                Action::Start => p.commands.push(cmd(&["schtasks", "/Run", "/TN", name])),
                Action::Stop => p.commands.push(cmd(&["schtasks", "/End", "/TN", name])),
                Action::Status => p.commands.push(lenient(&["schtasks", "/Query", "/TN", name, "/V", "/FO", "LIST"])),
                Action::Logs { follow, lines } => p.tail = Some((spec.log_file(), *lines, *follow)),
            }
        }
    }
    Ok(p)
}

/// Prints a plan instead of running it.
pub fn describe(p: &Plan) -> String {
    let mut s = String::new();
    for d in &p.mkdir {
        s.push_str(&format!("mkdir -p {}\n", d.display()));
    }
    for (path, bytes) in &p.write {
        let text = if bytes.starts_with(&[0xFF, 0xFE]) {
            let u: Vec<u16> =
                bytes[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
            String::from_utf16_lossy(&u).replace("\r\n", "\n")
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        s.push_str(&format!("write {}:\n{text}\n", path.display()));
    }
    let line = |c: &Cmd| format!("{}{}\n", c.argv.join(" "), if c.allow_fail { "  (may fail)" } else { "" });
    for c in &p.commands {
        s.push_str(&line(c));
    }
    for r in &p.remove {
        s.push_str(&format!("rm {}\n", r.display()));
    }
    for c in &p.post {
        s.push_str(&line(c));
    }
    if let Some((f, n, follow)) = &p.tail {
        s.push_str(&format!("tail -n {n}{} {}\n", if *follow { " -f" } else { "" }, f.display()));
    }
    for n in &p.notes {
        s.push_str(&format!("# {n}\n"));
    }
    s
}

fn tail(path: &Path, lines: usize, follow: bool) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    f.read_to_string(&mut text).map_err(|e| e.to_string())?;
    let all: Vec<&str> = text.lines().collect();
    for l in &all[all.len().saturating_sub(lines)..] {
        println!("{l}");
    }
    if !follow {
        return Ok(());
    }
    let mut pos = f.stream_position().map_err(|e| e.to_string())?;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(pos);
        if len < pos {
            pos = 0; // rotated or truncated
        }
        if len > pos {
            f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
            let mut chunk = String::new();
            f.read_to_string(&mut chunk).map_err(|e| e.to_string())?;
            print!("{chunk}");
            pos = len;
        }
    }
}

/// Carries a plan out.
pub fn execute(p: &Plan) -> Result<(), String> {
    for d in &p.mkdir {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    for (path, bytes) in &p.write {
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    let run = |c: &Cmd| -> Result<(), String> {
        println!("+ {}", c.argv.join(" "));
        match std::process::Command::new(&c.argv[0]).args(&c.argv[1..]).status() {
            Ok(s) if s.success() => Ok(()),
            Ok(s) if c.allow_fail => {
                eprintln!("({} exited {s}; continuing)", c.argv[0]);
                Ok(())
            }
            Ok(s) => Err(format!("{} exited {s}", c.argv.join(" "))),
            Err(e) if c.allow_fail => {
                eprintln!("({}: {e}; continuing)", c.argv[0]);
                Ok(())
            }
            Err(e) => Err(format!("{}: {e}", c.argv[0])),
        }
    };
    for c in &p.commands {
        run(c)?;
    }
    for r in &p.remove {
        match std::fs::remove_file(r) {
            Ok(()) => println!("removed {}", r.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", r.display())),
        }
    }
    for c in &p.post {
        run(c)?;
    }
    if let Some((f, n, follow)) = &p.tail {
        tail(f, *n, *follow)?;
    }
    for n in &p.notes {
        println!("{n}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(system: bool) -> Spec {
        Spec {
            name: "lain".into(),
            exe: PathBuf::from("/opt/lain/bin/lain"),
            data_dir: PathBuf::from("/home/u/.local/share/lain"),
            port: 9360,
            bind: "127.0.0.1".into(),
            system,
            linger: false,
        }
    }

    fn host() -> Host {
        Host {
            home: PathBuf::from("/home/u"),
            xdg_config: None,
            user: "u".into(),
            uid: "501".into(),
            windows_user: "DESKTOP\\u".into(),
        }
    }

    #[test]
    fn systemd_user_unit() {
        let u = systemd_unit(&spec(false), &host());
        assert!(
            u.contains(
                "ExecStart=/opt/lain/bin/lain serve --data-dir /home/u/.local/share/lain --port 9360 --bind 127.0.0.1\n"
            ),
            "{u}"
        );
        assert!(u.contains("Environment=LAIN_HOME=/home/u/.local/share/lain\n"));
        assert!(u.contains("Restart=on-failure\n"));
        assert!(u.contains("WantedBy=default.target\n"));
        assert!(!u.contains("User="));
        let sys = systemd_unit(&spec(true), &host());
        assert!(sys.contains("User=u\n") && sys.contains("WantedBy=multi-user.target\n"));
        let mut odd = spec(false);
        odd.data_dir = PathBuf::from("/srv/my media/100%");
        let u = systemd_unit(&odd, &host());
        assert!(u.contains("--data-dir \"/srv/my media/100%%\""), "{u}");
        assert!(u.contains("Environment=\"LAIN_HOME=/srv/my media/100%%\""), "{u}");
    }

    #[test]
    fn linux_plans() {
        let h = host();
        let p = plan(Os::Linux, &Action::Install, &spec(false), &h).unwrap();
        assert_eq!(p.write[0].0, PathBuf::from("/home/u/.config/systemd/user/lain.service"));
        let argv: Vec<String> = p.commands.iter().map(|c| c.argv.join(" ")).collect();
        assert_eq!(argv, vec!["systemctl --user daemon-reload", "systemctl --user enable lain.service"]);
        let mut linger = spec(false);
        linger.linger = true;
        assert!(
            plan(Os::Linux, &Action::Install, &linger, &h)
                .unwrap()
                .commands
                .iter()
                .any(|c| c.argv == ["loginctl", "enable-linger", "u"])
        );
        let p = plan(Os::Linux, &Action::Install, &spec(true), &h).unwrap();
        assert_eq!(p.write[0].0, PathBuf::from("/etc/systemd/system/lain.service"));
        assert_eq!(p.commands[1].argv, ["systemctl", "enable", "lain.service"]);
        let p = plan(Os::Linux, &Action::Uninstall, &spec(false), &h).unwrap();
        assert!(
            p.commands[0].allow_fail
                && p.commands[0].argv == ["systemctl", "--user", "disable", "--now", "lain.service"]
        );
        assert_eq!(p.remove, vec![PathBuf::from("/home/u/.config/systemd/user/lain.service")]);
        assert_eq!(p.post[0].argv, ["systemctl", "--user", "daemon-reload"], "reload after the unit is gone");
        let p = plan(Os::Linux, &Action::Logs { follow: true, lines: 50 }, &spec(false), &h).unwrap();
        assert_eq!(p.commands[0].argv, ["journalctl", "--user", "-u", "lain.service", "-n", "50", "--no-pager", "-f"]);
        let x = Host { xdg_config: Some(PathBuf::from("/cfg")), ..host() };
        assert_eq!(unit_path(&spec(false), &x), PathBuf::from("/cfg/systemd/user/lain.service"));
        assert!(plan(Os::Linux, &Action::Start, &Spec { name: "a b".into(), ..spec(false) }, &h).is_err());
    }

    #[test]
    fn launchd_agent() {
        let p = launchd_plist(&spec(false));
        assert!(p.contains("<key>Label</key>\n  <string>dev.lain.lain</string>"));
        assert!(p.contains(
            "    <string>/opt/lain/bin/lain</string>\n    <string>serve</string>\n    <string>--data-dir</string>"
        ));
        assert!(p.contains("<key>LAIN_HOME</key>\n    <string>/home/u/.local/share/lain</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(p.contains("<key>SuccessfulExit</key>\n    <false/>"));
        assert!(p.contains(&format!("<string>{}</string>", spec(false).log_file().display())));
        let mut amp = spec(false);
        amp.data_dir = PathBuf::from("/a&b<c>");
        assert!(launchd_plist(&amp).contains("/a&amp;b&lt;c&gt;"));
        let h = host();
        let pl = plan(Os::MacOs, &Action::Install, &spec(false), &h).unwrap();
        assert_eq!(pl.write[0].0, PathBuf::from("/home/u/Library/LaunchAgents/dev.lain.lain.plist"));
        let plist = plist_path(&spec(false), &h).to_string_lossy().into_owned();
        assert_eq!(pl.commands[1].argv, ["launchctl", "bootstrap", "gui/501", plist.as_str()]);
        assert_eq!(
            plan(Os::MacOs, &Action::Stop, &spec(false), &h).unwrap().commands[0].argv,
            ["launchctl", "kill", "SIGTERM", "gui/501/dev.lain.lain"]
        );
        let sys = plan(Os::MacOs, &Action::Install, &spec(true), &h).unwrap();
        assert_eq!(sys.write[0].0, PathBuf::from("/Library/LaunchDaemons/dev.lain.lain.plist"));
        assert_eq!(sys.commands[1].argv[2], "system");
        assert!(plan(Os::MacOs, &Action::Logs { follow: false, lines: 10 }, &spec(false), &h).unwrap().tail.is_some());
    }

    #[test]
    fn windows_logon_task() {
        let mut s = spec(false);
        s.exe = PathBuf::from(r"C:\Users\u\bin\lain.exe");
        s.data_dir = PathBuf::from(r"C:\Users\u\AppData\Local\lain");
        let x = task_xml(&s, &host());
        assert!(x.contains("<LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>DESKTOP\\u</UserId>"), "{x}");
        assert!(x.contains("<LogonType>InteractiveToken</LogonType>\n      <RunLevel>LeastPrivilege</RunLevel>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(x.contains(r"<Command>C:\Users\u\bin\lain.exe</Command>"));
        assert!(
            x.contains(
                r"<Arguments>serve --data-dir C:\Users\u\AppData\Local\lain --port 9360 --bind 127.0.0.1 --log-file "
            ),
            "{x}"
        );
        assert!(!x.contains("S-1-5-18"));
        let sys = task_xml(&Spec { system: true, ..s.clone() }, &host());
        assert!(
            sys.contains("<BootTrigger>") && sys.contains("<UserId>S-1-5-18</UserId>") && !sys.contains("LogonTrigger")
        );
        let bytes = utf16le_bom(&x);
        assert_eq!(&bytes[..4], &[0xFF, 0xFE, b'<', 0]);
        let p = plan(Os::Windows, &Action::Install, &s, &host()).unwrap();
        assert_eq!(p.commands[0].argv[..4], ["schtasks", "/Create", "/TN", "lain"]);
        assert_eq!(p.commands[0].argv[6], "/F");
        let u = plan(Os::Windows, &Action::Uninstall, &s, &host()).unwrap();
        assert!(u.commands[0].allow_fail && u.commands[1].argv == ["schtasks", "/Delete", "/TN", "lain", "/F"]);
        assert_eq!(
            plan(Os::Windows, &Action::Start, &s, &host()).unwrap().commands[0].argv,
            ["schtasks", "/Run", "/TN", "lain"]
        );
        assert!(
            plan(Os::Windows, &Action::Install, &Spec { system: true, ..s }, &host()).unwrap().notes[0]
                .contains("mapped drives")
        );
    }

    #[test]
    fn windows_argument_quoting() {
        assert_eq!(win_arg("plain"), "plain");
        assert_eq!(win_arg(r"C:\My Media\x"), r#""C:\My Media\x""#);
        assert_eq!(win_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(win_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(win_arg(""), r#""""#);
    }
}
