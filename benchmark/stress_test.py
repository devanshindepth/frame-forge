#!/usr/bin/env python3
"""
FrameForge stress-test harness
==============================

Compares surrogate models / optimisers for the settings-search problem.

IMPORTANT: by default it runs on a SYNTHETIC frame-time simulator defined below.
Results from that mode say how the methods behave on our toy model, NOT how they
behave in real games. Nothing from synthetic mode is published on the website.
Validation step V3 (roadmap) replaces the simulator with recorded real sweeps (--data).

Usage
-----
    pip install tabpfn scikit-learn xgboost catboost optuna scipy numpy pandas
    python benchmark/stress_test.py --quick                     # smoke run, synthetic
    python benchmark/stress_test.py --seeds 10                  # full synthetic run
    python benchmark/stress_test.py --data sweeps.csv           # (V3, not implemented yet)

Real data format (--data): one row per benchmark run with columns
    task_id, <setting columns...>, <hardware columns...>, fps_avg, fps_p1_low, quality, vram_peak_gb
Settings may be strings (categorical) or numbers. NaN allowed.

Tests implemented here
    T1 sample efficiency        T2 end-to-end BO (% of oracle)
    T3 calibration              T4 measurement-noise robustness
    T5 cold start (leave-one-GPU-out)
    T6 1%-low ranking (Spearman) T7 missing telemetry
    T9 compute cost              T10 large-data ceiling
    (T8 constraint-violation re-runs needs real hardware; planned for Phase 0.)
"""
from __future__ import annotations

import argparse
import json
import time
import warnings
from dataclasses import dataclass, field
from datetime import date

import numpy as np
import pandas as pd
from scipy.stats import norm, spearmanr

warnings.filterwarnings("ignore")

# --------------------------------------------------------------------------
# 1. Synthetic oracle: a physically-motivated GPU/CPU frametime simulator
#    (used when no real data is supplied). Mirrors the in-browser simulator.
# --------------------------------------------------------------------------
SETTINGS = {
    #  name           levels  gpu_cost_per_level            cpu_cost          vram_gb              quality
    "upscaler":      (4, [1.00, .67, .58, .48],           [0, 0, 0, 0],      [0, -.3, -.45, -.6], [14, 12.6, 11.2, 9.0]),
    "textures":      (4, [0, .01, .02, .03],               [0, 0, 0, 0],      [2.2, 3.4, 5.0, 7.0], [3, 6.5, 9, 10]),
    "shadows":       (4, [.02, .06, .11, .19],             [.02, .05, .08, .12], [0, .1, .2, .35], [3, 6.5, 8.5, 9]),
    "ray_tracing":   (4, [0, .22, .48, 1.1],               [0, .10, .18, .22], [0, .6, 1.1, 1.8],  [5, 9.5, 12, 13]),
    "ao":            (4, [0, .03, .06, .14],               [0, 0, 0, 0],      [0, .05, .1, .2],    [2, 4.5, 6, 6.5]),
    "fog":           (4, [.02, .05, .10, .18],             [0, 0, 0, 0],      [0, .05, .1, .2],    [3, 4.4, 5, 5.3]),
    "ssr":           (4, [0, .04, .09, .17],               [0, 0, 0, 0],      [0, .05, .1, .15],   [2, 4, 5, 5.3]),
    "aa":            (3, [0, .02, .07],                    [0, 0, 0],         [0, .05, .1],        [2, 4.5, 5]),
    "draw_distance": (4, [.02, .04, .07, .10],             [.05, .10, .17, .27], [0, .1, .2, .3],  [3, 5.5, 7, 7.5]),
    "crowd":         (3, [.01, .02, .04],                  [.04, .12, .26],   [0, .05, .1],        [2, 3.5, 4]),
    "lod":           (4, [.03, .06, .09, .13],             [.03, .06, .09, .13], [0, .1, .25, .4], [3, 5.5, 6.6, 7]),
    "post":          (3, [.01, .03, .05],                  [0, 0, 0],         [0, 0, 0],           [1.5, 2.6, 3]),
    "motion_blur":   (3, [0, .01, .02],                    [0, 0, 0],         [0, 0, 0],           [1.6, 1.6, 1.4]),
    "frame_gen":     (2, [0, 0],                           [0, 0],            [0, .5],             [3, 2.2]),
}
SNAMES = list(SETTINGS)
Q_MAX = sum(max(v[4]) for v in SETTINGS.values())

