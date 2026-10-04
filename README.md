# FrameForge — Adaptive Game Performance & Stress Optimizer (TabPFN)

Concept landing page and technical roadmap for an optimizer that learns from a small number of benchmark runs on a user's PC, game and scene, then finds graphics settings that maximize FPS while preserving visual quality. TabPFN is the surrogate model inside a constrained Bayesian-optimization loop.

## Pages / entry URIs
| Path | Contents |
|---|---|
| `index.html` | Hero, problem, how it works, why TabPFN, **10 interactive stress tests** + scorecard, test protocol, 9 alternatives compared, honest limitations, interactive simulator |
| `index.html#stress-tests` | Stress-test charts (T1–T10) |
| `index.html#demo` | In-browser simulator (GPU / CPU / game / scene / resolution / quality floor / budget) |
| `roadmap.html` | User flow, architecture, data model, optimizer code, visual-quality metrics, TabPFN serving, 5-phase plan, KPIs, risks, tech stack |

## Stress tests (in `js/stress-data.js`)
T1 sample efficiency · T2 end-to-end BO (% of oracle) · T3 calibration · T4 noise robustness · T5 cold start on unseen GPU · T6 1%-low ranking · T7 missing telemetry · T8 constraint violations · T9 compute cost (TabPFN loses on CPU) · T10 large-data ceiling (GBDTs win). Baselines: GP, GP-BO, XGBoost, CatBoost, RF/SMAC, TPE, CMA-ES, MLP, kNN, Ridge, random search, vendor presets.

**Provenance:** the current numbers are labelled *design-phase reference numbers* (a banner on the page says so). They were not measured on real hardware. `benchmark/stress_test.py` runs the tests on a synthetic frametime oracle (90 tasks, 14 settings). Running it with `--out js/stress-data.js` rewrites the data file with `provenance: "measured"`, and the page switches to a "Measured results" banner.

```
pip install tabpfn scikit-learn xgboost catboost optuna scipy numpy pandas
python benchmark/stress_test.py --quick
python benchmark/stress_test.py --seeds 10 --out js/stress-data.js   # needs node to merge
```

## Files
- `css/style.css`: shared design system
- `js/stress-data.js`: stress-test data and narrative (the page renders entirely from this file)
- `js/main.js`: terminal animation, Chart.js charts, scorecard, simulator (an analytical frametime model, not TabPFN in the browser)
- `benchmark/stress_test.py`: reproducible harness

## Not yet implemented
- Real-hardware oracle datasets and the `--data` CSV loader (Phase 0)
- T8 (constraint violations on re-run) in the harness, because it needs real hardware
- Desktop app, cloud TabPFN service, community data platform (Phases 1–4 in `roadmap.html`)

## Next steps
1. Run the harness and replace the reference numbers with measured ones.
2. Build real oracles (3 games × 3 PCs × 2 scenes × 1,200 configs).
3. Go or no-go: TabPFN must beat GP-BO on T1, T2 and T3 with non-overlapping confidence intervals.

## Data / storage
Static site with no database. All data is in `js/stress-data.js`.
