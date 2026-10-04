//! FrameForge desktop app — Tauri v2 backend.
//! Everything runs on the player's PC. There is no server and no account.

mod adapters;
mod benchmark;
mod configio;
mod engine;
mod hardware;
mod session;
mod store;
mod util;

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    pub data_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub log_dir: PathBuf,
    pub resource_dir: PathBuf,
    pub adapters: Vec<adapters::Adapter>,
    pub hardware: hardware::HardwareInfo,
    pub store: Mutex<store::Store>,
    pub status: Mutex<session::SessionStatus>,
    pub cancel: AtomicBool,
    pub player_ready: AtomicBool,
}

#[derive(Serialize)]
struct GameView {
    id: String,
    name: String,
    short: String,
    installed: bool,
    settings_found: bool,
    setting_count: usize,
    tips: Vec<String>,
    where_to_test: String,
    last_result: Option<store::SessionResult>,
    has_backup: bool,
    support: String,
}

#[derive(Serialize)]
struct Bootstrap {
    version: String,
    hardware: hardware::HardwareInfo,
    games: Vec<GameView>,
    preferences: store::Preferences,
    onboarded: bool,
    ai_engine_installed: bool,
    measuring_tool_installed: bool,
    status: session::SessionStatus,
    recovered_session: bool,
}

#[derive(Serialize)]
struct ResultView {
    result: store::SessionResult,
    game_name: String,
    /// One row per setting that differs between "before" and "after".
    changes: Vec<Change>,
    unchanged: usize,
}

#[derive(Serialize)]
struct Change {
    label: String,
    help: String,
    before: String,
    after: String,
}

fn games(state: &AppState) -> Vec<GameView> {
    let steam = adapters::Steam::detect();
    let store = state.store.lock().unwrap();
    state
        .adapters
        .iter()
        .map(|a| {
            let installed = a.steam_app_id.and_then(|id| steam.install_dir(id)).map_or(false, |p| p.exists());
            GameView {
                id: a.id.clone(),
                name: a.name.clone(),
                short: a.short.clone(),
                installed,
                settings_found: adapters::resolve_config(a, &steam).is_some(),
                setting_count: a.settings.len(),
                tips: a.tips.clone(),
                where_to_test: a.benchmark.where_to_test.clone(),
                last_result: store.latest_for(&a.id).cloned(),
                has_backup: configio::oldest(&state.backup_dir, &a.id).is_some(),
                support: a.support.clone(),
            }
        })
        .collect()
}

#[tauri::command]
fn get_bootstrap(state: State<'_, Arc<AppState>>, app: AppHandle) -> Bootstrap {
    let s = state.inner();
    let store = s.store.lock().unwrap();
    let prefs = store.data.preferences.clone();
    let onboarded = store.data.onboarded;
    drop(store);
    Bootstrap {
        version: app.package_info().version.to_string(),
        hardware: s.hardware.clone(),
        games: games(s),
        preferences: prefs,
        onboarded,
        ai_engine_installed: s.resource_dir.join("engine").join(if cfg!(windows) { "frameforge-engine.exe" } else { "frameforge-engine" }).exists(),
        measuring_tool_installed: s.resource_dir.join("tools").join("PresentMon.exe").exists(),
        status: s.status.lock().unwrap().clone(),
        recovered_session: RECOVERED.load(Ordering::SeqCst),
    }
}

#[tauri::command]
fn refresh_games(state: State<'_, Arc<AppState>>) -> Vec<GameView> {
    games(state.inner())
}