GPUS = {  # power, vram, rt_penalty, has_fg
    "rtx4090": (2.6, 24, 1.0, 1), "rtx4070": (1.35, 12, 1.0, 1), "rx7800": (1.38, 16, 1.6, 1),
    "rtx3060": (0.78, 12, 1.15, 0), "rtx4060m": (0.90, 8, 1.1, 1),
}
CPUS = {"7800x3d": 1.55, "13600k": 1.35, "5800x3d": 1.25, "5600": 1.0, "10400": 0.8}
GAMES = {  # gpu_load, cpu_load, scenes{name:(gpu_mult,cpu_mult)}
    "cp2077": (1.25, 1.1, {"market": (1.0, 1.35), "badlands": (0.9, 0.8), "rain": (1.25, 1.0)}),
    "aw2":    (1.45, 0.8, {"forest": (1.2, 0.9), "city": (1.05, 1.0), "lake": (1.1, 0.85)}),
    "bg3":    (0.75, 1.4, {"lowercity": (1.0, 1.5), "combat": (0.9, 0.9), "camp": (0.8, 0.8)}),
    "cs2":    (0.35, 0.9, {"mirage": (1.2, 1.1), "dm": (1.0, 1.2), "inferno": (1.1, 1.0)}),
    "msfs":   (1.10, 1.6, {"nyc": (1.1, 1.5), "ocean": (0.8, 0.7), "alps": (1.0, 0.9)}),
    "f124":   (0.90, 1.0, {"monaco": (1.1, 1.2), "spa_rain": (1.2, 1.0), "bahrain": (0.9, 0.9)}),
}
HW_TIERS = [("rtx4090", "7800x3d"), ("rtx4070", "5800x3d"), ("rx7800", "13600k"),
            ("rtx3060", "5600"), ("rtx4060m", "10400")]


@dataclass
class Task:
    game: str
    scene: str
    gpu: str
    cpu: str
    res_mult: float = 1.78   # 1440p
    interaction_seed: int = 0
    _rng: np.random.Generator = field(default=None, repr=False)

    def __post_init__(self):
        # hidden game-specific interactions -> makes the problem non-additive
        r = np.random.default_rng(self.interaction_seed)
        self.inter = r.normal(0, 0.04, size=(len(SNAMES), len(SNAMES)))

    def true_metrics(self, X: np.ndarray) -> dict:
        """X: (n, len(SNAMES)) integer levels -> noiseless metrics."""
        gp, gv, grt, gfg = GPUS[self.gpu]
        cp = CPUS[self.cpu]
        gl, cl, scenes = GAMES[self.game]
        sg, sc = scenes[self.scene]
        n = X.shape[0]
        gpu_cost = np.ones(n); cpu_cost = np.ones(n); vram = np.full(n, 1.6); q = np.zeros(n)
        scale = np.ones(n); fg = np.zeros(n, bool)
        for j, name in enumerate(SNAMES):
            _, gc, cc, vr, qq = SETTINGS[name]
            lv = X[:, j].astype(int)
            q += np.take(qq, lv); vram += np.take(vr, lv); cpu_cost += np.take(cc, lv)
            if name == "upscaler":
                scale = np.take(gc, lv)
            elif name == "frame_gen":
                fg = lv == 1
            elif name == "ray_tracing":
                gpu_cost += np.take(gc, lv) * grt
            else:
                gpu_cost += np.take(gc, lv)
        # pairwise interactions on normalised levels
        Z = X / np.array([SETTINGS[s][0] - 1 for s in SNAMES])
        gpu_cost *= np.exp(np.einsum("ni,ij,nj->n", Z, self.inter, Z) * 0.5)
        vram *= 1.1
        gpu_ms = 9.5 * self.res_mult * scale * gpu_cost * gl * sg / gp
        cpu_ms = 6.2 * cpu_cost * cl * sc / cp
        fps = 1000 / np.maximum(gpu_ms, cpu_ms)
        low = fps * np.where(cpu_ms > gpu_ms, 0.58, 0.72)
        over = vram > gv * 0.94
        fps = np.where(over, fps * 0.62, fps); low = np.where(over, low * 0.35, low)
        fgon = fg & bool(gfg)
        fps = np.where(fgon, fps * 1.75, fps); low = np.where(fgon, low * 1.5, low)
        temp = 62 + 22 * np.clip(gpu_ms / np.maximum(gpu_ms, cpu_ms), 0, 1) + 3 * (gp > 2)
        return {"fps_avg": fps, "fps_p1_low": low, "quality": q / Q_MAX * 100,
                "vram_peak_gb": vram, "gpu_hotspot_c": temp, "vram_cap": gv}

    def hw_features(self) -> np.ndarray:
        gp, gv, grt, gfg = GPUS[self.gpu]
        return np.array([gp, gv, grt, gfg, CPUS[self.cpu]], float)


