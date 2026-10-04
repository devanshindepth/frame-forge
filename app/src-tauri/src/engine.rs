//! The "Smart Tuner": decides which settings to test next.
//!
//! Primary: the bundled offline AI engine (`resources/engine/frameforge-engine.exe`,
//! Python + TabPFN frozen with PyInstaller, model weights included). It is a
//! long-running child process speaking one JSON object per line on stdin/stdout.
//! It never opens a network connection (HF_HUB_OFFLINE / telemetry disabled).
//!
//! Fallback: if the engine is missing or crashes, a built-in "Basic tuner" in Rust
//! keeps the session going. It fits 1/fps = a + b * cost(settings), a physically
//! motivated frame-time model, and searches it. Results are good but need more tests.

use crate::util;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Serialize, Clone, Debug)]
pub struct Observation {
    pub levels: Vec<usize>,
    pub fps_avg: f64,
    pub fps_low: f64,
    pub safe: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Problem {
    pub space: Vec<usize>,
    pub quality: Vec<Vec<f64>>,
    pub cost: Vec<Vec<f64>>,
    pub quality_floor: f64,
    pub target_hz: f64,
    /// "smooth" | "balanced" | "beautiful"
    pub goal: String,
    pub max_tests: usize,
    pub hardware: Vec<f64>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Suggestion {
    #[serde(default)]
    pub levels: Vec<usize>,
    #[serde(default)]
    pub stop: bool,
    #[serde(default)]
    pub predicted_fps: Option<f64>,
    /// 0..1, how sure the tuner is that this beats the current best.
    #[serde(default)]
    pub chance_better: Option<f64>,
    /// Plain-language reason shown to the player.
    #[serde(default)]
    pub reason: String,
}

pub struct Engine {
    ai: Option<AiProcess>,
    pub label: String,
}

struct AiProcess {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<String>,
}

impl Engine {
    pub fn start(resource_dir: &Path) -> Self {
        let exe = resource_dir.join("engine").join(if cfg!(windows) { "frameforge-engine.exe" } else { "frameforge-engine" });
        if !exe.exists() {
            util::log(format!("AI engine not found at {} -> basic tuner", exe.display()));
            return Self { ai: None, label: "Basic tuner".into() };
        }
        match AiProcess::spawn(&exe) {
            Ok(mut p) => match p.call(&json!({"cmd": "hello"}), Duration::from_secs(90)) {
                Ok(v) if v.get("ok").and_then(|x| x.as_bool()) == Some(true) => {
                    util::log(format!("AI engine ready: {v}"));
                    Self { ai: Some(p), label: "Smart Tuner (TabPFN, offline)".into() }
                }
                other => {
                    util::log(format!("AI engine hello failed: {other:?} -> basic tuner"));
                    p.kill();
                    Self { ai: None, label: "Basic tuner".into() }
                }
            },
            Err(e) => {
                util::log(format!("AI engine spawn failed: {e} -> basic tuner"));
                Self { ai: None, label: "Basic tuner".into() }
            }
        }
    }

    pub fn suggest(&mut self, problem: &Problem, history: &[Observation]) -> Suggestion {
        if let Some(ai) = self.ai.as_mut() {
            let req = json!({"cmd": "suggest", "problem": problem, "history": history});
            match ai.call(&req, Duration::from_secs(120)) {
                Ok(v) => match serde_json::from_value::<Suggestion>(v.clone()) {
                    Ok(s) if s.stop || valid(&s.levels, &problem.space) => return s,
                    _ => util::log(format!("AI engine bad reply: {v}")),
                },
                Err(e) => util::log(format!("AI engine error: {e}")),
            }
            util::log("switching to basic tuner for the rest of the session");
            if let Some(mut p) = self.ai.take() {
                p.kill();
            }
            self.label = "Basic tuner".into();
        }
        basic_suggest(problem, history)
    }

    pub fn is_ai(&self) -> bool {
        self.ai.is_some()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(mut p) = self.ai.take() {
            let _ = writeln!(p.stdin, "{}", json!({"cmd": "quit"}));
            std::thread::sleep(Duration::from_millis(200));
            p.kill();
        }
    }
}

impl AiProcess {
    fn spawn(exe: &Path) -> std::io::Result<Self> {
        let mut child = util::hidden_command(exe)
            .env("HF_HUB_OFFLINE", "1")
            .env("TABPFN_DISABLE_TELEMETRY", "1")
            .env("NO_PROXY", "*")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout: ChildStdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.trim_start().starts_with('{') && tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, stdin, rx })
    }

