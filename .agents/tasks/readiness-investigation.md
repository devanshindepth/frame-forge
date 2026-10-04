# FrameForge — Readiness Investigation

**Prepared by:** Investigation Agent  
**Date:** 2025  
**Verdict:** ⚠️ **Alpha — not ready for end users without additional build steps**

---

## Summary Answer

FrameForge is a well-architected, feature-complete Tauri v2 desktop application for Windows that automatically optimises game graphics settings using an offline AI engine (TabPFN). The codebase is solid and the CI pipeline is thorough. However, the project **cannot be installed or run by an end user right now** because two runtime dependencies — the frozen AI engine binary and the PresentMon frame-capture tool — are **absent from the repo** and must be built/downloaded before packaging. Once those are in place, the release build would be straightforwardly achievable. Additionally, only two game adapters (CS2 and Palworld) exist, limiting its practical appeal at launch.

---

## 1. What Does the Project Do?

FrameForge is a Windows desktop app (Tauri v2 / Rust + Python + HTML/CSS/JS) that:

1. Detects the user's GPU, RAM, and CPU via registry / `nvidia-smi` / `sysinfo`.
2. Reads a game's graphics-settings file (INI, KV, or JSON formats supported via game *adapters*).
3. Runs a constrained Bayesian optimisation loop: it sets different graphics configurations, launches the game, and uses **PresentMon** (a frame-timing tool, MIT licence) to measure FPS while the player plays at a test spot.
4. Uses **TabPFN** (a pre-trained in-context regressor, frozen offline into an `.exe` sidecar) to predict which untested configuration is most likely to beat the current best.
5. After several tests it applies the winning settings (or reverts to originals if cancelled).

Everything is fully offline and local — no account, no cloud, no telemetry.

There is also a **separate static website** (`index.html` / `roadmap.html` at the repo root) that describes the concept with interactive stress-test charts and a simulator. These are a pre-launch concept/marketing page, not the desktop app.

---

## 2. Tech Stack

| Layer | Technology |
|---|---|
| Desktop shell | Tauri v2 (Rust, Tauri v2 API) |
| Backend / business logic | Rust 1.77+ (`src-tauri/src/`) |
| AI engine sidecar | Python 3.11 + TabPFN 2.0.9 + PyTorch (CPU), frozen with PyInstaller |
| Frame-time capture | PresentMon 2.x (external MIT binary, downloaded at build time) |
| Frontend UI | Vanilla HTML/CSS/JS (`app/ui/`) — no framework |
| Installer | NSIS + MSI via Tauri bundle |
| CI/CD | GitHub Actions (`release.yml`) on `windows-latest` |
| Game-adapter data | JSON files in `resources/adapters/` |
| Persistence | JSON file in `%LOCALAPPDATA%\FrameForge\` |

---

## 3. Build System

There is **no `package.json`** — the frontend is static HTML/JS with no npm dependencies. The entire build is driven by Cargo/Tauri.

### Full release build (as performed by CI — `release.yml`)

```
# Step 1: Build the frozen offline AI engine (do this on the release machine)
pip install -r app/engine/requirements.txt huggingface_hub
python app/engine/build_engine.py
# → writes app/src-tauri/resources/engine/frameforge-engine.exe + _internal/ + models/

# Step 2: Download PresentMon (CI fetches latest release automatically)
# Manually: download PresentMon-2.x-x64.exe from
#   https://github.com/GameTechDev/PresentMon/releases
# Rename it to:
#   app/src-tauri/resources/tools/PresentMon.exe

# Step 3: Generate icons from SVG source
cd app/src-tauri
cargo tauri icon icons/source.svg
# → produces icons/32x32.png, icons/128x128.png, icons/128x128@2x.png, icons/icon.ico