def make_tasks() -> list[Task]:
    tasks, k = [], 0
    for g, (_, _, scenes) in GAMES.items():
        for gpu, cpu in HW_TIERS:
            for s in scenes:
                tasks.append(Task(g, s, gpu, cpu, interaction_seed=hash((g, s)) % 2**31))
                k += 1
    return tasks  # 6 * 5 * 3 = 90


def sample_configs(rng, n) -> np.ndarray:
    return np.column_stack([rng.integers(0, SETTINGS[s][0], n) for s in SNAMES]).astype(float)


def noisy(y, cv, rng):
    return y * (1 + rng.normal(0, cv, size=y.shape))


# --------------------------------------------------------------------------
# 2. Models. Every model exposes fit(X,y) and predict(X) -> (mean, std)
# --------------------------------------------------------------------------
class Model:
    name = "base"
    def fit(self, X, y): ...
    def predict(self, X): ...


class TabPFNModel(Model):
    name = "TabPFN"
    def __init__(self, device="auto", n_estimators=8):
        from tabpfn import TabPFNRegressor
        import torch
        dev = ("cuda" if torch.cuda.is_available() else "cpu") if device == "auto" else device
        self.m = TabPFNRegressor(device=dev, n_estimators=n_estimators, ignore_pretraining_limits=True)
    def fit(self, X, y): self.m.fit(X, y); return self
    def predict(self, X):
        q16, q50, q84 = self.m.predict(X, output_type="quantiles", quantiles=[0.16, 0.5, 0.84])
        return np.asarray(q50), np.maximum((np.asarray(q84) - np.asarray(q16)) / 2, 1e-6)


class GPModel(Model):
    name = "GP (Matérn-5/2)"
    def fit(self, X, y):
        from sklearn.gaussian_process import GaussianProcessRegressor
        from sklearn.gaussian_process.kernels import Matern, WhiteKernel, ConstantKernel
        from sklearn.impute import SimpleImputer
        self.imp = SimpleImputer().fit(X)
        self.mu, self.sd = y.mean(), y.std() + 1e-9
        k = ConstantKernel() * Matern(length_scale=np.ones(X.shape[1]), nu=2.5) + WhiteKernel()
        self.m = GaussianProcessRegressor(k, normalize_y=False, n_restarts_optimizer=2)
        self.m.fit(self.imp.transform(X), (y - self.mu) / self.sd); return self
    def predict(self, X):
        m, s = self.m.predict(self.imp.transform(X), return_std=True)
        return m * self.sd + self.mu, s * self.sd


