//! One optimisation session, run on a background thread.
//!
//! Test 1 measures the player's settings exactly as they are (file untouched).
//! Each later test: close game → write settings → start game → player presses F9
//! at the test spot → measure → close game → re-read the settings file to check
//! the game kept our values → Smart Tuner picks the next test.
//!
//! When the search ends, the best settings AND the original settings are measured
//! once more. The result shows the average of both measurements and the run-to-run
//! difference, so we only call something "smoother" if the gain is bigger than
//! the measurement noise.
//!
//! The player's ORIGINAL settings are always put back at the end; new settings are
//! only written when the player clicks "Use these settings".

use crate::adapters::{self, Adapter};
use crate::benchmark::{self, Capture};
use crate::engine::{Engine, Observation, Problem};
use crate::store::{Confirmed, RunRecord, SessionResult};
use crate::{configio, hardware, util, AppState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};

#[derive(Serialize, Clone, Debug, Default)]
pub struct SessionStatus {
    pub active: bool,
    pub game_id: String,
    pub game_name: String,
    /// preparing | applying | launching | waiting | warmup | measuring | thinking | checking | finished | stopped | failed
    pub phase: String,
    pub headline: String,
    pub detail: String,
    pub test_index: usize,
    pub max_tests: usize,
    pub seconds_done: u64,
    pub seconds_total: u64,
    pub gpu_temp: Option<f64>,
    pub temp_guard: bool,
    pub engine: String,
    pub best_fps: Option<f64>,
    pub baseline_fps: Option<f64>,
    pub runs: Vec<RunRecord>,
    pub result_id: Option<String>,
    pub error_code: Option<String>,
    pub started: String,
}

pub struct SessionRequest {
    pub game_id: String,
    pub goal: String,
    pub quality_floor: f64,
    pub refresh_hz: f64,
}

fn emit(app: &AppHandle, state: &AppState, f: impl FnOnce(&mut SessionStatus)) {
    let snapshot = {
        let mut s = state.status.lock().unwrap();
        f(&mut s);
        s.clone()
    };
    let _ = app.emit("session", snapshot);
}

/// FPS above this target is worth nothing to the player (screen can't show it).
fn goal_target(goal: &str, hz: f64) -> f64 {
    match goal {
        "smooth" => hz.max(60.0),
        "beautiful" => 60f64.min(hz).max(45.0),
        _ => (hz * 0.75).clamp(60.0, 144.0),
    }
}

fn objective(r: &RunRecord, target: f64) -> f64 {
    r.fps_avg.min(target) + 0.5 * r.fps_low.min(target) + 0.02 * r.quality
}

pub fn spawn(app: AppHandle, req: SessionRequest) {
    std::thread::spawn(move || {
        let state = app.state::<Arc<AppState>>().inner().clone();
        let outcome = run(&app, &state, &req);
        // Always put the original settings back, whatever happened.
        restore_active(&state);
        if let Err(code) = outcome {
            util::log(format!("session ended: {code}"));
            let (phase, headline, detail) = explain(&code);
            emit(&app, &state, |s| {
                s.active = false;
                s.phase = phase.into();
                s.headline = headline.into();
                s.detail = detail.into();
                s.error_code = Some(code.clone());
            });
        }
        state.cancel.store(false, Ordering::SeqCst);
    });
}

/// Put the game's settings file back the way it was before the session.
pub fn restore_active(state: &AppState) {
    let mut store = state.store.lock().unwrap();
    if let (Some(b), Some(c)) = (store.data.active_backup.clone(), store.data.active_config.clone()) {
        if let Err(e) = configio::restore(&PathBuf::from(b), &PathBuf::from(c)) {
            util::log(format!("restore failed: {e}"));
        }
    }
    store.data.active_session_game = None;
    store.data.active_backup = None;
    store.data.active_config = None;
    let _ = store.save();
}