# Step 4: Build the installer
cargo tauri build
# → produces app/src-tauri/target/release/bundle/nsis/*.exe
#            app/src-tauri/target/release/bundle/msi/*.msi
```

### For a dev/debug build (no installer needed)

```
# Prerequisites: same as above (engine binary and PresentMon must exist)
cd app/src-tauri
cargo tauri dev
```

### Rust unit tests only

```
cd app/src-tauri
cargo test --release
```

---

## 4. Missing Pieces That Block a Release

### BLOCKING — must be resolved before any end-user installer can be produced

| # | Issue | Location | Details |
|---|---|---|---|
| **B1** | AI engine binary absent | `resources/engine/` | Directory contains only `README.txt`. The `frameforge-engine.exe` + Python runtime + model weights (~400–800 MB) are built by `python app/engine/build_engine.py`. The app falls back to a "Basic tuner" (pure Rust) if missing, so it still *runs*, but the Smart Tuner (the main value proposition) is unavailable. |
| **B2** | PresentMon.exe absent | `resources/tools/` | Directory contains only `README.txt`. Without this binary `benchmark::presentmon_ready()` returns `false` and every session immediately fails with error code `"presentmon"`. The app cannot measure FPS at all. This is a **hard blocker**: no PresentMon = no sessions. |
| **B3** | Icons not generated | `icons/` | Only `source.svg` is present. The four icon files referenced in `tauri.conf.json` (`32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.ico`) do not exist. `cargo tauri build` will fail without them. Run `cargo tauri icon icons/source.svg` to generate them. |

### NOT BLOCKING but important limitations

| # | Issue | Details |
|---|---|---|
| **L1** | Only 2 game adapters | `resources/adapters/` has `cs2.json` and `palworld.json`. Both are marked `"support": "unverified"` — the code shows `unverified` as the default when the field is absent or explicitly set. This means neither adapter has had real-hardware confirmation that setting keys round-trip correctly. |
| **L2** | Benchmark harness stress-test data are synthetic | `benchmark/stress_test.py` and the website's `js/stress-data.js` use a toy simulator, not real-hardware data. The README explicitly labels them "design-phase reference numbers". T8 (constraint violations) is not implemented because it requires real hardware. |
| **L3** | GPU temperature guard is NVIDIA-only | `hardware.rs`: `live_sensors` is set `true` only when `nvidia-smi` is available and reports a temperature. AMD and Intel GPU users get no temperature protection. |
| **L4** | Live-sensor polling uses `nvidia-smi` subprocess | `hardware.rs::sample_gpu()` spawns `nvidia-smi` every ~2 s during a benchmark. This is fragile compared to a proper WMI or NVML binding, but functional. |
| **L5** | Windows-only | `tauri.conf.json` targets `nsis` and `msi` only. Linux/macOS builds are not configured and several Rust paths are `#[cfg(windows)]`-only (registry, PresentMon, `explorer.exe`). |
| **L6** | NSIS installer hook encodes UTF-8 ellipsis literally | `installer-hooks.nsh` line: `DetailPrint "Allowing FrameForge to count frames (Performance Log Users)â€¦"` — the ellipsis `…` is misencoded as mojibake. This is cosmetic (installer still works) but visible during installation. |

---

## 5. Existing Tests

### Rust unit tests (in-codebase)

Two `#[cfg(test)]` modules exist:

- **`src/benchmark.rs`** — `parses_v1_csv()`: builds a synthetic PresentMon CSV with 500 rows and asserts avg FPS ~95 and 1% low < 40. Exercises `parse_frametimes()` and `summarize()`. Run with: `cargo test --release` from `app/src-tauri/`.
- **`src/session.rs`** — `value_compare()`: verifies `same_value()` handles numeric string equivalence (`"1"` == `"1.000000"`) and case-insensitive string matching. Small but important correctness test.

No integration tests or end-to-end tests exist in the Rust codebase.

### Python benchmark harness (`benchmark/stress_test.py`)

This is not a unit-test suite — it is a stress-test / benchmarking harness that compares surrogate models (TabPFN, GP, XGBoost, CatBoost, kNN, Ridge, TPE, CMA-ES, etc.) across 10 scenarios (T1–T10) using a **synthetic** frame-time oracle.

Run it with:
```
pip install tabpfn scikit-learn xgboost catboost optuna scipy numpy pandas
python benchmark/stress_test.py --quick     # fast smoke run (~1–2 minutes)
python benchmark/stress_test.py --seeds 10  # full run (slow)
```

T8 (constraint violations on re-run) is explicitly listed as **not implemented** because it requires real hardware.

---

## 6. How to Run or Test the App Locally

### Prerequisites

- Windows 10/11 (the app is Windows-only)
- Rust toolchain (`rustup` with stable channel, Rust ≥ 1.77)
- Tauri CLI v2: `cargo install tauri-cli --version "^2" --locked`
- Python 3.11 (for the AI engine build)

### Step-by-step

```powershell
# 1. Clone the repo
git clone <repo-url>
cd frame-drop

# 2. Build the AI engine (one-time, takes several minutes — downloads ~500 MB)
python -m venv .venv
.venv\Scripts\activate
pip install -r app/engine/requirements.txt huggingface_hub
python app/engine/build_engine.py
# → app/src-tauri/resources/engine/ is now populated

# 3. Place PresentMon
# Download PresentMon-2.x-x64.exe from:
#   https://github.com/GameTechDev/PresentMon/releases
# Rename it and copy:
Copy-Item PresentMon-2.x-x64.exe app\src-tauri\resources\tools\PresentMon.exe

# 4. Generate icons
cd app/src-tauri
cargo tauri icon icons/source.svg

# 5a. Run in dev mode (no installer, hot-reload-ish)
cargo tauri dev

# 5b. OR build a release installer
cargo tauri build
# Installer: app/src-tauri/target/release/bundle/nsis/FrameForge_1.0.0_x64-setup.exe
```

### Running only Rust unit tests (no engine/PresentMon needed)

```powershell
cd app/src-tauri
cargo test --release
```

### Previewing the UI without the Tauri backend