def _tuned(make, space, X, y, trials):
    """Optuna-tuned model via 3-fold CV (only when n >= 12)."""
    import optuna
    from sklearn.model_selection import cross_val_score
    optuna.logging.set_verbosity(optuna.logging.WARNING)
    if len(y) < 12 or trials == 0:
        return make({}).fit(X, y)
    def obj(t):
        return cross_val_score(make(space(t)), X, y, cv=3, scoring="neg_mean_absolute_percentage_error").mean()
    st = optuna.create_study(direction="maximize"); st.optimize(obj, n_trials=trials)
    return make(st.best_params).fit(X, y)


class XGBModel(Model):
    name = "XGBoost"
    def __init__(self, trials=50): self.trials = trials
    def fit(self, X, y):
        from xgboost import XGBRegressor
        mk = lambda p: XGBRegressor(n_estimators=p.get("n", 300), max_depth=p.get("d", 4),
                                    learning_rate=p.get("lr", .05), subsample=p.get("ss", .8), verbosity=0)
        sp = lambda t: dict(n=t.suggest_int("n", 50, 600), d=t.suggest_int("d", 2, 8),
                            lr=t.suggest_float("lr", .01, .3, log=True), ss=t.suggest_float("ss", .5, 1))
        self.m = _tuned(mk, sp, X, y, self.trials)
        # bootstrap ensemble for uncertainty
        rng = np.random.default_rng(0); self.ens = []
        for _ in range(8):
            idx = rng.integers(0, len(y), len(y))
            self.ens.append(mk({}).fit(X[idx], y[idx]))
        return self
    def predict(self, X):
        P = np.stack([e.predict(X) for e in self.ens]); return self.m.predict(X), P.std(0) + 1e-6


class CatBoostModel(Model):
    name = "CatBoost"
    def __init__(self, trials=50): self.trials = trials
    def fit(self, X, y):
        from catboost import CatBoostRegressor
        mk = lambda p: CatBoostRegressor(iterations=p.get("it", 500), depth=p.get("d", 4),
                                         learning_rate=p.get("lr", .05), verbose=0,
                                         loss_function="RMSEWithUncertainty")
        sp = lambda t: dict(it=t.suggest_int("it", 100, 800), d=t.suggest_int("d", 2, 8),
                            lr=t.suggest_float("lr", .01, .3, log=True))
        self.m = _tuned(mk, sp, X, y, self.trials); return self
    def predict(self, X):
        P = self.m.predict(X); return P[:, 0], np.sqrt(np.maximum(P[:, 1], 1e-9))


class RFModel(Model):
    name = "Random Forest"
    def fit(self, X, y):
        from sklearn.ensemble import RandomForestRegressor
        from sklearn.impute import SimpleImputer
        self.imp = SimpleImputer().fit(X)
        self.m = RandomForestRegressor(300, min_samples_leaf=2, n_jobs=-1).fit(self.imp.transform(X), y); return self
    def predict(self, X):
        Xi = self.imp.transform(X); P = np.stack([t.predict(Xi) for t in self.m.estimators_])
        return P.mean(0), P.std(0) + 1e-6


class MLPModel(Model):
    name = "MLP"
    def fit(self, X, y):
        from sklearn.neural_network import MLPRegressor
        from sklearn.preprocessing import StandardScaler
        from sklearn.impute import SimpleImputer
        self.imp = SimpleImputer().fit(X); self.sc = StandardScaler().fit(self.imp.transform(X))
        self.mu, self.sd = y.mean(), y.std() + 1e-9
        Xs = self.sc.transform(self.imp.transform(X))
        self.ens = [MLPRegressor((64, 64), alpha=1e-3, max_iter=2000, random_state=s).fit(Xs, (y - self.mu) / self.sd)
                    for s in range(5)]
        return self
    def predict(self, X):
        Xs = self.sc.transform(self.imp.transform(X)); P = np.stack([e.predict(Xs) for e in self.ens])
        return P.mean(0) * self.sd + self.mu, P.std(0) * self.sd + 1e-6


