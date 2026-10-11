//! Short-lived host-owned management actions. No producer lock, feed access or child processes.
use crate::{read_config, CalendarSet, Result};
use std::{fs, io::Read, path::Path, time::Duration};

pub fn id(command: &str) -> Option<&'static str> {
    match command {
        "configure" => Some("configure"),
        "status-ui" => Some("status"),
        "open-logs" => Some("logs"),
        "about" => Some("about"),
        _ => None,
    }
}

pub fn record_action(path: &Path, testing: bool, custom: bool, command: &str, success: bool) {
    use std::io::Write;
    let log_path = if custom {
        path.with_extension("actions.log")
    } else if let Some(root) = path.parent().and_then(Path::parent) {
        root.join("Logs/aw-calendar-sync").join(if testing {
            "aw-calendar-sync-testing-actions.log"
        } else {
            "aw-calendar-sync-actions.log"
        })
    } else {
        return;
    };
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(&log_path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::rename(&log_path, log_path.with_extension("log.1"));
    }
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(
            file,
            "{} action={} completed={}",
            chrono::Utc::now().to_rfc3339(),
            id(command).unwrap_or("unknown"),
            success
        );
    }
}

pub fn save_config(path: &Path, text: &str) -> Result<()> {
    CalendarSet::from_json(text)?;
    // Create exclusively, then atomically replace. Never truncate a live config.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut owned = false;
    let result = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "cannot create config replacement")?;
        owned = true;
        file.write_all(text.as_bytes())
            .map_err(|_| "cannot write config replacement")?;
        file.sync_all()
            .map_err(|_| "cannot flush config replacement")?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "cannot replace config")?;
        Ok(())
    })();
    if owned {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn safe_logs(path: &Path) -> String {
    // Do not display arbitrary historical error text or raw file contents.
    let Ok(file) = fs::File::open(path) else {
        return "No sync log available for this profile.".into();
    };
    let mut text = String::new();
    if file
        .take(2 * 1024 * 1024)
        .read_to_string(&mut text)
        .is_err()
    {
        return "Sync log unavailable.".into();
    }
    let lines: Vec<_> = text
        .lines()
        .rev()
        .take(30)
        .map(|line| {
            let time = line.split_whitespace().next().unwrap_or("");
            let timestamp = if chrono::DateTime::parse_from_rfc3339(time).is_ok() {
                time
            } else {
                "Unknown time"
            };
            let counts: Vec<_> = line
                .split_whitespace()
                .filter(|word| {
                    word.split_once('=').is_some_and(|(key, value)| {
                        ["eligible", "inserted", "unchanged", "changed", "deleted"].contains(&key)
                            && !value.is_empty()
                            && value.bytes().all(|c| c.is_ascii_digit())
                    })
                })
                .collect();
            if counts.is_empty() {
                format!("{timestamp}  Status/error entry (details omitted)")
            } else {
                format!("{timestamp}  {}", counts.join(" "))
            }
        })
        .collect();
    if lines.is_empty() {
        "No sync entries yet.".into()
    } else {
        lines.join("\r\n")
    }
}

pub fn dispatch(
    command: &str,
    path: &Path,
    testing: bool,
    server: Option<&str>,
    custom: bool,
    headless: bool,
) -> Result<()> {
    id(command).ok_or("unknown action")?;
    if command == "configure" && !headless {
        return native::edit(path);
    }
    let log_path = if custom {
        path.with_extension("log")
    } else {
        path.parent()
            .and_then(Path::parent)
            .ok_or("invalid config location")?
            .join("Logs/aw-calendar-sync")
            .join(if testing {
                "aw-calendar-sync-testing.log"
            } else {
                "aw-calendar-sync.log"
            })
    };
    let message = match command {
        "about" => format!("aw-calendar-sync {}\r\nActivityWatch-managed calendar interval importer.\r\nCompleted events only; at most seven days of catch-up.\r\nCalendar intervals are scheduled time, not proof of attendance.\r\nTray actions protocol 1. No refresh IPC is implemented.", env!("CARGO_PKG_VERSION")),
        "open-logs" => safe_logs(&log_path),
        "configure" => {
            read_config(path, testing, server)?;
            format!("Configuration valid: {}\r\nNative configuration editor available without --headless.\r\nControls: payload_mode, include_summary, include_all_day, include_status, include_attendees.\r\nNo feed accessed; no producer started.", path.display())
        }
        "status-ui" => {
            let settings = read_config(path, testing, server)?;
            let endpoint = settings.server_url.clone();
            let response = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).redirects(0).try_proxy_from_env(false).build()
                .get(&format!("{}/api/0/info", endpoint.trim_end_matches('/'))).call();
            let connected = response.ok().filter(|r| r.status() == 200).and_then(|r| {
                let mut text = String::new();
                r.into_reader().take(65536).read_to_string(&mut text).ok()?;
                serde_json::from_str::<serde_json::Value>(&text).ok()
            }).is_some_and(|v| v["version"].is_string() && v["hostname"].is_string());
            format!("AW connection: {}\r\nEnabled calendars: {}\r\nProducer running state: owned by the tray; not inferred by this helper.\r\nFeed access and sync success were not tested.\r\nRecent entries (newest first):\r\n{}",
                if connected {"reachable; valid info response"} else {"unavailable or invalid info response"}, settings.calendars.len(), safe_logs(&log_path))
        }
        _ => return Err("unknown action".into()),
    };
    if headless {
        println!("{message}");
        Ok(())
    } else {
        native::message(&message)
    }
}