Open any of the HTML files in `app/preview/` directly in a browser:
- `app/preview/home.html` — game library view
- `app/preview/live.html` — active session view
- `app/preview/result.html` — results view
- `app/preview/setup.html` — onboarding

These use `app/ui/mock.js` to stand in for the Rust backend, so the full UI can be explored without building the app.

### Viewing the concept/marketing site

Open `index.html` (repo root) in a browser — no build step needed. All data is in `js/stress-data.js`. To replace with measured data:
```
python benchmark/stress_test.py --seeds 10 --out js/stress-data.js
```

---

## 7. CI/CD Pipeline

`release.yml` (`.github/workflows/release.yml`) triggers on:
- Any push matching tag `v*`
- Manual `workflow_dispatch`

It runs on `windows-latest` and performs:

1. Checks out the code
2. Installs Rust stable
3. Runs `cargo test --release`
4. Installs Python 3.11 and builds the frozen AI engine
5. Downloads the latest PresentMon release from GitHub via `gh api`
6. Installs Tauri CLI
7. Generates icons from `icons/source.svg`
8. Runs `cargo tauri build`
9. Uploads NSIS `.exe` and MSI `.msi` as build artifacts

The pipeline is well-structured and covers all the blocking gaps automatically. It has one minor robustness issue: the PresentMon step verifies the downloaded binary by checking its exit from `--help`, but does not pin to a specific PresentMon version, so a breaking API change in a future PresentMon release could silently break sessions without a build failure.

---

## 8. Resource Directory Status

| Directory | Status | Contents |
|---|---|---|
| `resources/engine/` | **Empty (placeholder)** | Only `README.txt` explaining that `python app/engine/build_engine.py` populates it |
| `resources/tools/` | **Empty (placeholder)** | Only `README.txt` explaining where to download PresentMon |
| `resources/adapters/` | **Populated** | `cs2.json` (Counter-Strike 2) and `palworld.json` — fully structured with settings, costs, quality weights, config paths, and launch instructions |

---

## 9. Preview HTML Files

`app/preview/*.html` are **developer UI previews**, not end-user pages. They load `../ui/app.css`, `../ui/mock.js`, and `../ui/ui.js`. The `mock.js` file provides a browser-compatible stand-in for `window.__TAURI__` invoke calls, returning hard-coded fixture data so the UI renders and is interactive without the Rust backend.

- `home.html` — game library with mock game cards (CS2, Palworld)
- `live.html` — the in-session "Testing now" screen with progress
- `result.html` — post-session result card with FPS gain display
- `setup.html` — first-run onboarding flow

These are useful for: UI development, visual QA, and demonstrating the app without a full build.

---

## 10. Overall Readiness Assessment

**Alpha** — the engineering is production-quality but the product is not ready for end users.

### What is solid

- The Rust backend is complete, well-structured, and defensively coded (crash recovery, settings backup, adapter mismatch detection, temperature guard, configurable max tests).
- The session flow is thorough: backs up settings before touching them, verifies the game didn't revert the written values, does a confirmation double-measurement at the end, and always restores originals on cancel/crash.
- The AI engine (Python/TabPFN) is fully implemented with a sensible fallback (pure Rust "Basic tuner") when the engine is missing.
- The CI pipeline covers the full end-to-end build.
- The NSIS installer correctly adds the user to "Performance Log Users" for PresentMon ETW access.
- The UI has proper mock infrastructure for development previews.

### What blocks end-user readiness

1. **PresentMon missing** — the single hardest blocker. Without it no session can start.
2. **AI engine binary missing** — the Smart Tuner won't work; fallback to Basic tuner is functional but less capable.
3. **Icons not generated** — blocks `cargo tauri build` from completing.
4. **Only 2 unverified adapters** — a very limited game library for a v1.0 release.
5. **Stress-test data is synthetic** — the go/no-go criterion from the README (TabPFN must beat GP-BO on T1/T2/T3 with non-overlapping confidence intervals on real hardware) has not been met.

### Recommendations

| Priority | Action |
|---|---|
| **P0** | Run the CI pipeline on a tag (or `workflow_dispatch`) — this resolves blockers B1, B2, B3 automatically and produces a real installer. |
| **P0** | Verify both adapters (CS2, Palworld) on real hardware and set `"support": "verified"` once confirmed. |
| **P1** | Run `benchmark/stress_test.py` with real hardware data (`--data sweeps.csv`) and publish measured stress-test numbers to `js/stress-data.js`. |
| **P1** | Add AMD and Intel GPU temperature monitoring (e.g. via WMI or LibreHardwareMonitor interop) to remove the NVIDIA-only limitation. |
| **P1** | Fix the mojibake in `installer-hooks.nsh` (save as UTF-8 BOM or replace `…` with `...`). |
| **P2** | Add more game adapters. Two games is a very narrow appeal for a public release. |
| **P2** | Pin PresentMon to a specific tested release in the CI workflow to prevent silent breakage from upstream API changes. |
| **P2** | Add T8 (constraint-violation re-runs) to the benchmark harness once real hardware oracle data is available. |