MODEL_FACTORIES = {
    "TabPFN": lambda a: TabPFNModel(device=a.device),
    "GP (Matérn-5/2)": lambda a: GPModel(),
    "CatBoost": lambda a: CatBoostModel(trials=a.trials),
    "XGBoost": lambda a: XGBModel(trials=a.trials),
    "Random Forest": lambda a: RFModel(),
    "MLP": lambda a: MLPModel(),
}


def mape(y, p): return float(np.mean(np.abs((y - p) / np.maximum(np.abs(y), 1e-6))) * 100)


# --------------------------------------------------------------------------
# 3. Tests
# --------------------------------------------------------------------------
def t1_sample_efficiency(tasks, args, ns=(8, 16, 32, 64, 128, 256), cv=0.04):
    out = {m: [] for m in args.models}
    for n in ns:
        for m in args.models:
            errs = []
            for seed in range(args.seeds):
                rng = np.random.default_rng(seed)
                task = tasks[rng.integers(len(tasks))]
                Xtr, Xte = sample_configs(rng, n), sample_configs(rng, 400)
                ytr = noisy(task.true_metrics(Xtr)["fps_avg"], cv, rng)
                yte = task.true_metrics(Xte)["fps_avg"]
                mu, _ = MODEL_FACTORIES[m](args).fit(Xtr, ytr).predict(Xte)
                errs.append(mape(yte, mu))
            out[m].append(round(float(np.mean(errs)), 2))
            print(f"[T1] n={n:4d} {m:18s} MAPE={out[m][-1]:.2f}")
    return {"x": list(ns), "series": out}


def bo_run(task, model_name, args, budget, rng, floor=85.0, cv=0.04):
    """Constrained-EI BO with the given surrogate. Returns best-feasible TRUE objective trace."""
    hz = 165.0
    def obj(m): return np.minimum(m["fps_avg"], hz) + 0.5 * np.minimum(m["fps_p1_low"], hz)
    def feas(m): return (m["quality"] >= floor) & (m["vram_peak_gb"] <= m["vram_cap"] * 0.94)
    X = sample_configs(rng, 4); trace = []
    for it in range(budget):
        if it >= 4:
            cand = sample_configs(rng, 2000)
            Ym = {k: noisy(v, cv, rng) for k, v in task.true_metrics(X).items() if k != "vram_cap"}
            yo = obj(Ym); f = feas({**Ym, "vram_cap": task.true_metrics(X)["vram_cap"]})
            best = yo[f].max() if f.any() else yo.min()
            mo, so = MODEL_FACTORIES[model_name](args).fit(X, yo).predict(cand)
            mq, sq = MODEL_FACTORIES[model_name](args).fit(X, Ym["quality"]).predict(cand)
            z = (mo - best) / so; ei = so * (z * norm.cdf(z) + norm.pdf(z))
            pq = norm.cdf((mq - floor) / sq)
            X = np.vstack([X, cand[np.argmax(ei * pq)]])
        tm = task.true_metrics(X[: max(it + 1, 1)])
        o, f = obj(tm), feas(tm)
        trace.append(o[f].max() if f.any() else 0.0)
    return np.array(trace)


def oracle_best(task, rng, floor=85.0, n=20000):
    X = sample_configs(rng, n); m = task.true_metrics(X)
    o = np.minimum(m["fps_avg"], 165) + 0.5 * np.minimum(m["fps_p1_low"], 165)
    f = (m["quality"] >= floor) & (m["vram_peak_gb"] <= m["vram_cap"] * 0.94)
    return o[f].max()