    fn call(&mut self, req: &serde_json::Value, timeout: Duration) -> anyhow::Result<serde_json::Value> {
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;
        let line = self.rx.recv_timeout(timeout)?;
        let v: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(err) = v.get("error") {
            anyhow::bail!("engine: {err}");
        }
        Ok(v)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn valid(levels: &[usize], space: &[usize]) -> bool {
    levels.len() == space.len() && levels.iter().zip(space).all(|(l, n)| l < n)
}

// ---------------------------------------------------------------- basic tuner (fallback)

pub fn quality_of(levels: &[usize], q: &[Vec<f64>]) -> f64 {
    let max: f64 = q.iter().map(|v| v.iter().cloned().fold(0.0, f64::max)).sum();
    let got: f64 = q.iter().zip(levels).map(|(v, &l)| v.get(l).copied().unwrap_or(0.0)).sum();
    if max <= 0.0 { 100.0 } else { got / max * 100.0 }
}

fn cost_of(levels: &[usize], c: &[Vec<f64>]) -> f64 {
    c.iter().zip(levels).map(|(v, &l)| v.get(l).copied().unwrap_or(0.0)).sum()
}

fn objective(fps: f64, low: f64, hz: f64) -> f64 {
    fps.min(hz) + 0.5 * low.min(hz)
}

/// Cheapest configuration (by adapter cost) that still meets the quality floor:
/// greedily lowers whichever setting saves most cost per quality point lost.
pub fn prior_guess(p: &Problem, start: &[usize]) -> Vec<usize> {
    let mut cur: Vec<usize> = p.space.iter().map(|n| n - 1).collect();
    if quality_of(&cur, &p.quality) < p.quality_floor {
        return start.to_vec();
    }
    loop {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..cur.len() {
            if cur[i] == 0 {
                continue;
            }
            let mut c = cur.clone();
            c[i] -= 1;
            if quality_of(&c, &p.quality) < p.quality_floor {
                continue;
            }
            let saved = p.cost[i][cur[i]] - p.cost[i][c[i]];
            let lost = (p.quality[i][cur[i]] - p.quality[i][c[i]]).max(0.05);
            let ratio = saved / lost;
            if saved > 0.0 && best.map_or(true, |(_, r)| ratio > r) {
                best = Some((i, ratio));
            }
        }
        match best {
            Some((i, _)) => cur[i] -= 1,
            None => return cur,
        }
    }
}

fn basic_suggest(p: &Problem, h: &[Observation]) -> Suggestion {
    let mut rng = rand::thread_rng();
    let tried = |c: &Vec<usize>| h.iter().any(|o| &o.levels == c);
    if h.is_empty() {
        return Suggestion { reason: "First we measure your current settings.".into(), ..Default::default() };
    }
    if h.len() == 1 {
        let g = prior_guess(p, &h[0].levels);
        if !tried(&g) {
            return Suggestion { levels: g, reason: "Trying the settings that usually give the biggest speed-up for the smallest visual loss.".into(), ..Default::default() };
        }
    }
    if h.len() >= p.max_tests {
        return Suggestion { stop: true, reason: "Test limit reached.".into(), ..Default::default() };
    }
    // fit 1/fps = a + b*cost on safe runs (least squares)
    let pts: Vec<(f64, f64, f64)> = h.iter().filter(|o| o.safe && o.fps_avg > 0.0).map(|o| (cost_of(&o.levels, &p.cost), 1.0 / o.fps_avg, o.fps_low / o.fps_avg)).collect();
    let (a, b) = if pts.len() >= 2 {
        let n = pts.len() as f64;
        let mx = pts.iter().map(|x| x.0).sum::<f64>() / n;
        let my = pts.iter().map(|x| x.1).sum::<f64>() / n;
        let sxx: f64 = pts.iter().map(|x| (x.0 - mx).powi(2)).sum();
        let sxy: f64 = pts.iter().map(|x| (x.0 - mx) * (x.1 - my)).sum();
        let b = if sxx > 1e-9 { (sxy / sxx).max(1e-6) } else { my / (mx + 1.0) };
        (my - b * mx, b)
    } else {
        let o = &h[0];
        let c = cost_of(&o.levels, &p.cost);
        (0.5 / o.fps_avg, 0.5 / o.fps_avg / c.max(0.1))
    };
    let low_ratio = if pts.is_empty() { 0.7 } else { pts.iter().map(|x| x.2).sum::<f64>() / pts.len() as f64 };
    let predict = |c: &[usize]| {
        let inv = (a + b * cost_of(c, &p.cost)).max(1e-4);
        1.0 / inv
    };
    let best_obs = h.iter().filter(|o| o.safe).map(|o| objective(o.fps_avg, o.fps_low, p.target_hz)).fold(0.0, f64::max);
    let incumbent = h.iter().filter(|o| o.safe).max_by(|x, y| objective(x.fps_avg, x.fps_low, p.target_hz).total_cmp(&objective(y.fps_avg, y.fps_low, p.target_hz))).map(|o| o.levels.clone()).unwrap_or_else(|| h[0].levels.clone());

    let mut best: Option<(Vec<usize>, f64, f64)> = None;
    for k in 0..4000 {
        let mut c = incumbent.clone();
        if k % 3 == 0 {
            c = p.space.iter().map(|&n| rng.gen_range(0..n)).collect();
        } else {
            let mut idx: Vec<usize> = (0..c.len()).collect();
            idx.shuffle(&mut rng);
            for &i in idx.iter().take(rng.gen_range(1..=3)) {
                c[i] = rng.gen_range(0..p.space[i]);
            }
        }
        if tried(&c) || quality_of(&c, &p.quality) < p.quality_floor {
            continue;
        }
        let f = predict(&c);
        // prefer higher quality among similar predicted speed; tiny exploration bonus
        let score = objective(f, f * low_ratio, p.target_hz) + 0.15 * quality_of(&c, &p.quality) + rng.gen_range(0.0..1.0);
        if best.as_ref().map_or(true, |(_, s, _)| score > *s) {
            best = Some((c, score, f));
        }
    }
    match best {
        Some((c, _, f)) => {
            let gain = objective(f, f * low_ratio, p.target_hz) / best_obs.max(1.0) - 1.0;
            if h.len() >= 6 && gain < 0.015 {
                return Suggestion { stop: true, reason: "More tests are unlikely to find anything better.".into(), ..Default::default() };
            }
            Suggestion { levels: c, predicted_fps: Some(f), chance_better: None, stop: false, reason: "Testing a promising combination based on your results so far.".into() }
        }
        None => Suggestion { stop: true, reason: "Every option that meets your picture-quality choice has been covered.".into(), ..Default::default() },
    }
}
