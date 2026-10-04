#!/usr/bin/env python3
"""
FrameForge Smart Tuner — offline AI engine (sidecar process).

Protocol: one JSON object per line on stdin -> one JSON object per line on stdout.
    {"cmd":"hello"}                               -> {"ok":true,"model":"tabpfn","device":"cuda"}
    {"cmd":"suggest","problem":{...},"history":[...]} -> {"levels":[...],"stop":false,"predicted_fps":..,"chance_better":..,"reason":".."}
    {"cmd":"quit"}

100% offline:
  * model weights ship next to this executable (models/tabpfn-v2-regressor.ckpt)
  * HF_HUB_OFFLINE=1, telemetry disabled, and socket connections are blocked
    in-process as a belt-and-braces guarantee.

How a suggestion is made (constrained Bayesian optimisation):
  1. Context = every test measured so far this session (levels -> fps, 1% low).
     Feature columns = setting levels + the adapter's prior "cost" of the config.
  2. TabPFN (in-context, no training) predicts a distribution of FPS and 1%-low
     for ~4,000 candidate configurations that meet the player's picture-quality choice.
  3. Expected improvement of  min(fps, target) + 0.5*min(low, target)  is computed;
     the best candidate is tested next.
  4. Stop when the chance of a meaningful (>1.5 %) improvement is low.
"""
from __future__ import annotations

import json
import os
import socket
import sys
import traceback
from pathlib import Path

# ---------------------------------------------------------------- offline guarantees
os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
os.environ.setdefault("TABPFN_DISABLE_TELEMETRY", "1")
os.environ.setdefault("TABPFN_ALLOW_CPU_LARGE_DATASET", "1")


def _no_network(*_a, **_k):
    raise OSError("FrameForge engine is offline by design")


socket.socket.connect = _no_network          # type: ignore[assignment]
socket.create_connection = _no_network       # type: ignore[assignment]

import numpy as np  # noqa: E402
from scipy.stats import norm  # noqa: E402

BASE = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))
MODEL_PATH = BASE / "models" / "tabpfn-v2-regressor.ckpt"

_regressor_cls = None
_device = "cpu"


