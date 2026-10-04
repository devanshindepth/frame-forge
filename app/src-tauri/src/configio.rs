//! Read / write game settings files and keep backups.
//!
//! Supported formats
//! * `ini`  — keys written as `Section/Key` or plain `Key` (first match anywhere).
//! * `kv`   — Valve quoted key/value text, e.g. CS2 `cs2_video.txt`.
//! * `json` — dotted path into a JSON object, e.g. `graphics.shadows`.
//!
//! Every write is atomic; the original file is copied to the backup folder
//! before the first change of a session, and restored on stop / crash.

use crate::util;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn read_values(format: &str, path: &Path, keys: &[String]) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(match format {
        "ini" => ini_read(&text, keys),
        "kv" => kv_read(&text, keys),
        "json" => json_read(&text, keys)?,
        f => return Err(anyhow!("unsupported format {f}")),
    })
}

pub fn write_values(format: &str, path: &Path, values: &[(String, String)]) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let crlf = text.contains("\r\n");
    let new_text = match format {
        "ini" => ini_write(&text, values),
        "kv" => kv_write(&text, values),
        "json" => json_write(&text, values)?,
        f => return Err(anyhow!("unsupported format {f}")),
    };
    let new_text = if crlf && !new_text.contains("\r\n") { new_text.replace('\n', "\r\n") } else { new_text };
    util::atomic_write(path, new_text.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------- INI

fn split_key(k: &str) -> (Option<&str>, &str) {
    match k.rsplit_once('/') {
        Some((s, k)) => (Some(s), k),
        None => (None, k),
    }
}

fn ini_read(text: &str, keys: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].to_string();
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        for full in keys {
            let (sec, key) = split_key(full);
            if k.trim().eq_ignore_ascii_case(key) && sec.map_or(true, |s| s.eq_ignore_ascii_case(&section)) && !out.contains_key(full) {
                out.insert(full.clone(), v.trim().to_string());
            }
        }
    }
    out
}

