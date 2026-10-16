//! prog-scanner: a strictly READ-ONLY installed-program lister.
//!
//! Safety design:
//!  * Registry is only opened with KEY_READ and only enumerated/read.
//!  * Folders are only listed with `read_dir`. Nothing is deleted, moved,
//!    renamed, created, or modified.
//!  * The only external commands run are read-only "list" queries
//!    (winget list, pip list, npm ls -g, cargo install --list). Skip them
//!    with --no-commands.
//!  * The ONLY thing that can ever be written is the optional report file
//!    from --save, and that refuses to overwrite an existing file.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const HELP: &str = "\
prog-scanner - read-only installed program lister (changes NOTHING)

USAGE:
    prog-scanner [OPTIONS]

OPTIONS:
    --no-commands     Do not run winget/pip/npm/cargo list queries
    --path            Also list executables found in the folders on PATH
    --save <FILE>     Also write the report to a NEW text file
                      (refuses to overwrite an existing file)
    --pause           Wait for Enter before exiting (handy if double-clicked)
    -h, --help        Show this help
    -V, --version     Show version
";

struct Section {
    title: String,
    lines: Vec<String>,
}

#[derive(Clone)]
struct Entry {
    name: String,
    version: String,
    publisher: String,
    location: String,
}

struct Options {
    commands: bool,
    path_scan: bool,
    save: Option<PathBuf>,
    pause: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options { commands: true, path_scan: false, save: None, pause: false };
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-commands" => o.commands = false,
            "--path" => o.path_scan = true,
            "--pause" => o.pause = true,
            "--save" => {
                let p = args.next().ok_or("--save needs a file name")?;
                o.save = Some(PathBuf::from(p));
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("prog-scanner {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown option: {other}\n\n{HELP}")),
        }
    }
    Ok(o)
}

// ---------- helpers ----------

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}~")
    }
}

fn format_entries(mut entries: Vec<Entry>) -> Vec<String> {
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.dedup_by(|a, b| {
        a.name.eq_ignore_ascii_case(&b.name) && a.version == b.version
    });
    entries
        .iter()
        .map(|e| {
            let mut s = format!(
                "{:<48} {:<18} {:<28}",
                trunc(&e.name, 48),
                trunc(&e.version, 18),
                trunc(&e.publisher, 28)
            );
            if !e.location.is_empty() {
                s.push(' ');
                s.push_str(&e.location);
            }
            s.trim_end().to_string()
        })
        .collect()
}

fn header_line() -> String {
    format!("{:<48} {:<18} {:<28} {}", "NAME", "VERSION", "PUBLISHER", "LOCATION")
}

/// Lists immediate sub-folders of `dir` (read-only).
fn list_subdirs(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for item in rd.flatten() {
        if item.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            out.push(Entry {
                name: item.file_name().to_string_lossy().into_owned(),
                version: String::new(),
                publisher: String::new(),
                location: item.path().display().to_string(),
            });
        }
    }
    Ok(out)
}

fn dir_section(title: &str, dir: &Path) -> Option<Section> {
    if !dir.is_dir() {
        return None;
    }
    let lines = match list_subdirs(dir) {
        Ok(entries) => {
            let mut l = vec![header_line()];
            l.extend(format_entries(entries));
            l
        }
        Err(e) => vec![format!("(skipped: {e})")],
    };
    Some(Section { title: format!("{title}  [{}]", dir.display()), lines })
}

/// Runs a read-only query command with a timeout. Never gets any input.
fn run_query(prog: &str, args: &[&str], secs: u64) -> Result<String, String> {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "not installed / not on PATH".to_string())?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(buf) => {
            let _ = child.wait();
            Ok(String::from_utf8_lossy(&buf).into_owned())
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("timed out after {secs}s"))
        }
    }
}