#[cfg(not(windows))]
mod native {
    use super::*;
    pub fn message(_: &str) -> Result<()> {
        Err("native actions require Windows; use --headless".into())
    }
    pub fn edit(_: &Path) -> Result<()> {
        Err("native configuration editor requires Windows".into())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{cell::RefCell, ffi::c_void, path::PathBuf, ptr};
    type Handle = *mut c_void;
    #[repr(C)]
    struct Class {
        style: u32,
        proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        extra: i32,
        window_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        name: *const u16,
    }
    #[repr(C)]
    struct Message {
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        x: i32,
        y: i32,
        private: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(h: Handle, text: *const u16, title: *const u16, kind: u32) -> i32;
        fn RegisterClassW(class: *const Class) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: Handle,
        ) -> Handle;
        fn DefWindowProcW(h: Handle, m: u32, w: usize, l: isize) -> isize;
        fn DestroyWindow(h: Handle) -> i32;
        fn PostQuitMessage(code: i32);
        fn GetMessageW(m: *mut Message, h: Handle, min: u32, max: u32) -> i32;
        fn TranslateMessage(m: *const Message) -> i32;
        fn DispatchMessageW(m: *const Message) -> isize;
        fn GetWindowTextLengthW(h: Handle) -> i32;
        fn GetWindowTextW(h: Handle, text: *mut u16, count: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    thread_local! {static EDITOR: RefCell<Option<(PathBuf,Handle)>> = const {RefCell::new(None)};}
    pub fn message(text: &str) -> Result<()> {
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(text).as_ptr(),
                wide("Calendar sync").as_ptr(),
                0,
            );
        }
        Ok(())
    }
    unsafe extern "system" fn callback(h: Handle, m: u32, w: usize, l: isize) -> isize {
        if m == 0x111 && w & 0xffff == 1 {
            EDITOR.with(|state| {
                if let Some((path,edit)) = state.borrow().as_ref() {
                    let length = GetWindowTextLengthW(*edit);
                    if length <= 0 || length > 131072 { let _ = message("Config must contain 1..131072 characters."); return; }
                    let mut text=vec![0u16;length as usize+1];
                    GetWindowTextW(*edit,text.as_mut_ptr(),length+1);
                    match save_config(path,&String::from_utf16_lossy(&text[..length as usize])) {
                        Ok(()) => {let _=message("Saved. The running module reloads at its next check. No import was started.");DestroyWindow(h);}
                        Err(e) => {let _=message(&format!("Not saved: {e}"));}
                    }
                }
            });
            return 0;
        }
        if (m == 0x111 && w & 0xffff == 2) || m == 0x10 {
            DestroyWindow(h);
            return 0;
        }
        if m == 2 {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(h, m, w, l)
    }
    pub fn edit(path: &Path) -> Result<()> {
        let text = fs::read_to_string(path).map_err(|_| "cannot read config")?;
        if text.encode_utf16().count() > 131072 {
            return Err("config too large for editor".into());
        }
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let name = wide("AwCalendarConfig");
            let class = Class {
                style: 0,
                proc: Some(callback),
                extra: 0,
                window_extra: 0,
                instance,
                icon: ptr::null_mut(),
                cursor: ptr::null_mut(),
                background: 6usize as Handle,
                menu: ptr::null(),
                name: name.as_ptr(),
            };
            if RegisterClassW(&class) == 0 {
                return Err("cannot register configuration window".into());
            }
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                wide("Calendar sources and privacy").as_ptr(),
                0x10ca0000,
                100,
                100,
                900,
                650,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            );
            if window.is_null() {
                return Err("cannot create configuration window".into());
            }
            CreateWindowExW(0,wide("STATIC").as_ptr(),wide("Edit local JSON. URLs stay private. Privacy: payload_mode (strict/extended), include_summary, include_all_day, include_status, include_attendees. Settings may be overridden per calendar. Older history is preserved.").as_ptr(),0x50000000,12,12,850,52,window,ptr::null_mut(),instance,ptr::null_mut());
            let edit = CreateWindowExW(
                0x200,
                wide("EDIT").as_ptr(),
                wide(&text).as_ptr(),
                0x503110c4,
                12,
                70,
                850,
                490,
                window,
                100usize as Handle,
                instance,
                ptr::null_mut(),
            );
            if edit.is_null() {
                DestroyWindow(window);
                return Err("cannot create config editor".into());
            }
            EDITOR.with(|state| *state.borrow_mut() = Some((path.to_owned(), edit)));
            for (label, id, x) in [("Save", 1, 665), ("Cancel", 2, 765)] {
                CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide(label).as_ptr(),
                    0x50010000,
                    x,
                    570,
                    95,
                    30,
                    window,
                    id as Handle,
                    instance,
                    ptr::null_mut(),
                );
            }
            let mut msg = std::mem::zeroed::<Message>();
            loop {
                let result = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            EDITOR.with(|state| *state.borrow_mut() = None);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_edits_preserve_original_and_valid_edits_replace() {
        let path = std::env::temp_dir().join(format!("calendar-edit-{}.json", std::process::id()));
        let original = r#"{"calendar_file":"synthetic.ics"}"#;
        fs::write(&path, original).unwrap();
        assert!(save_config(&path, "{invalid private input}").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        save_config(
            &path,
            r#"{"calendar_file":"synthetic.ics","include_summary":false}"#,
        )
        .unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("false"));
        fs::remove_file(path).unwrap();
    }
}
