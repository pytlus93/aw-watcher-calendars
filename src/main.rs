use aw_calendar_sync::{adapter::synchronize, Config, Result};
use chrono::Utc;
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const HELP: &str =
    "aw-calendar-sync [--config PATH] [--testing] [--once | --dry-run | --init-config]\n\
Default: stay running, import completed events from the last 7 days every 60 minutes.\n\
--once         Run one import and exit (nonzero on failure).\n\
--dry-run      Download/parse/count eligible events, without connecting to AW.\n\
--init-config  Create a template if absent, print its path, and exit.\n\
--testing      Use port 5666 and a separate default config.\n\
Reconciliation replaces changed events and deletes excluded/missing events inside the window. Older history is preserved.";

fn default_config(testing: bool) -> Result<PathBuf> {
    let profile = std::env::var("AW_PROFILE").unwrap_or_default();
    if !profile
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("invalid AW_PROFILE".into());
    }
    let app = if profile.is_empty() || profile == "default" {
        "activitywatch".into()
    } else {
        format!("activitywatch-{profile}")
    };
    #[cfg(windows)]
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA missing")?)
        .join(&app)
        .join(&app);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join(&app);
    Ok(base.join("aw-calendar-sync").join(if testing {
        "config-testing.json"
    } else {
        "config.json"
    }))
}
fn init_config(path: &Path, testing: bool) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|_| "cannot create config directory")?;
    }
    let mut config = Config {
        calendar_url: Some(
            "https://outlook.office365.com/REPLACE_WITH_YOUR_PRIVATE_ICS_LINK/calendar.ics".into(),
        ),
        ..Config::default()
    };
    if testing {
        config.server_url = "http://127.0.0.1:5666".into();
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => f
            .write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())
            .map_err(|_| "cannot write config")?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("cannot create config file".into()),
    }
    Ok(())
}
fn read_config(path: &Path, testing: bool) -> Result<Config> {
    let text = fs::read_to_string(path).map_err(|_| "cannot read config file")?;
    // Serde errors can contain the private URL; report location only.
    let mut c: Config = serde_json::from_str(&text).map_err(|e| {
        format!(
            "invalid config JSON at line {}, column {}",
            e.line(),
            e.column()
        )
    })?;
    if testing {
        c.server_url = "http://127.0.0.1:5666".into();
    }
    if c.calendar_url
        .as_ref()
        .is_some_and(|u| u.contains("REPLACE_WITH_YOUR_PRIVATE_ICS_LINK"))
    {
        return Err("paste your Outlook ICS URL into calendar_url in the config file".into());
    }
    if let Some(file) = &mut c.calendar_file {
        let p = PathBuf::from(&*file);
        if p.is_relative() {
            *file = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(p)
                .to_string_lossy()
                .into();
        }
    }
    c.validate()?;
    Ok(c)
}
fn log(path: &Path, message: &str) {
    let line = format!("{} {}\n", Utc::now().to_rfc3339(), message);
    eprint!("{line}");
    // Fixed-size rotation; contains counts/errors only, never calendar content/URLs.
    if fs::metadata(path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}
fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut testing = false;
    let mut once = false;
    let mut dry = false;
    let mut init = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--version" => {
                println!("aw-calendar-sync {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config" => path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?)),
            "--testing" => testing = true,
            "--once" => once = true,
            "--dry-run" => {
                dry = true;
                once = true;
            }
            "--init-config" => init = true,
            _ => return Err("unknown argument; use --help".into()),
        }
    }
    let custom_config = path.is_some();
    let path = path.unwrap_or(default_config(testing)?);
    init_config(&path, testing)?;
    if init {
        println!("{}", path.display());
        return Ok(());
    }
    let logpath = if custom_config {
        path.with_extension("log")
    } else {
        let root = path
            .parent()
            .and_then(Path::parent)
            .ok_or("invalid config location")?;
        let dir = root.join("Logs").join("aw-calendar-sync");
        fs::create_dir_all(&dir).map_err(|_| "cannot create log directory")?;
        dir.join(if testing {
            "aw-calendar-sync-testing.log"
        } else {
            "aw-calendar-sync.log"
        })
    };
    let lockpath = path.with_extension("lock");
    let lock: File = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lockpath)
        .map_err(|_| "cannot open instance lock")?;
    lock.try_lock_exclusive()
        .map_err(|_| "another instance uses this configuration")?;
    log(
        &logpath,
        "module started; completed events only; bounded reconciliation configured by reconcile",
    );
    loop {
        let config = read_config(&path, testing);
        let wait = config.as_ref().map(|c| c.poll_seconds).unwrap_or(60);
        let outcome = config.and_then(|c| synchronize(&c, c.window(Utc::now())?, dry));
        match outcome {
            Ok(r) => log(
                &logpath,
                &format!(
                    "{}eligible={} inserted={} unchanged={} changed={} deleted={}",
                    if dry { "dry-run " } else { "" },
                    r.eligible,
                    r.inserted,
                    r.unchanged,
                    r.changed,
                    r.deleted
                ),
            ),
            Err(e) => {
                log(&logpath, &e);
                if once {
                    return Err("import failed; see log".into());
                }
            }
        }
        if once {
            break;
        }
        thread::sleep(Duration::from_secs(wait));
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