fn explain(code: &str) -> (&'static str, &'static str, &'static str) {
    match code {
        "cancelled" => ("stopped", "Stopped. Your game is back to how it was.", "Nothing was changed. You can start again any time."),
        "no-config" => ("failed", "We couldn't find this game's settings file.", "Start the game once, change any graphics setting in its menu, close it and try again."),
        "access" => ("failed", "Windows blocked the frame counter.", "Restart your PC once after installing FrameForge (Windows needs this to give permission), then try again."),
        "no-frames" => ("failed", "We didn't see the game drawing any frames.", "Make sure you were in the game (not a menu or loading screen) when you pressed F9, and that the game window was in front."),
        "launch" => ("failed", "The game didn't start.", "Check that Steam is running and you're signed in, then try again."),
        "timeout" => ("failed", "We waited, but F9 wasn't pressed.", "When the game is ready, walk to the test spot and press F9. Or click \"I'm in position\" in FrameForge."),
        "presentmon" => ("failed", "A part of FrameForge is missing.", "Please reinstall FrameForge. Your games were not changed."),
        "reverted" => ("failed", "This game undid our setting changes.", "The game rewrote its settings file, so we can't test reliably. A game update may have changed how settings are stored. Your original settings are back. Please report this from Help so we can update support for this game."),
        "adapter-mismatch" => ("failed", "This version of the game stores its settings differently.", "FrameForge didn't recognise most settings in the game's file, so testing wouldn't change anything. Nothing was changed. Please report this from Help so support can be updated."),
        "write" => ("failed", "We couldn't change the game's settings file.", "Another program may have it open, or it's protected. Your original settings are back."),
        _ => ("failed", "Something went wrong. Your game is back to how it was.", "Details were saved to the log file (Settings → Open log folder)."),
    }
}

struct Ctx<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    adapter: &'a Adapter,
    config: &'a Path,
    presentmon: &'a Path,
    process: &'a str,
    duration_s: u64,
    temp_limit_c: f64,
}

/// Run one test with `levels` (or the untouched file when `levels` is None).
/// Returns the record and whether the game kept the values we wrote.
fn measure(c: &Ctx, levels: Option<&[usize]>, label: &str) -> Result<(benchmark::Measurement, bool), String> {
    let (app, state, adapter) = (c.app, c.state, c.adapter);
    hardware::close_processes(&adapter.executables);
    wait_until(|| !hardware::is_running(&adapter.executables), Duration::from_secs(20), state)?;

    let written: Vec<(String, String)> = match levels {
        Some(lv) => {
            let v: Vec<(String, String)> = adapter.settings.iter().zip(lv).map(|(s, &l)| (s.key.clone(), s.options[l].value.clone())).collect();
            configio::write_values(&adapter.config.format, c.config, &v).map_err(|e| {
                util::log(format!("{e:#}"));
                "write".to_string()
            })?;
            v
        }
        None => vec![],
    };

    emit(app, state, |s| {
        s.phase = "launching".into();
        s.headline = format!("Starting {}…", adapter.name);
    });
    launch(adapter)?;
    wait_until(|| hardware::is_running(&adapter.executables), Duration::from_secs(150), state).map_err(|e| if e == "timeout" { "launch".into() } else { e })?;

    state.player_ready.store(false, Ordering::SeqCst);
    emit(app, state, |s| {
        s.phase = "waiting".into();
        s.headline = format!("{label}: go to the test spot and press F9");
        s.detail = if adapter.benchmark.where_to_test.is_empty() { "Pick a busy spot you play often. Do the same thing every test.".into() } else { adapter.benchmark.where_to_test.clone() };
    });
    let hotkey = Shortcut::new(None, Code::F9);
    if let Err(e) = app.global_shortcut().register(hotkey) {
        util::log(format!("F9 hotkey unavailable ({e}); the on-screen button still works"));
    }
    let waited = wait_until(|| state.player_ready.load(Ordering::SeqCst), Duration::from_secs(600), state);
    let _ = app.global_shortcut().unregister(hotkey);
    waited?;

    emit(app, state, |s| {
        s.phase = "warmup".into();
        s.headline = "Hold on, getting a steady reading…".into();
        s.detail = "Keep doing the same thing as in the other tests.".into();
    });
    sleep_cancellable(Duration::from_secs(adapter.benchmark.warmup_s), state)?;
    emit(app, state, |s| {
        s.phase = "measuring".into();
        s.headline = "Measuring… keep playing normally".into();
        s.detail = "FrameForge is counting frames. It doesn't touch the game while it does.".into();
        s.seconds_done = 0;
    });
    let on_tick = |secs: u64, temp: Option<f64>| emit(app, state, |s| {
        s.seconds_done = secs;
        s.gpu_temp = temp;
    });
    // Target the process actively running, prioritizing game engine shipping binaries
    let active_process: String = adapter
        .executables
        .iter()
        .find(|x| x.to_lowercase().contains("shipping") && hardware::is_running(&[(*x).clone()]))
        .or_else(|| adapter.executables.iter().find(|x| hardware::is_running(&[(*x).clone()])))
        .cloned()
        .unwrap_or_else(|| c.process.to_string());
    util::log(format!("targeting process for measurement: {active_process}"));

    let m = benchmark::capture(Capture {
        presentmon: c.presentmon,
        process_name: &active_process,
        duration_s: c.duration_s,
        temp_limit_c: c.temp_limit_c,
        work_dir: &state.data_dir,
        cancel: &state.cancel,
        on_tick: &on_tick,
    })
    .map_err(|e| e.to_string())?;

    // Close the game, then check that it didn't rewrite our values on exit
    // (games with a different version or a cloud-synced config sometimes do).
    hardware::close_processes(&adapter.executables);
    let _ = wait_until(|| !hardware::is_running(&adapter.executables), Duration::from_secs(20), state);
    std::thread::sleep(Duration::from_millis(800));
    let held = if written.is_empty() {
        true
    } else {
        let keys: Vec<String> = written.iter().map(|(k, _)| k.clone()).collect();
        let now = configio::read_values(&adapter.config.format, c.config, &keys).unwrap_or_default();
        let bad: Vec<&String> = written.iter().filter(|(k, v)| now.get(k).map_or(true, |x| !same_value(x, v))).map(|(k, _)| k).collect();
        if !bad.is_empty() {
            util::log(format!("settings not kept by game: {bad:?}"));
        }
        bad.is_empty()
    };
    Ok((m, held))
}

