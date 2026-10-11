use aw_calendar_sync::{adapter::synchronize_all, config::CalendarSet, Result};
mod actions;
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
--validate-config Check configuration without fetching calendars or connecting to AW.\n\
--testing      Use port 5666 and a separate default config.\n\
configure | status-ui | open-logs | about  Run a short-lived tray action.\n\
--headless     Print action reports without opening native UI.\n\
--server-url URL Override the loopback AW origin, including for actions.\n\
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
    let config = CalendarSet::template(testing);
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => f
            .write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())
            .map_err(|_| "cannot write config")?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("cannot create config file".into()),
    }
    Ok(())
}
fn read_config(path: &Path, testing: bool, server: Option<&str>) -> Result<CalendarSet> {
    let text = fs::read_to_string(path).map_err(|_| "cannot read config file")?;
    let mut settings = CalendarSet::from_json(&text)?;
    if testing {
        settings.server_url = "http://127.0.0.1:5666".into();
    }
    if let Some(server) = server {
        settings.server_url = server.into();
    }
    for c in &mut settings.calendars {
        if testing {
            c.server_url = "http://127.0.0.1:5666".into();
        }
        if let Some(server) = server {
            c.server_url = server.into();
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
    }
    Ok(settings)
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
    let mut validate = false;
    let mut action = None;
    let mut headless = false;
    let mut server = None;
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
            "--server-url" => server = Some(args.next().ok_or("--server-url needs a value")?),
            "--headless" => headless = true,
            "configure" | "status-ui" | "open-logs" | "about" => {
                if action.replace(arg).is_some() {
                    return Err("only one action is allowed".into());
                }
            }
            "--once" => once = true,
            "--dry-run" => {
                dry = true;
                once = true;
            }
            "--init-config" => init = true,
            "--validate-config" => validate = true,
            _ => return Err("unknown argument; use --help".into()),
        }
    }
    if let Ok(protocol) = std::env::var("AW_MODULE_ACTION_PROTOCOL") {
        if protocol != "1" {
            return Err("unsupported module action protocol".into());
        }
        let command = action
            .as_deref()
            .ok_or("action context requires an action command")?;
        let expected = actions::id(command).ok_or("unknown action")?;
        if std::env::var("AW_MODULE_ACTION").ok().as_deref() != Some(expected) {
            return Err("module action identity mismatch".into());
        }
        match std::env::var("AW_TESTING").as_deref() {
            Ok("1") => testing = true,
            Ok("0") => (),
            _ => return Err("AW_TESTING must be 0 or 1".into()),
        }
        if server.is_none() {
            server = std::env::var("AW_SERVER_URL").ok();
        }
    }
    if (headless && action.is_none()) || (action.is_some() && (once || dry || init || validate)) {
        return Err("action options cannot be combined with producer commands".into());
    }
    if let Some(server) = &server {
        let check = aw_calendar_sync::Config {
            calendar_file: Some("context-validation.ics".into()),
            server_url: server.clone(),
            ..Default::default()
        };
        check.validate()?;
    }
    let custom_config = path.is_some();
    let path = path.unwrap_or(default_config(testing)?);
    init_config(&path, testing)?;
    if let Some(command) = action {
        let result = actions::dispatch(
            &command,
            &path,
            testing,
            server.as_deref(),
            custom_config,
            headless,
        );
        actions::record_action(&path, testing, custom_config, &command, result.is_ok());
        return result;
    }
    if init {
        println!("{}", path.display());
        return Ok(());
    }
    if validate {
        let settings = read_config(&path, testing, server.as_deref())?;
        println!(
            "Configuration valid; {} calendars enabled",
            settings.calendars.len()
        );
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
        let config = read_config(&path, testing, server.as_deref());
        let mut wait = config.as_ref().map(|c| c.poll_seconds).unwrap_or(60);
        let mut failed = false;
        match config {
            Ok(settings) => {
                if settings.calendars.is_empty() {
                    log(&logpath, "no calendars enabled; retained buckets untouched");
                }
                for outcome in synchronize_all(&settings.calendars, Utc::now(), dry) {
                    match outcome.result {
                        Ok(r) => log(
                            &logpath,
                            &format!(
                    "calendar={} {}eligible={} inserted={} unchanged={} changed={} deleted={}",
                    outcome.calendar_id,
                    if dry { "dry-run " } else { "" },
                    r.eligible,
                    r.inserted,
                    r.unchanged,
                    r.changed,
                    r.deleted
                ),
                        ),
                        Err(e) => {
                            log(&logpath, &format!("calendar={} {e}", outcome.calendar_id));
                            failed = true;
                        }
                    }
                }
            }
            Err(e) => {
                log(&logpath, &e);
                failed = true;
            }
        }
        if once {
            if failed {
                return Err("one or more calendar imports failed; see log".into());
            }
            break;
        }
        if failed {
            // Bounded catch-up retries; successful cycles retain the hourly cadence.
            wait = wait.min(60);
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