def out(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def load_model() -> dict:
    global _regressor_cls, _device
    try:
        import torch
        from tabpfn import TabPFNRegressor
    except Exception as e:  # pragma: no cover
        return {"ok": False, "error": f"import failed: {e}"}
    if not MODEL_PATH.exists():
        return {"ok": False, "error": f"model file missing: {MODEL_PATH}"}
    _device = "cuda" if torch.cuda.is_available() else "cpu"
    if _device == "cpu":
        torch.set_num_threads(max(1, (os.cpu_count() or 4) - 1))
    _regressor_cls = TabPFNRegressor
    return {"ok": True, "model": "tabpfn-v2", "device": _device}


def make_regressor():
    return _regressor_cls(
        model_path=str(MODEL_PATH),
        device=_device,
        n_estimators=8 if _device == "cuda" else 4,
        ignore_pretraining_limits=True,
        random_state=0,
    )


# ---------------------------------------------------------------- problem helpers
def quality_of(levels: np.ndarray, q: list[list[float]]) -> np.ndarray:
    mx = sum(max(v) for v in q) or 1.0
    got = np.zeros(levels.shape[0])
    for j, v in enumerate(q):
        got += np.asarray(v)[levels[:, j]]
    return got / mx * 100.0


def cost_of(levels: np.ndarray, c: list[list[float]]) -> np.ndarray:
    tot = np.zeros(levels.shape[0])
    for j, v in enumerate(c):
        tot += np.asarray(v)[levels[:, j]]
    return tot


def objective(fps, low, hz):
    return np.minimum(fps, hz) + 0.5 * np.minimum(low, hz)


def features(levels: np.ndarray, p: dict) -> np.ndarray:
    norm_lv = levels / np.maximum(np.asarray(p["space"]) - 1, 1)
    return np.column_stack([norm_lv, cost_of(levels, p["cost"]), quality_of(levels, p["quality"]) / 100.0])


def prior_guess(p: dict, start: list[int]) -> list[int]:
    """Cheapest-by-cost configuration that meets the quality floor (greedy)."""
    space, q, c, floor = p["space"], p["quality"], p["cost"], p["quality_floor"]
    cur = np.array([n - 1 for n in space])
    if quality_of(cur[None], q)[0] < floor:
        return list(start)
    while True:
        best, best_ratio = None, 0.0
        for i in range(len(cur)):
            if cur[i] == 0:
                continue
            cand = cur.copy(); cand[i] -= 1
            if quality_of(cand[None], q)[0] < floor:
                continue
            saved = c[i][cur[i]] - c[i][cand[i]]
            lost = max(q[i][cur[i]] - q[i][cand[i]], 0.05)
            if saved > 0 and saved / lost > best_ratio:
                best, best_ratio = i, saved / lost
        if best is None:
            return cur.tolist()
        cur[best] -= 1


def candidates(p: dict, incumbent: np.ndarray, tried: set, rng: np.random.Generator, n=4000) -> np.ndarray:
    space = np.asarray(p["space"])
    rand = np.column_stack([rng.integers(0, k, n // 2) for k in space])
    muts = np.repeat(incumbent[None], n // 2, axis=0)
    for row in muts:
        k = int(min(rng.integers(1, 4), len(space)))
        for j in rng.choice(len(space), size=k, replace=False):
            row[j] = rng.integers(0, space[j])
    allc = np.unique(np.vstack([rand, muts]), axis=0)
    keep = quality_of(allc, p["quality"]) >= p["quality_floor"]
    allc = allc[keep]
    if tried:
        allc = np.array([r for r in allc if tuple(r) not in tried]) if len(allc) else allc
    return allc.astype(int) if len(allc) else np.empty((0, len(space)), int)


def predict(X_ctx, y_ctx, X_new):
    m = make_regressor()
    m.fit(X_ctx, y_ctx)
    q16, q50, q84 = m.predict(X_new, output_type="quantiles", quantiles=[0.16, 0.5, 0.84])
    q50 = np.asarray(q50); sd = np.maximum((np.asarray(q84) - np.asarray(q16)) / 2.0, 1e-3)
    return q50, sd


def suggest(p: dict, history: list[dict]) -> dict:
    n = len(history)
    if n == 0:
        return {"levels": [], "stop": False, "reason": "First we measure your current settings."}
    tried = {tuple(h["levels"]) for h in history}
    if n == 1:
        g = prior_guess(p, history[0]["levels"])
        if tuple(g) not in tried:
            return {"levels": g, "stop": False,
                    "reason": "Trying the settings that usually give the biggest speed-up for the smallest visual loss."}
    if n >= p["max_tests"]:
        return {"stop": True, "reason": "Test limit reached."}

    hz = float(p["target_hz"])
    L = np.array([h["levels"] for h in history], int)
    fps = np.array([h["fps_avg"] for h in history], float)
    low = np.array([h["fps_low"] for h in history], float)
    safe = np.array([h["safe"] for h in history], bool)
    qual = quality_of(L, p["quality"])
    ok = safe & (qual >= p["quality_floor"] - 1e-6)
    obj_obs = objective(fps, low, hz)
    best = obj_obs[ok].max() if ok.any() else obj_obs.min()
    incumbent = L[np.argmax(np.where(ok, obj_obs, -1e9))]

    rng = np.random.default_rng(1000 + n)
    C = candidates(p, incumbent, tried, rng)
    if len(C) == 0:
        return {"stop": True, "reason": "Every option that meets your picture-quality choice has been covered."}

    # With fewer than 4 measurements a model fit is mostly guesswork, so explore:
    # pick the allowed candidate that is most different from everything tried.
    if n < 4:
        Ln = L / np.maximum(np.asarray(p["space"]) - 1, 1)
        Cn = C / np.maximum(np.asarray(p["space"]) - 1, 1)
        d = np.min(np.abs(Cn[:, None, :] - Ln[None, :, :]).sum(-1), axis=1)
        d = d - 0.5 * cost_of(C, p["cost"]) / max(cost_of(L, p["cost"]).max(), 1e-6)
        i = int(np.argmax(d))
        return {"levels": C[i].tolist(), "stop": False,
                "reason": "Exploring a different combination, so the Smart Tuner learns which settings matter on your PC."}

    X = features(L, p); Xc = features(C, p)
    # Unsafe runs (too hot / out of graphics memory) are kept in the context with a
    # pessimistic value so the model learns to steer away from them.
    fps_ctx = np.where(safe, fps, fps * 0.5); low_ctx = np.where(safe, low, low * 0.3)
    mu_f, sd_f = predict(X, fps_ctx, Xc)
    mu_l, sd_l = predict(X, low_ctx, Xc)

    mu = objective(mu_f, mu_l, hz)
    # fps above the target is worth nothing -> shrink uncertainty accordingly
    sd = np.hypot(np.where(mu_f < hz, sd_f, 0.2 * sd_f), 0.5 * np.where(mu_l < hz, sd_l, 0.2 * sd_l)) + 1e-6
    z = (mu - best) / sd
    ei = sd * (z * norm.cdf(z) + norm.pdf(z))
    # small preference for prettier configs when the speed is similar
    ei = ei * (1.0 + 0.002 * quality_of(C, p["quality"]))
    i = int(np.argmax(ei))
    p_better = float(1.0 - norm.cdf((best * 1.015 - mu[i]) / sd[i]))

    if n >= 6 and (ei[i] < 0.015 * best or p_better < 0.08):
        return {"stop": True, "chance_better": p_better,
                "reason": "More tests are unlikely to find anything better."}

    reason = (f"The Smart Tuner thinks this could reach about {mu_f[i]:.0f} FPS "
              f"({p_better * 100:.0f}% chance it beats your best so far).")
    return {"levels": C[i].tolist(), "stop": False, "predicted_fps": float(mu_f[i]),
            "chance_better": p_better, "reason": reason}


def main() -> None:
    loaded = None
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            cmd = req.get("cmd")
            if cmd == "hello":
                loaded = load_model()
                out(loaded)
            elif cmd == "suggest":
                if not loaded or not loaded.get("ok"):
                    loaded = load_model()
                    if not loaded.get("ok"):
                        out({"error": loaded.get("error", "model not loaded")}); continue
                out(suggest(req["problem"], req.get("history", [])))
            elif cmd == "quit":
                break
            else:
                out({"error": f"unknown cmd {cmd}"})
        except Exception as e:  # never crash the app: report and continue
            out({"error": f"{type(e).__name__}: {e}", "trace": traceback.format_exc()[-800:]})


if __name__ == "__main__":
    main()
