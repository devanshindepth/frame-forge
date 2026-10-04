#!/usr/bin/env python3
"""
Build the offline AI engine into   app/src-tauri/resources/engine/
(run on the Windows release machine — the CI workflow does this automatically).

    python -m venv .venv && .venv\\Scripts\\activate
    pip install -r app/engine/requirements.txt
    python app/engine/build_engine.py

Steps
  1. Download the TabPFN v2 regressor checkpoint ONCE (build time only) and copy
     it to engine/models/. Players never download anything.
  2. Freeze frameforge_engine.py with PyInstaller (--onedir, faster start than onefile).
  3. Smoke-test: start the frozen engine with networking blocked and run a suggestion.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "src-tauri" / "resources" / "engine"
MODELS = HERE / "models"
CKPT = "tabpfn-v2-regressor.ckpt"


def fetch_weights() -> Path:
    MODELS.mkdir(exist_ok=True)
    dst = MODELS / CKPT
    if dst.exists():
        return dst
    print("downloading TabPFN regressor weights (build-time only)…")
    from huggingface_hub import hf_hub_download
    p = hf_hub_download(repo_id="Prior-Labs/TabPFN-v2-reg", filename=CKPT)
    shutil.copy(p, dst)
    return dst


def freeze() -> None:
    sep = ";" if sys.platform == "win32" else ":"
    subprocess.check_call([
        sys.executable, "-m", "PyInstaller", str(HERE / "frameforge_engine.py"),
        "--name", "frameforge-engine", "--onedir", "--noconfirm", "--clean", "--console",
        "--add-data", f"{MODELS / CKPT}{sep}models",
        "--collect-all", "tabpfn",
        "--collect-submodules", "scipy.stats",
        "--exclude-module", "matplotlib", "--exclude-module", "tkinter",
        "--distpath", str(HERE / "dist"), "--workpath", str(HERE / "build"),
    ])
    if OUT.exists():
        shutil.rmtree(OUT)
    shutil.copytree(HERE / "dist" / "frameforge-engine", OUT)
    print(f"engine copied to {OUT}")


def smoke_test() -> None:
    exe = OUT / ("frameforge-engine.exe" if sys.platform == "win32" else "frameforge-engine")
    p = {"space": [4, 4, 3], "quality": [[1, 2, 3, 4], [1, 3, 5, 6], [1, 2, 3]],
         "cost": [[0, .1, .2, .4], [0, .05, .1, .3], [0, .02, .05]], "quality_floor": 60,
         "target_hz": 144, "goal": "balanced", "max_tests": 12, "hardware": [12, 32, 16]}
    hist = [{"levels": [3, 3, 2], "fps_avg": 70, "fps_low": 48, "safe": True},
            {"levels": [1, 2, 1], "fps_avg": 104, "fps_low": 71, "safe": True},
            {"levels": [2, 1, 2], "fps_avg": 96, "fps_low": 66, "safe": True}]
    msgs = "\n".join(json.dumps(m) for m in ({"cmd": "hello"}, {"cmd": "suggest", "problem": p, "history": hist}, {"cmd": "quit"}))
    r = subprocess.run([str(exe)], input=msgs, capture_output=True, text=True, timeout=600,
                       env={"HF_HUB_OFFLINE": "1", "NO_PROXY": "*", "SYSTEMROOT": "C:\\Windows", "PATH": ""})
    lines = [json.loads(x) for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines and lines[0].get("ok"), f"hello failed: {r.stdout} {r.stderr[-500:]}"
    assert "levels" in lines[1] or lines[1].get("stop"), f"suggest failed: {lines}"
    print("smoke test OK:", lines[1])


if __name__ == "__main__":
    fetch_weights()
    freeze()
    smoke_test()
