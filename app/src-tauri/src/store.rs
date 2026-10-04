//! Local persistence: one JSON file in the app-data folder. Nothing leaves the PC.

use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Preferences {
    /// Abort a test if the graphics card gets hotter than this (°C).
    pub temp_limit_c: f64,
    /// "short" (40 s) | "normal" (60 s) | "thorough" (90 s)
    pub test_length: String,
    /// Upper bound of tests in one session.
    pub max_tests: usize,
    /// Screen refresh rate. 0 = detected automatically by the UI.
    pub refresh_hz: f64,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { temp_limit_c: 87.0, test_length: "normal".into(), max_tests: 12, refresh_hz: 0.0 }
    }
}

impl Preferences {
    pub fn duration_s(&self) -> u64 {
        match self.test_length.as_str() {
            "short" => 40,
            "thorough" => 90,
            _ => 60,
        }
    }
    pub fn sanitized(mut self) -> Self {
        self.temp_limit_c = self.temp_limit_c.clamp(70.0, 95.0);
        self.max_tests = self.max_tests.clamp(6, 30);
        if self.refresh_hz != 0.0 {
            self.refresh_hz = self.refresh_hz.clamp(30.0, 500.0);
        }
        if !["short", "normal", "thorough"].contains(&self.test_length.as_str()) {
            self.test_length = "normal".into();
        }
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunRecord {
    pub index: usize,
    pub levels: Vec<usize>,
    pub fps_avg: f64,
    pub fps_low: f64,
    pub quality: f64,
    pub vram_gb: Option<f64>,
    pub max_gpu_temp: Option<f64>,
    pub safe: bool,
    pub note: String,
    /// False when the game changed our values back (setting not really applied).
    #[serde(default = "yes")]
    pub settings_held: bool,
    /// "search" | "check" (final repeat measurement)
    #[serde(default)]
    pub kind: String,
}

fn yes() -> bool {
    true
}

/// Final numbers = average of the search run and the repeat check run.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Confirmed {
    pub baseline_fps: f64,
    pub baseline_low: f64,
    pub best_fps: f64,
    pub best_low: f64,
    /// Biggest difference between two runs of the same settings, in % (measurement noise).
    pub noise_pct: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionResult {
    pub id: String,
    pub game_id: String,
    pub goal: String,
    pub quality_floor: f64,
    pub started_at: String,
    pub finished_at: String,
    pub engine: String,
    /// "completed" | "stopped" | "failed"
    pub status: String,
    pub baseline: Option<RunRecord>,
    pub best: Option<RunRecord>,
    pub runs: Vec<RunRecord>,
    pub applied: bool,
    #[serde(default)]
    pub confirmed: Option<Confirmed>,
}

impl SessionResult {
    pub fn gain_pct(&self) -> Option<f64> {
        if let Some(c) = &self.confirmed {
            if c.baseline_fps > 0.0 {
                return Some((c.best_fps / c.baseline_fps - 1.0) * 100.0);
            }
        }
        let (b, n) = (self.baseline.as_ref()?, self.best.as_ref()?);
        if b.fps_avg <= 0.0 {
            return None;
        }
        Some((n.fps_avg / b.fps_avg - 1.0) * 100.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct StoreData {
    pub version: u32,
    pub onboarded: bool,
    pub preferences: Preferences,
    pub results: Vec<SessionResult>,
    /// Set while a session is modifying a game's settings file (crash recovery).
    pub active_session_game: Option<String>,
    /// Backup file to restore if the app closed during a session.
    pub active_backup: Option<String>,
    pub active_config: Option<String>,
}

impl Default for StoreData {
    fn default() -> Self {
        Self { version: 1, onboarded: false, preferences: Preferences::default(), results: vec![], active_session_game: None, active_backup: None, active_config: None }
    }
}

pub struct Store {
    path: PathBuf,
    pub data: StoreData,
}

impl Store {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("frameforge.json");
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<StoreData>(&text) {
                Ok(d) => d,
                Err(e) => {
                    util::log(format!("store corrupt ({e}); starting fresh, old copy kept"));
                    let _ = std::fs::rename(&path, dir.join("frameforge.corrupt.json"));
                    StoreData::default()
                }
            },
            Err(_) => StoreData::default(),
        };
        Self { path, data }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(&self.data)?;
        util::atomic_write(&self.path, text.as_bytes())?;
        Ok(())
    }

    pub fn latest_for(&self, game_id: &str) -> Option<&SessionResult> {
        self.data.results.iter().rev().find(|r| r.game_id == game_id && r.best.is_some())
    }

    pub fn result(&self, id: &str) -> Option<&SessionResult> {
        self.data.results.iter().find(|r| r.id == id)
    }
}
