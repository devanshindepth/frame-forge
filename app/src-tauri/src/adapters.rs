//! Game adapters: one JSON file per game in `resources/adapters/`.
//! An adapter says where the game's settings file is, how to start the game,
//! and — for every graphics setting — its options, a plain-language explanation,
//! a relative performance cost and a visual-impact score.

use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Adapter {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    #[serde(default)]
    pub executables: Vec<String>,
    pub config: ConfigSpec,
    #[serde(default)]
    pub launch: LaunchSpec,
    #[serde(default)]
    pub benchmark: BenchSpec,
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub tips: Vec<String>,
    /// "verified" once a release-candidate run on real hardware confirmed every key
    /// is read and honoured by the game; "unverified" until then (shown in the UI).
    #[serde(default = "unverified")]
    pub support: String,
}

fn unverified() -> String {
    "unverified".into()
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ConfigSpec {
    /// "ini" | "kv" (Valve quoted key/value) | "json"
    pub format: String,
    /// Candidate paths with placeholders, first existing wins.
    pub paths: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LaunchSpec {
    #[serde(default)]
    pub steam_args: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct BenchSpec {
    /// "guided" = player goes to a spot and presses F9; "auto" = built-in benchmark.
    pub mode: String,
    pub warmup_s: u64,
    #[serde(default)]
    pub where_to_test: String,
}

impl Default for BenchSpec {
    fn default() -> Self {
        Self { mode: "guided".into(), warmup_s: 5, where_to_test: String::new() }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Setting {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
    pub options: Vec<SettingOption>,
    /// Relative graphics-card load per option (0 = free).
    pub cost: Vec<f64>,
    /// How much each option contributes to how good the game looks.
    pub quality: Vec<f64>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SettingOption {
    pub label: String,
    pub value: String,
}

impl Adapter {
    pub fn validate(&self) -> Result<(), String> {
        if self.settings.is_empty() {
            return Err("no settings".into());
        }
        for s in &self.settings {
            let n = s.options.len();
            if n < 2 || s.cost.len() != n || s.quality.len() != n {
                return Err(format!("setting {} has mismatched options/cost/quality", s.key));
            }
        }
        if !["ini", "kv", "json"].contains(&self.config.format.as_str()) {
            return Err(format!("unknown config format {}", self.config.format));
        }
        Ok(())
    }

    pub fn space(&self) -> Vec<usize> {
        self.settings.iter().map(|s| s.options.len()).collect()
    }

    pub fn quality_weights(&self) -> Vec<Vec<f64>> {
        self.settings.iter().map(|s| s.quality.clone()).collect()
    }

    pub fn cost_weights(&self) -> Vec<Vec<f64>> {
        self.settings.iter().map(|s| s.cost.clone()).collect()
    }

    /// Visual quality 0–100 for a configuration (same formula as the engine).
    pub fn quality_of(&self, levels: &[usize]) -> f64 {
        let max: f64 = self.settings.iter().map(|s| s.quality.iter().cloned().fold(0.0, f64::max)).sum();
        let got: f64 = self.settings.iter().zip(levels).map(|(s, &l)| s.quality.get(l).copied().unwrap_or(0.0)).sum();
        if max <= 0.0 { 100.0 } else { (got / max * 100.0).clamp(0.0, 100.0) }
    }

    pub fn default_levels(&self) -> Vec<usize> {
        self.settings.iter().map(|s| s.options.len() / 2).collect()
    }
}

pub fn load_all(dir: &Path) -> Vec<Adapter> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        util::log(format!("adapter folder missing: {}", dir.display()));
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Adapter>(&t).map_err(|e| e.to_string())) {
            Ok(a) => match a.validate() {
                Ok(()) => out.push(a),
                Err(e) => util::log(format!("adapter {} rejected: {e}", p.display())),
            },
            Err(e) => util::log(format!("adapter {} unreadable: {e}", p.display())),
        }
    }
    discover_unreal_engine_games(&mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    util::log(format!("loaded {} adapters (including auto-discovered)", out.len()));
    out
}

// ---------------------------------------------------------------- Generic Unreal Engine detection

pub fn ue_scalability_settings() -> Vec<Setting> {
    let opts = vec![
        SettingOption { label: "Low".into(), value: "0".into() },
        SettingOption { label: "Medium".into(), value: "1".into() },
        SettingOption { label: "High".into(), value: "2".into() },
        SettingOption { label: "Epic / Very High".into(), value: "3".into() },
        SettingOption { label: "Cinematic".into(), value: "4".into() },
    ];
    vec![
        Setting {
            key: "ScalabilityGroups/sg.ViewDistanceQuality".into(),
            label: "View distance".into(),
            help: "How far away objects and terrain are rendered in full detail.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.04, 0.08, 0.12, 0.16],
            quality: vec![2.0, 4.0, 6.0, 8.0, 9.0],
        },
        Setting {
            key: "ScalabilityGroups/sg.ShadowQuality".into(),
            label: "Shadows".into(),
            help: "Sharpness and detail of shadows. Usually a major performance cost.".into(),
            options: opts.clone(),
            cost: vec![0.02, 0.08, 0.15, 0.24, 0.35],
            quality: vec![2.0, 5.0, 7.0, 8.5, 9.5],
        },
        Setting {
            key: "ScalabilityGroups/sg.GlobalIlluminationQuality".into(),
            label: "Indirect lighting (Lumen)".into(),
            help: "Light bouncing off surfaces and ambient lighting quality.".into(),
            options: opts.clone(),
            cost: vec![0.02, 0.07, 0.14, 0.22, 0.30],
            quality: vec![2.0, 5.0, 7.0, 8.5, 9.5],
        },
        Setting {
            key: "ScalabilityGroups/sg.ReflectionQuality".into(),
            label: "Reflections".into(),
            help: "Quality of reflections on shiny surfaces and water.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.05, 0.10, 0.16, 0.22],
            quality: vec![2.0, 4.0, 6.0, 7.5, 8.5],
        },
        Setting {
            key: "ScalabilityGroups/sg.FoliageQuality".into(),
            label: "Grass & plants".into(),
            help: "Density and draw distance of grass and vegetation.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.06, 0.12, 0.18, 0.25],
            quality: vec![2.0, 4.5, 7.0, 8.5, 9.0],
        },
        Setting {
            key: "ScalabilityGroups/sg.PostProcessQuality".into(),
            label: "Post-processing".into(),
            help: "Camera bloom, motion blur, and screen effects.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.03, 0.06, 0.09, 0.12],
            quality: vec![2.0, 3.5, 4.5, 5.0, 5.5],
        },
        Setting {
            key: "ScalabilityGroups/sg.TextureQuality".into(),
            label: "Texture detail".into(),
            help: "Sharpness and resolution of surfaces. Uses graphics memory.".into(),
            options: opts.clone(),
            cost: vec![0.00, 0.01, 0.02, 0.03, 0.05],
            quality: vec![2.0, 5.0, 7.5, 9.0, 10.0],
        },
        Setting {
            key: "ScalabilityGroups/sg.EffectsQuality".into(),
            label: "Effects".into(),
            help: "Detail of smoke, fire, sparks, and spell particles.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.04, 0.07, 0.11, 0.15],
            quality: vec![2.0, 4.0, 5.5, 7.0, 8.0],
        },
        Setting {
            key: "ScalabilityGroups/sg.ShadingQuality".into(),
            label: "Surface lighting".into(),
            help: "Detail of light on materials like skin, fur and metal.".into(),
            options: opts.clone(),
            cost: vec![0.01, 0.03, 0.06, 0.09, 0.12],
            quality: vec![2.0, 4.0, 5.0, 6.0, 7.0],
        },
        Setting {
            key: "ScalabilityGroups/sg.AntiAliasingQuality".into(),
            label: "Edge smoothing".into(),
            help: "Removes jagged and shimmering edges.".into(),
            options: opts.clone(),
            cost: vec![0.00, 0.02, 0.04, 0.06, 0.08],
            quality: vec![2.0, 4.0, 5.0, 5.5, 6.0],
        },
    ]
}

fn has_shipping_exe(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&p) {
                for se in sub.flatten() {
                    let sp = se.path();
                    if sp.is_file() && sp.to_string_lossy().ends_with("-Win64-Shipping.exe") {
                        return true;
                    }
                    if sp.is_dir() {
                        if let Ok(sub2) = std::fs::read_dir(&sp) {
                            for s2e in sub2.flatten() {
                                let s2p = s2e.path();
                                if s2p.is_file() && s2p.to_string_lossy().ends_with("-Win64-Shipping.exe") {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

fn find_ue_executables(dir: &Path, default_name: &str) -> Vec<String> {
    let mut exes = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return exes };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() && p.extension().and_then(|x| x.to_str()) == Some("exe") {
            if let Some(name) = p.file_name().and_then(|x| x.to_str()) {
                exes.push(name.to_string());
            }
        } else if p.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&p) {
                for se in sub.flatten() {
                    let sp = se.path();
                    if sp.is_dir() {
                        if let Ok(sub2) = std::fs::read_dir(&sp) {
                            for s2e in sub2.flatten() {
                                let s2p = s2e.path();
                                if let Some(name) = s2p.file_name().and_then(|x| x.to_str()) {
                                    if name.ends_with("-Win64-Shipping.exe") {
                                        exes.push(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let fallback = format!("{default_name}.exe");
    if !exes.iter().any(|x| x.eq_ignore_ascii_case(&fallback)) {
        exes.push(fallback);
    }
    exes.sort();
    exes.dedup();
    exes
}

pub fn discover_unreal_engine_games(existing: &mut Vec<Adapter>) {
    let steam = Steam::detect();
    for lib in &steam.libraries {
        let steamapps = lib.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else { continue };
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let mut appid = None;
            let mut app_name = String::new();
            let mut installdir = String::new();
            for line in text.lines() {
                let t = util::quoted_tokens(line);
                if t.len() == 2 {
                    match t[0].as_str() {
                        "appid" => appid = t[1].parse::<u32>().ok(),
                        "name" => app_name = t[1].clone(),
                        "installdir" => installdir = t[1].clone(),
                        _ => {}
                    }
                }
            }

            let Some(id) = appid else { continue };
            if existing.iter().any(|a| a.steam_app_id == Some(id)) {
                continue;
            }
            if installdir.is_empty() {
                continue;
            }

            let install_path = steamapps.join("common").join(&installdir);
            if !install_path.exists() {
                continue;
            }

            let is_ue = install_path.join("Engine").is_dir() || has_shipping_exe(&install_path);
            if !is_ue {
                continue;
            }

            let executables = find_ue_executables(&install_path, &installdir);
            let mut paths = vec![
                format!("{{INSTALL}}\\{installdir}\\Saved\\Config\\Windows\\GameUserSettings.ini"),
                format!("{{INSTALL}}\\Saved\\Config\\Windows\\GameUserSettings.ini"),
                format!("{{LOCALAPPDATA}}\\{installdir}\\Saved\\Config\\Windows\\GameUserSettings.ini"),
                format!("{{LOCALAPPDATA}}\\{installdir}\\Saved\\Config\\WindowsNoEditor\\GameUserSettings.ini"),
            ];
            for exe in &executables {
                if let Some(prefix) = exe.strip_suffix("-Win64-Shipping.exe") {
                    paths.push(format!("{{INSTALL}}\\{prefix}\\Saved\\Config\\Windows\\GameUserSettings.ini"));
                    paths.push(format!("{{INSTALL}}\\{prefix}\\Saved\\Config\\WindowsNoEditor\\GameUserSettings.ini"));
                    paths.push(format!("{{LOCALAPPDATA}}\\{prefix}\\Saved\\Config\\Windows\\GameUserSettings.ini"));
                    paths.push(format!("{{LOCALAPPDATA}}\\{prefix}\\Saved\\Config\\WindowsNoEditor\\GameUserSettings.ini"));
                }
            }
            paths.dedup();

            let adapter = Adapter {
                id: format!("ue_{id}"),
                name: if app_name.is_empty() { installdir.clone() } else { app_name },
                short: "Unreal Engine game (auto-detected)".into(),
                steam_app_id: Some(id),
                executables,
                config: ConfigSpec {
                    format: "ini".into(),
                    paths,
                },
                launch: LaunchSpec::default(),
                benchmark: BenchSpec {
                    mode: "guided".into(),
                    warmup_s: 5,
                    where_to_test: "Go to a gameplay or benchmark spot and press F9.".into(),
                },
                settings: ue_scalability_settings(),
                tips: vec![
                    "Detected as an Unreal Engine game. Standard Scalability settings apply.".into(),
                    "Keep your testing route consistent between tests for accurate measurements.".into(),
                ],
                support: "auto-detected".into(),
            };

            if let Ok(()) = adapter.validate() {
                util::log(format!("auto-discovered Unreal Engine game: {} (AppID {id})", adapter.name));
                existing.push(adapter);
            }
        }
    }
}

// ---------------------------------------------------------------- Steam detection

#[derive(Clone, Debug, Default)]
pub struct Steam {
    pub root: Option<PathBuf>,
    pub libraries: Vec<PathBuf>,
}

impl Steam {
    pub fn detect() -> Self {
        let root = steam_root();
        let mut libraries = Vec::new();
        if let Some(r) = &root {
            libraries.push(r.clone());
            let vdf = r.join("steamapps").join("libraryfolders.vdf");
            if let Ok(text) = std::fs::read_to_string(vdf) {
                for line in text.lines() {
                    let t = util::quoted_tokens(line);
                    if t.len() == 2 && t[0] == "path" {
                        let p = PathBuf::from(t[1].replace("\\\\", "\\"));
                        if !libraries.contains(&p) {
                            libraries.push(p);
                        }
                    }
                }
            }
        }
        Self { root, libraries }
    }

    /// Install folder of an app, if installed.
    pub fn install_dir(&self, app_id: u32) -> Option<PathBuf> {
        for lib in &self.libraries {
            let acf = lib.join("steamapps").join(format!("appmanifest_{app_id}.acf"));
            if let Ok(text) = std::fs::read_to_string(&acf) {
                for line in text.lines() {
                    let t = util::quoted_tokens(line);
                    if t.len() == 2 && t[0] == "installdir" {
                        return Some(lib.join("steamapps").join("common").join(&t[1]));
                    }
                }
            }
        }
        None
    }
}

fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam") {
            if let Ok(p) = k.get_value::<String, _>("SteamPath") {
                let pb = PathBuf::from(p.replace('/', "\\"));
                if pb.exists() {
                    return Some(pb);
                }
            }
        }
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\WOW6432Node\Valve\Steam") {
            if let Ok(p) = k.get_value::<String, _>("InstallPath") {
                let pb = PathBuf::from(p);
                if pb.exists() {
                    return Some(pb);
                }
            }
        }
    }
    let fallback = PathBuf::from(r"C:\Program Files (x86)\Steam");
    if fallback.exists() { Some(fallback) } else { None }
}

// ---------------------------------------------------------------- config path resolution

/// Expand placeholders and return every existing candidate file, newest first.
pub fn resolve_config(adapter: &Adapter, steam: &Steam) -> Option<PathBuf> {
    let install = adapter.steam_app_id.and_then(|id| steam.install_dir(id));
    let mut found: Vec<PathBuf> = Vec::new();
    for raw in &adapter.config.paths {
        for p in expand(raw, steam, install.as_deref()) {
            if p.is_file() {
                found.push(p);
            }
        }
    }
    found.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    found.into_iter().next()
}

fn expand(raw: &str, steam: &Steam, install: Option<&Path>) -> Vec<PathBuf> {
    let mut s = raw.to_string();
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    s = s.replace("{LOCALAPPDATA}", &env("LOCALAPPDATA"));
    s = s.replace("{APPDATA}", &env("APPDATA"));
    s = s.replace("{USERPROFILE}", &env("USERPROFILE"));
    let docs = dirs::document_dir().map(|d| d.display().to_string()).unwrap_or_default();
    s = s.replace("{DOCUMENTS}", &docs);
    if s.contains("{INSTALL}") {
        let Some(i) = install else { return vec![] };
        s = s.replace("{INSTALL}", &i.display().to_string());
    }
    if s.contains("{STEAM}") {
        let Some(r) = &steam.root else { return vec![] };
        s = s.replace("{STEAM}", &r.display().to_string());
    }
    if let Some(idx) = s.find("{STEAM_USER}") {
        // expand to every Steam user folder: {STEAM}/userdata/<id>/...
        let (before, after) = s.split_at(idx);
        let after = &after["{STEAM_USER}".len()..];
        let base = PathBuf::from(before);
        let Ok(rd) = std::fs::read_dir(&base) else { return vec![] };
        return rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| PathBuf::from(format!("{}{}", e.path().display(), after)))
            .collect();
    }
    vec![PathBuf::from(s)]
}