def t2_regret(tasks, args, checkpoints=(5, 10, 15, 20, 30, 40, 60)):
    budget = max(checkpoints)
    bo_models = {"TabPFN": "TabPFN-BO", "GP (Matérn-5/2)": "GP-BO (BoTorch qEI)",
                 "Random Forest": "SMAC (RF)", "XGBoost": "XGB + bootstrap EI"}
    series = {}
    for m, label in bo_models.items():
        if m not in args.models: continue
        traces = []
        for seed in range(args.seeds):
            rng = np.random.default_rng(100 + seed); task = tasks[rng.integers(len(tasks))]
            ob = oracle_best(task, np.random.default_rng(seed))
            traces.append(bo_run(task, m, args, budget, rng) / ob * 100)
        T = np.mean(traces, 0); series[label] = [round(float(T[c - 1]), 1) for c in checkpoints]
        print(f"[T2] {label:22s} {series[label]}")
    # random search baseline
    traces = []
    for seed in range(args.seeds):
        rng = np.random.default_rng(100 + seed); task = tasks[rng.integers(len(tasks))]
        ob = oracle_best(task, np.random.default_rng(seed))
        X = sample_configs(rng, budget); m = task.true_metrics(X)
        o = np.minimum(m["fps_avg"], 165) + 0.5 * np.minimum(m["fps_p1_low"], 165)
        f = (m["quality"] >= 85) & (m["vram_peak_gb"] <= m["vram_cap"] * 0.94)
        o = np.where(f, o, 0); traces.append(np.maximum.accumulate(o) / ob * 100)
    T = np.mean(traces, 0); series["Random search"] = [round(float(T[c - 1]), 1) for c in checkpoints]
    return {"x": list(checkpoints), "series": series}


def t3_calibration(tasks, args, n=32, levels=(50, 80, 90, 95)):
    out = {"Ideal": list(levels)}
    for m in args.models:
        cov = {l: [] for l in levels}
        for seed in range(args.seeds):
            rng = np.random.default_rng(200 + seed); task = tasks[rng.integers(len(tasks))]
            Xtr, Xte = sample_configs(rng, n), sample_configs(rng, 400)
            ytr = noisy(task.true_metrics(Xtr)["fps_avg"], 0.06, rng)
            yte = noisy(task.true_metrics(Xte)["fps_avg"], 0.06, rng)
            mu, sd = MODEL_FACTORIES[m](args).fit(Xtr, ytr).predict(Xte)
            for l in levels:
                z = norm.ppf(0.5 + l / 200); cov[l].append(np.mean(np.abs(yte - mu) <= z * sd) * 100)
        out[m] = [round(float(np.mean(cov[l])), 1) for l in levels]
        print(f"[T3] {m:18s} coverage={out[m]}")
    return {"x": list(levels), "series": out}


def t4_noise(tasks, args, cvs=(0, .02, .05, .10, .15, .20), n=32):
    out = {m: [] for m in args.models}
    for cv in cvs:
        for m in args.models:
            errs = []
            for seed in range(args.seeds):
                rng = np.random.default_rng(300 + seed); task = tasks[rng.integers(len(tasks))]
                Xtr, Xte = sample_configs(rng, n), sample_configs(rng, 400)
                ytr = noisy(task.true_metrics(Xtr)["fps_avg"], cv, rng)
                mu, _ = MODEL_FACTORIES[m](args).fit(Xtr, ytr).predict(Xte)
                errs.append(mape(task.true_metrics(Xte)["fps_avg"], mu))
            out[m].append(round(float(np.mean(errs)), 2))
        print(f"[T4] cv={cv:.2f} " + " ".join(f"{m}={out[m][-1]}" for m in args.models))
    return {"x": [int(c * 100) for c in cvs], "series": out}


