//! Small helpers: hidden child processes, logging, atomic file writes.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init_log(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = LOG_DIR.set(dir.to_path_buf());
    // keep the log from growing forever
    let file = dir.join("frameforge.log");
    if let Ok(meta) = std::fs::metadata(&file) {
        if meta.len() > 5 * 1024 * 1024 {
            let _ = std::fs::rename(&file, dir.join("frameforge.old.log"));
        }
    }
}

/// Append a line to `logs/frameforge.log`. Never panics.
pub fn log(msg: impl AsRef<str>) {
    let Some(dir) = LOG_DIR.get() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("frameforge.log"))
    {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg.as_ref());
    }
}

/// A `Command` that never flashes a console window on Windows.
pub fn hidden_command<S: AsRef<OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Write to a temp file next to `path`, then rename over it (crash-safe).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Some players mark config files read-only; we have to lift that to write.
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(path, perm);
            log(format!("cleared read-only flag on {}", path.display()));
        }
    }
    let tmp = path.with_extension("frameforge-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Return every double-quoted token on a line: `"a"  "b c"` -> ["a", "b c"].
pub fn quoted_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inside = false;
    let mut escaped = false;
    for ch in line.chars() {
        if inside {
            if escaped {
                cur.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                out.push(std::mem::take(&mut cur));
                inside = false;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            inside = true;
        }
    }
    out
}

pub fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.exists()).cloned()
}