fn command_section(title: &str, prog: &str, args: &[&str]) -> Section {
    eprintln!("  querying: {prog} ...");
    let lines = match run_query(prog, args, 90) {
        Ok(text) => {
            let l: Vec<String> = text
                .lines()
                .map(|l| l.trim_end().to_string())
                // drop progress-spinner noise like "-", "\", "|", "/"
                .filter(|l| !l.trim().is_empty() && !matches!(l.trim(), "-" | "\\" | "|" | "/"))
                .collect();
            if l.is_empty() { vec!["(nothing listed)".to_string()] } else { l }
        }
        Err(e) => vec![format!("(skipped: {e})")],
    };
    Section { title: title.to_string(), lines }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

// ---------- Windows registry ----------

#[cfg(windows)]
fn registry_sections() -> Vec<Section> {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut programs: Vec<Entry> = Vec::new();
    let sources = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    for (hive, path) in sources {
        let root = RegKey::predef(hive);
        let Ok(key) = root.open_subkey_with_flags(path, KEY_READ) else { continue };
        for sub in key.enum_keys().flatten() {
            let Ok(k) = key.open_subkey_with_flags(&sub, KEY_READ) else { continue };
            let Ok(name) = k.get_value::<String, _>("DisplayName") else { continue };
            if name.trim().is_empty() {
                continue;
            }
            let get = |v: &str| k.get_value::<String, _>(v).unwrap_or_default();
            programs.push(Entry {
                name: name.trim().to_string(),
                version: get("DisplayVersion"),
                publisher: get("Publisher"),
                location: get("InstallLocation"),
            });
        }
    }
    let mut lines = vec![header_line()];
    lines.extend(format_entries(programs));

    let mut sections = vec![Section {
        title: "Installed programs (Windows registry: Apps & Features list)".into(),
        lines,
    }];

    // Microsoft Store (AppX/MSIX) packages - registry, read-only.
    let pk = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let mut store: Vec<Entry> = Vec::new();
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(pk, KEY_READ) {
        for sub in key.enum_keys().flatten() {
            let mut parts = sub.split('_');
            let name = parts.next().unwrap_or("").to_string();
            let version = parts.next().unwrap_or("").to_string();
            let arch = parts.next().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            store.push(Entry { name, version, publisher: arch, location: String::new() });
        }
    }
    let mut l = vec![header_line().replace("PUBLISHER", "ARCH")];
    l.extend(format_entries(store));
    sections.push(Section { title: "Microsoft Store / MSIX packages (registry)".into(), lines: l });
    sections
}

#[cfg(not(windows))]
fn registry_sections() -> Vec<Section> {
    vec![Section {
        title: "Installed programs (Windows registry)".into(),
        lines: vec!["(skipped: not running on Windows)".into()],
    }]
}

// ---------- scan ----------

fn scan(o: &Options) -> Vec<Section> {
    let mut out = registry_sections();

    // Program folders on the C: drive (taken from environment so it is correct
    // even on unusual setups; falls back to the usual C: paths).
    let pf = env_path("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let pf86 = env_path("ProgramFiles(x86)").unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let mut seen = BTreeSet::new();
    for (t, p) in [("Program Files folders", pf), ("Program Files (x86) folders", pf86)] {
        if seen.insert(p.to_string_lossy().to_lowercase()) {
            if let Some(s) = dir_section(t, &p) { out.push(s); }
        }
    }
    if let Some(la) = env_path("LOCALAPPDATA") {
        if let Some(s) = dir_section("Per-user installs (AppData\\Local\\Programs)", &la.join("Programs")) {
            out.push(s);
        }
    }

    // Package managers (folder based, read-only)
    let choco = env_path("ChocolateyInstall")
        .or_else(|| env_path("ProgramData").map(|p| p.join("chocolatey")));
    if let Some(c) = choco {
        if let Some(s) = dir_section("Chocolatey packages", &c.join("lib")) { out.push(s); }
    }
    let scoop = env_path("SCOOP")
        .or_else(|| env_path("USERPROFILE").map(|p| p.join("scoop")));
    if let Some(sc) = scoop {
        if let Some(s) = dir_section("Scoop apps (user)", &sc.join("apps")) { out.push(s); }
    }
    if let Some(g) = env_path("SCOOP_GLOBAL")
        .or_else(|| env_path("ProgramData").map(|p| p.join("scoop")))
    {
        if let Some(s) = dir_section("Scoop apps (global)", &g.join("apps")) { out.push(s); }
    }

    // Terminal tools (read-only list queries)
    if o.commands {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        out.push(command_section("winget list (terminal)", "winget", &["list", "--disable-interactivity"]));
        out.push(command_section("Python packages (pip list)", "pip", &["list", "--disable-pip-version-check"]));
        out.push(command_section("Global npm packages", npm, &["ls", "-g", "--depth=0"]));
        out.push(command_section("Cargo-installed programs", "cargo", &["install", "--list"]));
    }

    if o.path_scan {
        let exts = ["exe", "com", "bat", "cmd", "ps1"];
        let mut found = BTreeSet::new();
        if let Some(p) = env::var_os("PATH") {
            for dir in env::split_paths(&p) {
                if let Ok(rd) = fs::read_dir(&dir) {
                    for f in rd.flatten() {
                        let path = f.path();
                        let ok = path.extension().and_then(|e| e.to_str())
                            .map(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
                            .unwrap_or(false);
                        if ok {
                            found.insert(format!("{:<40} {}", f.file_name().to_string_lossy(), dir.display()));
                        }
                    }
                }
            }
        }
        out.push(Section {
            title: "Executables on PATH".into(),
            lines: found.into_iter().collect(),
        });
    }
    out
}

fn render(sections: &[Section]) -> String {
    let mut s = String::new();
    s.push_str("prog-scanner report (read-only scan)\n");
    for sec in sections {
        let count = sec.lines.len();
        s.push_str(&format!("\n=== {}  ({} lines) ===\n", sec.title, count));
        for l in &sec.lines {
            s.push_str(l);
            s.push('\n');
        }
    }
    s
}

fn wait_enter() {
    print!("\nPress Enter to close this window...");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
}

/// True when the program owns its console window, i.e. it was started by
/// double-clicking the .exe (the window would vanish the moment we finish).
#[cfg(windows)]
fn launched_by_double_click() -> bool {
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    }
    let mut buf = [0u32; 4];
    let n = unsafe { GetConsoleProcessList(buf.as_mut_ptr(), buf.len() as u32) };
    n == 1
}

#[cfg(not(windows))]
fn launched_by_double_click() -> bool {
    false
}

/// (year, month, day, hh, mm, ss) in UTC from the system clock.
fn now_parts() -> (i64, u32, u32, u64, u64, u64) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // civil-from-days algorithm
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Default report location: Desktop if it exists, else next to the .exe,
/// else the current folder. The name is timestamped so it is always new.
fn default_report_path() -> PathBuf {
    let (y, mo, d, h, mi, se) = now_parts();
    let name = format!("prog-scanner-report-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{se:02}.txt");
    let desktop = env_path("USERPROFILE").map(|p| p.join("Desktop")).filter(|p| p.is_dir());
    let exe_dir = env::current_exe().ok().and_then(|p| p.parent().map(|x| x.to_path_buf()));
    desktop.or(exe_dir).unwrap_or_else(|| PathBuf::from(".")).join(name)
}

fn run(opts: &Options, auto_save: bool) {
    eprintln!("Scanning (read-only)... please wait, this can take a minute.");
    let report = render(&scan(opts));
    print!("{report}");

    let target = match (&opts.save, auto_save) {
        (Some(p), _) => Some(p.clone()),
        (None, true) => Some(default_report_path()),
        _ => None,
    };
    if let Some(path) = target {
        // create_new => fails instead of overwriting anything that already exists.
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => match f.write_all(report.as_bytes()) {
                Ok(_) => {
                    let full = fs::canonicalize(&path).unwrap_or(path.clone());
                    println!("\n>>> Report saved to: {}", full.display());
                }
                Err(e) => println!("\nCould not write report: {e}"),
            },
            Err(e) => println!(
                "\nNot saved: {} ({e}). Existing files are never overwritten - pick a new name.",
                path.display()
            ),
        }
    }
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let double_click = launched_by_double_click();
    let pause = opts.pause || double_click;

    // catch_unwind so that even an unexpected error leaves the window open.
    let result = std::panic::catch_unwind(|| run(&opts, double_click));
    if result.is_err() {
        println!("\nAn unexpected error occurred. Nothing on your PC was changed.");
    }
    if pause {
        wait_enter();
    }
}