def t5_cold_start(tasks, args, n_target=8, n_pool=60):
    """Leave-one-GPU-out with hardware features + pooled rows from other GPUs."""
    out = {m: [0.0, 0.0] for m in args.models}
    for m in args.models:
        e_avg, e_low = [], []
        for seed in range(args.seeds):
            rng = np.random.default_rng(400 + seed)
            game = list(GAMES)[rng.integers(len(GAMES))]
            ts = [t for t in tasks if t.game == game and t.scene == list(GAMES[game][2])[0]]
            target = ts[rng.integers(len(ts))]
            rows, ya, yl = [], [], []
            for t in ts:
                k = n_target if t is target else n_pool
                Xc = sample_configs(rng, k); tm = t.true_metrics(Xc)
                rows.append(np.hstack([Xc, np.tile(t.hw_features(), (k, 1))]))
                ya.append(noisy(tm["fps_avg"], .04, rng)); yl.append(noisy(tm["fps_p1_low"], .06, rng))
            Xtr = np.vstack(rows); ya = np.concatenate(ya); yl = np.concatenate(yl)
            Xte_c = sample_configs(rng, 300); tm = target.true_metrics(Xte_c)
            Xte = np.hstack([Xte_c, np.tile(target.hw_features(), (300, 1))])
            e_avg.append(mape(tm["fps_avg"], MODEL_FACTORIES[m](args).fit(Xtr, ya).predict(Xte)[0]))
            e_low.append(mape(tm["fps_p1_low"], MODEL_FACTORIES[m](args).fit(Xtr, yl).predict(Xte)[0]))
        out[m] = [round(float(np.mean(e_avg)), 2), round(float(np.mean(e_low)), 2)]
        print(f"[T5] {m:18s} {out[m]}")
    return {"x": ["Average FPS", "1% low FPS"], "series": out}


def t6_ranking(tasks, args, n=32):
    out = {}
    for m in args.models:
        rs = []
        for seed in range(args.seeds):
            rng = np.random.default_rng(500 + seed); task = tasks[rng.integers(len(tasks))]
            Xtr, Xte = sample_configs(rng, n), sample_configs(rng, 400)
            ytr = noisy(task.true_metrics(Xtr)["fps_p1_low"], .06, rng)
            mu, _ = MODEL_FACTORIES[m](args).fit(Xtr, ytr).predict(Xte)
            rs.append(spearmanr(mu, task.true_metrics(Xte)["fps_p1_low"]).correlation)
        out[m] = [round(float(np.mean(rs)), 3)]
        print(f"[T6] {m:18s} rho={out[m][0]}")
    return {"x": ["Spearman ρ"], "series": out}


def t7_missing(tasks, args, fracs=(0, .1, .2, .3, .4), n=64):
    out = {m: [] for m in args.models if m != "MLP"}
    for fr in fracs:
        for m in out:
            errs = []
            for seed in range(args.seeds):
                rng = np.random.default_rng(600 + seed); task = tasks[rng.integers(len(tasks))]
                Xtr, Xte = sample_configs(rng, n), sample_configs(rng, 400)
                ytr = noisy(task.true_metrics(Xtr)["fps_avg"], .04, rng)
                yte = task.true_metrics(Xte)["fps_avg"]
                # add hardware telemetry columns then knock values out
                hw = task.hw_features()
                Htr = np.tile(hw, (n, 1)) * (1 + rng.normal(0, .02, (n, len(hw))))
                Hte = np.tile(hw, (400, 1)) * (1 + rng.normal(0, .02, (400, len(hw))))
                A, B = np.hstack([Xtr, Htr]), np.hstack([Xte, Hte])
                A[rng.random(A.shape) < fr] = np.nan; B[rng.random(B.shape) < fr] = np.nan
                mu, _ = MODEL_FACTORIES[m](args).fit(A, ytr).predict(B)
                errs.append(mape(yte, mu))
            out[m].append(round(float(np.mean(errs)), 2))
        print(f"[T7] missing={fr:.1f} " + " ".join(f"{m}={out[m][-1]}" for m in out))
    return {"x": [int(f * 100) for f in fracs], "series": out}