fn ini_write(text: &str, values: &[(String, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    for (full, val) in values {
        let (sec, key) = split_key(full);
        let mut section = String::new();
        let mut section_end: Option<usize> = None;
        let mut done = false;
        for i in 0..lines.len() {
            let t = lines[i].trim().to_string();
            if t.starts_with('[') && t.ends_with(']') {
                if sec.map_or(false, |s| s.eq_ignore_ascii_case(&section)) && section_end.is_none() {
                    section_end = Some(i);
                }
                section = t[1..t.len() - 1].to_string();
                continue;
            }
            if let Some((k, _)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) && sec.map_or(true, |s| s.eq_ignore_ascii_case(&section)) {
                    lines[i] = format!("{}={}", k.trim(), val);
                    done = true;
                    break;
                }
            }
        }
        if !done {
            match sec {
                Some(s) => {
                    let has_section = lines.iter().any(|l| l.trim().eq_ignore_ascii_case(&format!("[{s}]")));
                    if has_section {
                        let at = section_end.unwrap_or(lines.len());
                        lines.insert(at, format!("{key}={val}"));
                    } else {
                        lines.push(String::new());
                        lines.push(format!("[{s}]"));
                        lines.push(format!("{key}={val}"));
                    }
                }
                None => lines.push(format!("{key}={val}")),
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

// ---------------------------------------------------------------- Valve KV

fn kv_read(text: &str, keys: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let t = util::quoted_tokens(line);
        if t.len() == 2 {
            if let Some(k) = keys.iter().find(|k| k.eq_ignore_ascii_case(&t[0])) {
                out.insert(k.clone(), t[1].clone());
            }
        }
    }
    out
}

fn kv_write(text: &str, values: &[(String, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    for (key, val) in values {
        let mut done = false;
        for line in lines.iter_mut() {
            let t = util::quoted_tokens(line);
            if t.len() == 2 && t[0].eq_ignore_ascii_case(key) {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                *line = format!("{indent}\"{}\"\t\t\"{}\"", t[0], val);
                done = true;
                break;
            }
        }
        if !done {
            // insert before the last closing brace
            let pos = lines.iter().rposition(|l| l.trim() == "}").unwrap_or(lines.len());
            lines.insert(pos, format!("\t\"{key}\"\t\t\"{val}\""));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

// ---------------------------------------------------------------- JSON

fn json_read(text: &str, keys: &[String]) -> Result<HashMap<String, String>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let mut out = HashMap::new();
    for k in keys {
        let mut cur = &v;
        let mut ok = true;
        for part in k.split('.') {
            match cur.get(part) {
                Some(n) => cur = n,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let s = match cur {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.insert(k.clone(), s);
        }
    }
    Ok(out)
}

fn json_write(text: &str, values: &[(String, String)]) -> Result<String> {
    let mut v: serde_json::Value = serde_json::from_str(text)?;
    for (k, val) in values {
        let parts: Vec<&str> = k.split('.').collect();
        let mut cur = &mut v;
        for (i, part) in parts.iter().enumerate() {
            if !cur.is_object() {
                *cur = serde_json::json!({});
            }
            let obj = cur.as_object_mut().expect("object");
            if i == parts.len() - 1 {
                // keep the original type (number / bool / string)
                let new_val = match obj.get(*part) {
                    Some(serde_json::Value::Number(_)) => {
                        if let Ok(i) = val.parse::<i64>() {
                            serde_json::json!(i)
                        } else {
                            val.parse::<f64>().map(|n| serde_json::json!(n)).unwrap_or(serde_json::json!(val))
                        }
                    }
                    Some(serde_json::Value::Bool(_)) => serde_json::json!(val == "true" || val == "1"),
                    _ => serde_json::json!(val),
                };
                obj.insert((*part).to_string(), new_val);
                break;
            }
            cur = obj.entry((*part).to_string()).or_insert_with(|| serde_json::json!({}));
        }
    }
    Ok(serde_json::to_string_pretty(&v)?)
}

// ---------------------------------------------------------------- backups

#[derive(serde::Serialize, Clone, Debug)]
pub struct BackupInfo {
    pub file: String,
    pub created: String,
    pub size: u64,
}

pub fn backup(backup_dir: &Path, game_id: &str, config: &Path) -> Result<PathBuf> {
    let dir = backup_dir.join(game_id);
    std::fs::create_dir_all(&dir)?;
    let name = format!(
        "{}__{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        config.file_name().and_then(|s| s.to_str()).unwrap_or("settings")
    );
    let dest = dir.join(name);
    std::fs::copy(config, &dest).with_context(|| format!("backing up {}", config.display()))?;
    prune(&dir, 20);
    util::log(format!("backup {} -> {}", config.display(), dest.display()));
    Ok(dest)
}

pub fn restore(backup: &Path, config: &Path) -> Result<()> {
    let bytes = std::fs::read(backup).with_context(|| format!("reading backup {}", backup.display()))?;
    util::atomic_write(config, &bytes)?;
    util::log(format!("restored {} from {}", config.display(), backup.display()));
    Ok(())
}

pub fn list(backup_dir: &Path, game_id: &str) -> Vec<BackupInfo> {
    let dir = backup_dir.join(game_id);
    let mut out: Vec<BackupInfo> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    let created: chrono::DateTime<chrono::Local> = m.modified().ok()?.into();
                    Some(BackupInfo { file: e.path().display().to_string(), created: created.format("%d %b %Y, %H:%M").to_string(), size: m.len() })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.file.cmp(&a.file));
    out
}

/// The very first backup ever made for this game = the player's original settings.
pub fn oldest(backup_dir: &Path, game_id: &str) -> Option<PathBuf> {
    list(backup_dir, game_id).last().map(|b| PathBuf::from(&b.file))
}

fn prune(dir: &Path, keep: usize) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    // always keep the oldest (original) and the newest `keep - 1`
    if files.len() > keep {
        for f in &files[1..files.len() - (keep - 1)] {
            let _ = std::fs::remove_file(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_roundtrip() {
        let src = "[Graphics]\nShadows=3\nFog=2\n[Audio]\nShadows=9\n";
        let out = ini_write(src, &[("Graphics/Shadows".into(), "1".into()), ("Graphics/AO".into(), "0".into())]);
        let r = ini_read(&out, &["Graphics/Shadows".into(), "Audio/Shadows".into(), "Graphics/AO".into()]);
        assert_eq!(r["Graphics/Shadows"], "1");
        assert_eq!(r["Audio/Shadows"], "9");
        assert_eq!(r["Graphics/AO"], "0");
    }

    #[test]
    fn kv_roundtrip() {
        let src = "\"video.cfg\"\n{\n\t\"setting.shaderquality\"\t\t\"1\"\n}\n";
        let out = kv_write(src, &[("setting.shaderquality".into(), "0".into()), ("setting.msaa_samples".into(), "4".into())]);
        let r = kv_read(&out, &["setting.shaderquality".into(), "setting.msaa_samples".into()]);
        assert_eq!(r["setting.shaderquality"], "0");
        assert_eq!(r["setting.msaa_samples"], "4");
        assert!(out.trim_end().ends_with('}'));
    }

    #[test]
    fn json_roundtrip() {
        let src = r#"{"gfx":{"shadows":3,"rt":true,"name":"x"}}"#;
        let out = json_write(src, &[("gfx.shadows".into(), "1".into()), ("gfx.rt".into(), "false".into())]).unwrap();
        let r = json_read(&out, &["gfx.shadows".into(), "gfx.rt".into()]).unwrap();
        assert_eq!(r["gfx.shadows"], "1");
        assert_eq!(r["gfx.rt"], "false");
    }
}