#[tauri::command]
fn start_session(app: AppHandle, state: State<'_, Arc<AppState>>, game_id: String, goal: String, quality_floor: f64, refresh_hz: f64) -> Result<(), String> {
    if state.status.lock().unwrap().active {
        return Err("A test session is already running.".into());
    }
    if !["smooth", "balanced", "beautiful"].contains(&goal.as_str()) {
        return Err("Unknown goal.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    {
        let mut s = state.status.lock().unwrap();
        *s = session::SessionStatus { active: true, game_id: game_id.clone(), phase: "preparing".into(), headline: "Getting ready…".into(), ..Default::default() };
    }
    session::spawn(app, session::SessionRequest { game_id, goal, quality_floor: quality_floor.clamp(50.0, 100.0), refresh_hz: refresh_hz.clamp(30.0, 500.0) });
    Ok(())
}

#[tauri::command]
fn player_ready(state: State<'_, Arc<AppState>>) {
    state.player_ready.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn stop_session(state: State<'_, Arc<AppState>>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn session_status(state: State<'_, Arc<AppState>>) -> session::SessionStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
fn get_result(state: State<'_, Arc<AppState>>, id: String) -> Result<ResultView, String> {
    let store = state.store.lock().unwrap();
    let r = store.result(&id).cloned().ok_or("This result no longer exists.")?;
    let a = state.adapters.iter().find(|a| a.id == r.game_id).ok_or("This game is no longer supported.")?;
    let (Some(b), Some(n)) = (&r.baseline, &r.best) else { return Err("This session has no result.".into()) };
    let mut changes = vec![];
    let mut unchanged = 0;
    for (i, s) in a.settings.iter().enumerate() {
        let (lb, ln) = (b.levels.get(i).copied().unwrap_or(0), n.levels.get(i).copied().unwrap_or(0));
        if lb == ln {
            unchanged += 1;
        } else {
            changes.push(Change { label: s.label.clone(), help: s.help.clone(), before: s.options[lb].label.clone(), after: s.options[ln].label.clone() });
        }
    }
    Ok(ResultView { game_name: a.name.clone(), result: r, changes, unchanged })
}

#[tauri::command]
fn history(state: State<'_, Arc<AppState>>) -> Vec<store::SessionResult> {
    let mut v = state.store.lock().unwrap().data.results.clone();
    v.reverse();
    v
}

#[tauri::command]
fn apply_result(state: State<'_, Arc<AppState>>, id: String) -> Result<String, String> {
    if state.status.lock().unwrap().active {
        return Err("Please wait until the current test session ends.".into());
    }
    let mut store = state.store.lock().unwrap();
    let r = store.result(&id).cloned().ok_or("This result no longer exists.")?;
    let a = state.adapters.iter().find(|a| a.id == r.game_id).ok_or("This game is no longer supported.")?;
    let best = r.best.ok_or("Nothing to apply.")?;
    if hardware::is_running(&a.executables) {
        return Err(format!("Please close {} first, then click again.", a.name));
    }
    let cfg = adapters::resolve_config(a, &adapters::Steam::detect()).ok_or("We couldn't find this game's settings file.")?;
    configio::backup(&state.backup_dir, &a.id, &cfg).map_err(|e| { util::log(format!("{e:#}")); "Couldn't save a backup, so nothing was changed.".to_string() })?;
    let values: Vec<(String, String)> = a.settings.iter().zip(&best.levels).map(|(s, &l)| (s.key.clone(), s.options[l].value.clone())).collect();
    configio::write_values(&a.config.format, &cfg, &values).map_err(|e| { util::log(format!("{e:#}")); "Couldn't write the settings file. Is it open in another program?".to_string() })?;
    for x in store.data.results.iter_mut() {
        if x.game_id == r.game_id {
            x.applied = x.id == id;
        }
    }
    let _ = store.save();
    Ok(format!("Done. Start {} and enjoy.", a.name))
}

#[tauri::command]
fn restore_original(state: State<'_, Arc<AppState>>, game_id: String) -> Result<String, String> {
    if state.status.lock().unwrap().active {
        return Err("Please wait until the current test session ends.".into());
    }
    let a = state.adapters.iter().find(|a| a.id == game_id).ok_or("Unknown game.")?;
    if hardware::is_running(&a.executables) {
        return Err(format!("Please close {} first, then click again.", a.name));
    }
    let cfg = adapters::resolve_config(a, &adapters::Steam::detect()).ok_or("We couldn't find this game's settings file.")?;
    let original = configio::oldest(&state.backup_dir, &game_id).ok_or("There is no saved copy for this game yet.")?;
    configio::restore(&original, &cfg).map_err(|e| { util::log(format!("{e:#}")); "Couldn't put the original settings back.".to_string() })?;
    let mut store = state.store.lock().unwrap();
    for x in store.data.results.iter_mut() {
        if x.game_id == game_id {
            x.applied = false;
        }
    }
    let _ = store.save();
    Ok("Your original settings are back.".into())
}

#[tauri::command]
fn save_preferences(state: State<'_, Arc<AppState>>, preferences: store::Preferences) -> Result<store::Preferences, String> {
    let mut store = state.store.lock().unwrap();
    store.data.preferences = preferences.sanitized();
    store.save().map_err(|e| e.to_string())?;
    Ok(store.data.preferences.clone())
}

#[tauri::command]
fn set_onboarded(state: State<'_, Arc<AppState>>) {
    let mut store = state.store.lock().unwrap();
    store.data.onboarded = true;
    let _ = store.save();
}

#[tauri::command]
fn open_folder(state: State<'_, Arc<AppState>>, which: String) -> Result<(), String> {
    let dir = match which.as_str() {
        "logs" => &state.log_dir,
        "backups" => &state.backup_dir,
        _ => &state.data_dir,
    };
    let _ = std::fs::create_dir_all(dir);
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(dir).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("xdg-open").arg(dir).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_history(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut store = state.store.lock().unwrap();
    store.data.results.clear();
    store.save().map_err(|e| e.to_string())
}

static RECOVERED: AtomicBool = AtomicBool::new(false);

pub fn run() {
    use tauri_plugin_global_shortcut::{Code, Shortcut, ShortcutState};
    let f9 = Shortcut::new(None, Code::F9);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut == &f9 && event.state() == ShortcutState::Pressed {
                        if let Some(st) = app.try_state::<Arc<AppState>>() {
                            st.player_ready.store(true, Ordering::SeqCst);
                            let _ = app.emit("hotkey", "F9");
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            let log_dir = data_dir.join("logs");
            let backup_dir = data_dir.join("backups");
            std::fs::create_dir_all(&backup_dir)?;
            util::init_log(&log_dir);
            util::log(format!("FrameForge {} starting", app.package_info().version));

            let resource_dir = app.path().resource_dir()?.join("resources");
            let adapters = adapters::load_all(&resource_dir.join("adapters"));
            let hardware = hardware::detect();
            let store = store::Store::load(&data_dir);

            let state = Arc::new(AppState {
                data_dir,
                backup_dir,
                log_dir,
                resource_dir,
                adapters,
                hardware,
                store: Mutex::new(store),
                status: Mutex::new(session::SessionStatus::default()),
                cancel: AtomicBool::new(false),
                player_ready: AtomicBool::new(false),
            });

            // Crash recovery: if the app was closed mid-session, put the original settings back.
            if state.store.lock().unwrap().data.active_backup.is_some() {
                util::log("previous session did not finish - restoring original settings");
                session::restore_active(&state);
                RECOVERED.store(true, Ordering::SeqCst);
            }

            app.manage(state);
            // F9 is only registered while we are waiting for the player (see session.rs),
            // so FrameForge never steals F9 from games (many use it for quick-save).
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(st) = window.app_handle().try_state::<Arc<AppState>>() {
                    st.cancel.store(true, Ordering::SeqCst);
                    if st.status.lock().unwrap().active {
                        session::restore_active(&st);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            refresh_games,
            start_session,
            player_ready,
            stop_session,
            session_status,
            get_result,
            history,
            apply_result,
            restore_original,
            save_preferences,
            set_onboarded,
            open_folder,
            delete_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running FrameForge");
}