def t9_compute(tasks, args, n=64, n_cand=5000):
    out = {}
    rng = np.random.default_rng(900); task = tasks[0]
    Xtr, Xc = sample_configs(rng, n), sample_configs(rng, n_cand)
    ytr = task.true_metrics(Xtr)["fps_avg"]
    for m in args.models:
        t0 = time.perf_counter(); MODEL_FACTORIES[m](args).fit(Xtr, ytr).predict(Xc)
        out[m] = [round(time.perf_counter() - t0, 2)]
        print(f"[T9] {m:18s} {out[m][0]} s")
    return {"x": ["Wall-clock seconds"], "series": out}


def t10_scale(tasks, args, sizes=(256, 1000, 4000, 10000)):
    models = [m for m in ("TabPFN", "CatBoost", "XGBoost") if m in args.models]
    out = {m: [] for m in models}
    pool = [t for t in tasks if t.game == "cp2077"]
    for N in sizes:
        rng = np.random.default_rng(1000)
        rows, ys = [], []
        for t in pool:
            k = N // len(pool); Xc = sample_configs(rng, k)
            rows.append(np.hstack([Xc, np.tile(t.hw_features(), (k, 1))]))
            ys.append(noisy(t.true_metrics(Xc)["fps_avg"], .04, rng))
        X, y = np.vstack(rows), np.concatenate(ys)
        te = pool[0]; Xt = sample_configs(rng, 500)
        Xte = np.hstack([Xt, np.tile(te.hw_features(), (500, 1))]); yte = te.true_metrics(Xt)["fps_avg"]
        for m in models:
            mdl = MODEL_FACTORIES[m](args)
            if m != "TabPFN" and hasattr(mdl, "trials"): mdl.trials = min(args.trials, 15)
            out[m].append(round(mape(yte, mdl.fit(X, y).predict(Xte)[0]), 2))
        print(f"[T10] N={N} " + " ".join(f"{m}={out[m][-1]}" for m in models))
    return {"x": [str(s) if s < 1000 else f"{s // 1000}k" for s in sizes], "series": out}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seeds", type=int, default=10)
    ap.add_argument("--trials", type=int, default=50, help="Optuna trials for GBDT tuning")
    ap.add_argument("--device", default="auto")
    ap.add_argument("--models", nargs="+", default=list(MODEL_FACTORIES))
    ap.add_argument("--tests", nargs="+", default=["t1", "t2", "t3", "t4", "t5", "t6", "t7", "t9", "t10"])
    ap.add_argument("--quick", action="store_true", help="3 seeds, 5 tuning trials")
    ap.add_argument("--json", default="benchmark/results-synthetic.json")
    ap.add_argument("--data", help="(planned) CSV of real runs — see docstring")
    args = ap.parse_args()
    if args.quick: args.seeds, args.trials = 3, 5
    if args.data:
        raise SystemExit("Real-sweep loader is validation step V3 (not implemented yet); see roadmap.html#validation.")

    tasks = make_tasks(); print(f"{len(tasks)} tasks, {len(SNAMES)} settings")
    fns = {"t1": t1_sample_efficiency, "t2": t2_regret, "t3": t3_calibration, "t4": t4_noise,
           "t5": t5_cold_start, "t6": t6_ranking, "t7": t7_missing, "t9": t9_compute, "t10": t10_scale}
    results = {}
    for k in args.tests:
        t0 = time.time(); results[k] = fns[k](tasks, args); print(f"  {k} done in {time.time() - t0:.0f}s")
    payload = {"provenance": "synthetic-simulator", "date": str(date.today()), "seeds": args.seeds,
               "note": "Toy frame-time model; not real-game evidence.", "results": results}
    with open(args.json, "w") as f: json.dump(payload, f, indent=2)
    print(f"wrote {args.json} (synthetic)")


if __name__ == "__main__":
    main()
