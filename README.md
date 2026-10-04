# FrameForge — Adaptive Game Graphics Optimizer

> **Stop guessing graphics settings. Measure them.**  
> FrameForge is a free Windows desktop application that measures your real frame rate while you play, tests graphics configurations automatically, and finds the fastest settings that preserve your chosen visual quality. Runs 100% offline on your own PC.

---

## 🚀 Downloads (v1.0.0)

| Package | Format | Architecture | Download Link |
|---|---|---|---|
| **Windows Setup (Recommended)** | `.exe` (NSIS) | x86_64 | [FrameForge_1.0.0_x64-setup.exe](https://github.com/devanshindepth/frame-forge/releases/download/v1.0.0/FrameForge_1.0.0_x64-setup.exe) |
| **Windows Package** | `.msi` (WiX) | x86_64 | [FrameForge_1.0.0_x64_en-US.msi](https://github.com/devanshindepth/frame-forge/releases/download/v1.0.0/FrameForge_1.0.0_x64_en-US.msi) |

### Integrity Verification (SHA-256)

```text
# FrameForge_1.0.0_x64-setup.exe
71eeb4839a038c444b22ffcf06aa1d669c150226c5bee78479818a4cf01cfb18

# FrameForge_1.0.0_x64_en-US.msi
152695991ae4c052314779aff6bdd5625e5030416a0beee620acb3b34070930a
```

To verify on Windows PowerShell:
```powershell
Get-FileHash -Algorithm SHA256 "path\to\FrameForge_1.0.0_x64-setup.exe"
```

---

## ✨ Features

- **Real Frame-Rate Measurement**: Measures average FPS and 1% lows using Intel PresentMon via Windows Event Tracing (ETW) — never injects into game memory.
- **Safety First**: Your game's configuration file is backed up before any test begins, and restored automatically if stopped, interrupted, or on application restart.
- **Applied Verification**: Confirms that the game actually accepted and wrote modified settings after each run.
- **NVIDIA Temperature Guard**: Real-time GPU temperature telemetry with automatic stop threshold.
- **100% Local & Private**: No account, no cloud servers, no analytics, no data uploaded.
- **Dual Confirmation**: Final settings and baseline are tested twice to ensure gains are statistically real.

---

## 💻 System Requirements

- **OS**: Windows 10 or Windows 11 (64-bit)
- **Supported Games**: Steam versions of Counter-Strike 2, Palworld (more supported via JSON adapters)
- **Permissions**: Windows "Performance Log Users" group (auto-configured by the installer; requires one PC restart)
- **GPU**: NVIDIA, AMD, or Intel discrete or integrated graphics

---

## 🛠️ Architecture & Tech Stack

| Layer | Component | Notes |
|---|---|---|
| **Shell** | Tauri v2 (Rust) | Native Windows desktop integration, IPC communication |
| **Frontend** | Vanilla HTML / CSS / JS | Zero-runtime UI running inside WebView2 |
| **Frame Capture** | Intel PresentMon (MIT) | Event Tracing for Windows (ETW) frame timing |
| **Smart Tuner** | TabPFN & In-Context Optimizer | Frozen Python sidecar engine for sample-efficient optimization |
| **Adapters** | JSON Schemas (`resources/adapters/`) | Declarative game settings paths and options |

---

## 🏗️ Building from Source

### Prerequisites

1. **Rust**: Stable toolchain (1.77+) with `cargo`
2. **Node / Tauri CLI**: `cargo install tauri-cli --version "^2" --locked`
3. **Python**: Python 3.11 with PyTorch for the engine sidecar
4. **PresentMon**: Console x64 binary placed in `app/src-tauri/resources/tools/PresentMon.exe`

### Build Steps

```powershell
# 1. Prepare engine dependencies
pip install -r app/engine/requirements.txt huggingface_hub
python app/engine/build_engine.py

# 2. Build Tauri desktop application
cd app/src-tauri
cargo tauri build
```

Installers are generated at:
- `app/src-tauri/target/release/bundle/nsis/*.exe`
- `app/src-tauri/target/release/bundle/msi/*.msi`

---

## 📁 Repository Structure

```text
frame-forge/
├── app/
│   ├── engine/           # Python optimization engine & build script
│   ├── src-tauri/        # Rust Tauri desktop core & configuration
│   └── ui/               # Desktop application UI (HTML, CSS, JS)
├── benchmark/            # Offline benchmarking and stress-testing harness
├── css/                  # Landing page stylesheet
├── js/                   # Landing page scripts
├── index.html            # Product landing page & download portal
└── README.md             # Project documentation
```

---

## 📄 License

- PresentMon: MIT License (Intel GameTechDev)
- FrameForge Application: Open Source under the MIT License