/// "1" == "1.0" == "1.000000" for numeric values; otherwise case-insensitive text.
fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().trim_matches('"'), b.trim().trim_matches('"'));
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-6,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn run(app: &AppHandle, state: &AppState, req: &SessionRequest) -> Result<(), String> {
    let adapter: Adapter = state.adapters.iter().find(|a| a.id == req.game_id).cloned().ok_or("unknown-game")?;
    let prefs = state.store.lock().unwrap().data.preferences.clone();
    let presentmon = state.resource_dir.join("tools").join("PresentMon.exe");
    if !benchmark::presentmon_ready(&presentmon) {
        return Err("presentmon".into());
    }
    let process = adapter.executables.first().cloned().ok_or("unknown-game")?;
    let steam = adapters::Steam::detect();
    let config = adapters::resolve_config(&adapter, &steam).ok_or("no-config")?;
    let target = goal_target(&req.goal, req.refresh_hz);
    let temp_guard = state.hardware.live_sensors;

    emit(app, state, |s| {
        *s = SessionStatus {
            active: true,
            game_id: adapter.id.clone(),
            game_name: adapter.name.clone(),
            phase: "preparing".into(),
            headline: "Getting ready…".into(),
            detail: "Saving a copy of your current settings so we can always put them back.".into(),
            max_tests: prefs.max_tests,
            seconds_total: prefs.duration_s(),
            temp_guard,
            started: chrono::Local::now().to_rfc3339(),
            ..Default::default()
        }
    });

    // 1. backup + remember for crash recovery
    hardware::close_processes(&adapter.executables);
    std::thread::sleep(Duration::from_secs(2));
    let backup = configio::backup(&state.backup_dir, &adapter.id, &config).map_err(|e| {
        util::log(format!("{e:#}"));
        "backup".to_string()
    })?;
    {
        let mut st = state.store.lock().unwrap();
        st.data.active_session_game = Some(adapter.id.clone());
        st.data.active_backup = Some(backup.display().to_string());
        st.data.active_config = Some(config.display().to_string());
        let _ = st.save();
    }

    // 2. current settings → starting point for the tuner
    let keys: Vec<String> = adapter.settings.iter().map(|s| s.key.clone()).collect();
    let current = configio::read_values(&adapter.config.format, &config, &keys).unwrap_or_default();
    let mut unknown = 0;
    let baseline_levels: Vec<usize> = adapter
        .settings
        .iter()
        .map(|s| {
            match current.get(&s.key).and_then(|v| s.options.iter().position(|o| same_value(&o.value, v))) {
                Some(i) => i,
                None => {
                    unknown += 1;
                    s.options.len() / 2
                }
            }
        })
        .collect();
    if unknown > 0 {
        util::log(format!("{unknown} current setting value(s) not recognised: {:?}", adapter.settings.iter().filter(|s| !current.contains_key(&s.key)).map(|s| &s.key).collect::<Vec<_>>()));
    }
    // If most settings aren't in the file, this game version stores them differently
    // and testing would silently change nothing. Refuse instead of producing fake results.
    if unknown * 2 > adapter.settings.len() {
        return Err("adapter-mismatch".into());
    }

    // 3. tuner
    emit(app, state, |s| s.detail = "Starting the Smart Tuner on your PC…".into());
    let mut engine = Engine::start(&state.resource_dir);
    let hw = &state.hardware;
    let problem = Problem {
        space: adapter.space(),
        quality: adapter.quality_weights(),
        cost: adapter.cost_weights(),
        quality_floor: req.quality_floor,
        target_hz: target,
        goal: req.goal.clone(),
        max_tests: prefs.max_tests,
        hardware: vec![hw.vram_gb, hw.ram_gb, hw.cpu_threads as f64],
    };
    emit(app, state, |s| s.engine = engine.label.clone());

    let ctx = Ctx { app, state, adapter: &adapter, config: &config, presentmon: &presentmon, process: &process, duration_s: prefs.duration_s(), temp_limit_c: prefs.temp_limit_c };
    let mut history: Vec<Observation> = vec![];
    let mut runs: Vec<RunRecord> = vec![];
    let mut next_levels = baseline_levels.clone();
    let mut next_reason = "First we measure your settings exactly as they are now, so we know where you're starting from.".to_string();
    let mut reverted_in_a_row = 0;

    let record = |i: usize, lv: &[usize], m: &benchmark::Measurement, held: bool, kind: &str| {
        let safe = !m.too_hot && m.max_vram_gb.map_or(true, |v| hw.vram_gb <= 0.0 || v < hw.vram_gb * 0.97);
        let note = if !held {
            "The game changed these settings back. Not counted.".to_string()
        } else if m.too_hot {
            format!("Stopped early: graphics card reached {:.0} °C", m.max_gpu_temp.unwrap_or(0.0))
        } else if !safe {
            "Used almost all graphics memory (may stutter)".into()
        } else {
            String::new()
        };
        RunRecord { index: i, levels: lv.to_vec(), fps_avg: m.fps_avg, fps_low: m.fps_low, quality: adapter.quality_of(lv), vram_gb: m.max_vram_gb, max_gpu_temp: m.max_gpu_temp, safe, note, settings_held: held, kind: kind.into() }
    };

    for i in 0..prefs.max_tests {
        if state.cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        emit(app, state, |s| {
            s.test_index = i + 1;
            s.phase = "applying".into();
            s.headline = format!("Test {} of up to {}", i + 1, prefs.max_tests);
            s.detail = next_reason.clone();
            s.seconds_done = 0;
        });
        let first = i == 0;
        let (m, held) = measure(&ctx, if first { None } else { Some(&next_levels) }, &format!("Test {}", i + 1))?;
        let rec = record(i + 1, &next_levels, &m, held, "search");

        if held {
            reverted_in_a_row = 0;
            history.push(Observation { levels: rec.levels.clone(), fps_avg: rec.fps_avg, fps_low: rec.fps_low, safe: rec.safe });
        } else {
            reverted_in_a_row += 1;
            if reverted_in_a_row >= 2 {
                return Err("reverted".into());
            }
        }
        runs.push(rec);

        let counted: Vec<&RunRecord> = runs.iter().filter(|r| r.settings_held).collect();
        let baseline_fps = runs.first().map(|r| r.fps_avg);
        let best_fps = counted.iter().filter(|r| r.safe && r.quality >= req.quality_floor - 0.01).map(|r| r.fps_avg).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |x| x.max(b))));
        emit(app, state, |s| {
            s.runs = runs.clone();
            s.baseline_fps = baseline_fps;
            s.best_fps = best_fps;
            s.phase = "thinking".into();
            s.headline = "Choosing what to try next…".into();
            s.detail = "The Smart Tuner is learning how your PC handles this game.".into();
        });

        let sug = engine.suggest(&problem, &history);
        emit(app, state, |s| s.engine = engine.label.clone());
        if sug.stop {
            util::log(format!("tuner stop: {}", sug.reason));
            break;
        }
        next_levels = sug.levels;
        next_reason = if sug.reason.is_empty() { "Testing a promising combination.".into() } else { sug.reason };
        sleep_cancellable(Duration::from_secs(3), state)?;
    }

    // 4. pick the best counted, safe run that meets the quality choice
    let baseline = runs.first().cloned();
    let best = runs
        .iter()
        .filter(|r| r.settings_held && r.safe && r.quality >= req.quality_floor - 0.01)
        .max_by(|a, b| objective(a, target).total_cmp(&objective(b, target)))
        .cloned();

    // 5. confirmation: measure best and original once more
    let mut confirmed = None;
    if let (Some(b0), Some(n0)) = (&baseline, &best) {
        if n0.index != b0.index {
            emit(app, state, |s| {
                s.phase = "checking".into();
                s.headline = "Double-checking the winner".into();
                s.detail = "We measure the best settings and your original settings one more time, so the final numbers are reliable.".into();
            });
            let (mb, _) = measure(&ctx, Some(&n0.levels), "Check 1 of 2 (best settings)")?;
            let rb = record(runs.len() + 1, &n0.levels, &mb, true, "check");
            runs.push(rb.clone());
            let (mo, _) = measure(&ctx, None, "Check 2 of 2 (your original settings)")?;
            let ro = record(runs.len() + 1, &b0.levels, &mo, true, "check");
            runs.push(ro.clone());
            let diff = |a: f64, b: f64| if a > 0.0 && b > 0.0 { ((a - b).abs() / a.max(b)) * 100.0 } else { 0.0 };
            confirmed = Some(Confirmed {
                baseline_fps: (b0.fps_avg + ro.fps_avg) / 2.0,
                baseline_low: (b0.fps_low + ro.fps_low) / 2.0,
                best_fps: (n0.fps_avg + rb.fps_avg) / 2.0,
                best_low: (n0.fps_low + rb.fps_low) / 2.0,
                noise_pct: diff(b0.fps_avg, ro.fps_avg).max(diff(n0.fps_avg, rb.fps_avg)),
            });
            emit(app, state, |s| s.runs = runs.clone());
        }
    }

    let result = SessionResult {
        id: uuid::Uuid::new_v4().to_string(),
        game_id: adapter.id.clone(),
        goal: req.goal.clone(),
        quality_floor: req.quality_floor,
        started_at: state.status.lock().unwrap().started.clone(),
        finished_at: chrono::Local::now().to_rfc3339(),
        engine: engine.label.clone(),
        status: "completed".into(),
        baseline: baseline.clone(),
        best: best.clone().or_else(|| baseline.clone()),
        runs: runs.clone(),
        applied: false,
        confirmed: confirmed.clone(),
    };
    let gain = result.gain_pct().unwrap_or(0.0);
    let noise = confirmed.as_ref().map_or(5.0, |c| c.noise_pct.max(3.0));
    let id = result.id.clone();
    {
        let mut st = state.store.lock().unwrap();
        st.data.results.push(result);
        if st.data.results.len() > 100 {
            st.data.results.remove(0);
        }
        let _ = st.save();
    }
    drop(engine);
    emit(app, state, |s| {
        s.active = false;
        s.phase = "finished".into();
        s.headline = if best.is_none() {
            "Done. None of the tested settings met your picture-quality choice.".into()
        } else if gain > noise {
            format!("Done! {gain:.0}% more frames per second, confirmed.")
        } else {
            "Done. Your current settings are already about as fast as it gets.".into()
        };
        s.detail = "Your original settings are back in place. Review the result and decide whether to use the new settings.".into();
        s.result_id = Some(id);
    });
    Ok(())
}

fn launch(adapter: &Adapter) -> Result<(), String> {
    let Some(id) = adapter.steam_app_id else { return Err("launch".into()) };
    let url = if adapter.launch.steam_args.is_empty() { format!("steam://rungameid/{id}") } else { format!("steam://run/{id}//{}/", adapter.launch.steam_args) };
    util::log(format!("launch {url}"));
    #[cfg(windows)]
    let r = util::hidden_command("cmd").args(["/C", "start", "", &url]).spawn();
    #[cfg(not(windows))]
    let r = util::hidden_command("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|_| "launch".into())
}

fn wait_until(cond: impl Fn() -> bool, max: Duration, state: &AppState) -> Result<(), String> {
    let start = Instant::now();
    while !cond() {
        if state.cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        if start.elapsed() > max {
            return Err("timeout".into());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}

fn sleep_cancellable(d: Duration, state: &AppState) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < d {
        if state.cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_value;
    #[test]
    fn value_compare() {
        assert!(same_value("1", "1.000000"));
        assert!(same_value("\"High\"", "high"));
        assert!(!same_value("2", "3"));
    }
}
