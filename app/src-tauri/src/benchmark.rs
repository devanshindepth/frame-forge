//! Frame-time capture with PresentMon (bundled, MIT licence) + live temperature guard.
//!
//! PresentMon uses Windows Event Tracing — the same mechanism Windows' own
//! performance tools use. It does not inject into or modify the game, so it is
//! safe with anti-cheat systems. It requires the user to be in the built-in
//! "Performance Log Users" group, which the installer sets up once.

use crate::hardware;
use crate::util;
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Measurement {
    pub fps_avg: f64,
    pub fps_low: f64,
    pub frames: usize,
    pub max_gpu_temp: Option<f64>,
    pub max_vram_gb: Option<f64>,
    pub too_hot: bool,
}

pub struct Capture<'a> {
    pub presentmon: &'a Path,
    pub process_name: &'a str,
    pub duration_s: u64,
    pub temp_limit_c: f64,
    pub work_dir: &'a Path,
    pub cancel: &'a AtomicBool,
    /// Called about once per second with seconds elapsed (for the progress bar).
    pub on_tick: &'a dyn Fn(u64, Option<f64>),
}

pub fn presentmon_ready(presentmon: &Path) -> bool {
    presentmon.exists()
}

pub fn capture(c: Capture) -> Result<Measurement> {
    if !c.presentmon.exists() {
        bail!("The measuring tool is missing. Please reinstall FrameForge.");
    }
    let csv: PathBuf = c.work_dir.join(format!("capture-{}.csv", uuid::Uuid::new_v4()));
    let mut child = util::hidden_command(c.presentmon)
        .args([
            "--process_name", c.process_name,
            "--output_file", &csv.display().to_string(),
            "--timed", &c.duration_s.to_string(),
            "--terminate_after_timed",
            "--stop_existing_session",
            "--session_name", "FrameForgeCapture",
            "--no_console_stats",
            "--v1_metrics",
        ])
        .spawn()
        .map_err(|e| anyhow!("Could not start the measuring tool: {e}"))?;

    let start = Instant::now();
    let deadline = Duration::from_secs(c.duration_s + 20);
    let mut m = Measurement::default();
    let mut last_tick = 0;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() && !csv.exists() {
                bail!("access");
            }
            break;
        }
        if c.cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = std::fs::remove_file(&csv);
            bail!("cancelled");
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            break;
        }
        let secs = start.elapsed().as_secs();
        if secs != last_tick && secs % 2 == 0 {
            last_tick = secs;
            let s = hardware::sample_gpu();
            if let Some(t) = s.temp_c {
                m.max_gpu_temp = Some(m.max_gpu_temp.map_or(t, |x: f64| x.max(t)));
                if t > c.temp_limit_c {
                    m.too_hot = true;
                    util::log(format!("temperature guard: {t} °C > {} °C, stopping test", c.temp_limit_c));
                    let _ = child.kill();
                    break;
                }
            }
            if let Some(v) = s.vram_used_gb {
                m.max_vram_gb = Some(m.max_vram_gb.map_or(v, |x: f64| x.max(v)));
            }
            (c.on_tick)(secs.min(c.duration_s), s.temp_c);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = child.wait();

    if m.too_hot {
        let _ = std::fs::remove_file(&csv);
        return Ok(m);
    }
    let text = std::fs::read_to_string(&csv).map_err(|_| anyhow!("no-frames"))?;
    let _ = std::fs::remove_file(&csv);
    let frametimes = parse_frametimes(&text);
    if frametimes.len() < 100 {
        bail!("no-frames");
    }
    let (avg, low) = summarize(&frametimes);
    m.fps_avg = avg;
    m.fps_low = low;
    m.frames = frametimes.len();
    util::log(format!("capture: {} frames, {avg:.1} fps avg, {low:.1} fps 1% low", m.frames));
    Ok(m)
}

/// Pull frame times (ms) out of a PresentMon CSV (v1 or v2 column names).
pub fn parse_frametimes(csv: &str) -> Vec<f64> {
    let mut lines = csv.lines();
    let Some(header) = lines.next() else { return vec![] };
    let cols: Vec<String> = header.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect();
    let idx = ["MsBetweenPresents", "msBetweenPresents", "FrameTime", "MsBetweenDisplayChange"]
        .iter()
        .find_map(|name| cols.iter().position(|c| c == name));
    let Some(i) = idx else { return vec![] };
    lines
        .filter_map(|l| l.split(',').nth(i)?.trim().trim_matches('"').parse::<f64>().ok())
        .filter(|ft| *ft > 0.05 && *ft < 1000.0)
        .collect()
}

/// (average FPS, "1% low" FPS = FPS of the slowest 1% of frames).
pub fn summarize(ft: &[f64]) -> (f64, f64) {
    // drop the first second (loading hitch after the player presses F9)
    let mut acc = 0.0;
    let skip = ft.iter().take_while(|x| { acc += **x; acc < 1000.0 }).count();
    let ft = if ft.len() - skip > 200 { &ft[skip..] } else { ft };
    let total: f64 = ft.iter().sum();
    let avg = ft.len() as f64 / total * 1000.0;
    let mut sorted = ft.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let n = (sorted.len() / 100).max(1);
    let worst = sorted[..n].iter().sum::<f64>() / n as f64;
    (avg, 1000.0 / worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v1_csv() {
        let mut csv = String::from("Application,ProcessID,MsBetweenPresents\n");
        for i in 0..500 {
            csv.push_str(&format!("game.exe,1,{}\n", if i % 100 == 0 { 33.3 } else { 10.0 }));
        }
        let ft = parse_frametimes(&csv);
        assert_eq!(ft.len(), 500);
        let (avg, low) = summarize(&ft);
        assert!(avg > 90.0 && avg < 101.0, "avg {avg}");
        assert!(low < 40.0, "low {low}");
    }
}
