/* Browser-preview backend for FrameForge's UI.
   Used ONLY when the UI is opened in a normal browser (no window.__TAURI__),
   so designers can click through every screen. ALL NUMBERS HERE ARE MADE UP
   (a toy frame-time formula, compressed in time) and a banner says so on screen.
   The real app ignores this file and only ever shows measured values. */
(function () {
  "use strict";
  if (window.__TAURI__) return;

  const listeners = {};
  const emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb({ payload }));

  const ADAPTERS = {
    cs2: { name: "Counter-Strike 2", short: "Competitive shooter. Every frame counts.", where: "Play → Practice → Workshop map \"CS2 FPS Benchmark\". When the map has loaded, press F9.",
      settings: [["Shadows", ["Low", "Medium", "High", "Very high"]], ["Moving shadows", ["Off", "On"]], ["Texture detail", ["Low", "Medium", "High"]], ["Lighting detail", ["Low", "High"]], ["Effects (smoke, sparks)", ["Low", "Medium", "High", "Very high"]], ["Soft shadows in corners", ["Off", "Medium", "High"]], ["Edge smoothing", ["None", "2x", "4x", "8x"]], ["Render scaling (FSR)", ["Off (sharpest)", "Ultra quality", "Quality", "Balanced"]]], base: 248 },
    palworld: { name: "Palworld", short: "Open world. Heavy on grass, shadows and view distance.", where: "Load your world and stand at your base. Press F9 and slowly turn the camera in a full circle.",
      settings: [["View distance", ["Low", "Medium", "High", "Epic"]], ["Shadows", ["Low", "Medium", "High", "Epic"]], ["Indirect lighting", ["Low", "Medium", "High", "Epic"]], ["Reflections", ["Low", "Medium", "High", "Epic"]], ["Texture detail", ["Low", "Medium", "High", "Epic"]], ["Grass & plants", ["Low", "Medium", "High", "Epic"]], ["Effects", ["Low", "Medium", "High", "Epic"]], ["Edge smoothing", ["Low", "Medium", "High", "Epic"]]], base: 71 },
    b1_benchmark: { name: "Black Myth: Wukong Benchmark", short: "Unreal Engine 5 standalone benchmark with automated camera flythrough.", where: "Click 'Start Benchmark' in the main menu. Press F9 once the flythrough starts.",
      settings: [["View distance", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Shadows", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Indirect lighting (Lumen)", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Reflections", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Grass & plants", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Post-processing", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Texture detail", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Visual effects", ["Low", "Medium", "High", "Very High", "Cinematic"]], ["Edge smoothing", ["Low", "Medium", "High", "Very High", "Cinematic"]]], base: 64 },
  };
  const HELP = { "Shadows": "How sharp and detailed shadows are.", "Indirect lighting": "Light bouncing off surfaces.", "Indirect lighting (Lumen)": "Lumen dynamic global illumination.", "Grass & plants": "How much grass is drawn.", "View distance": "How far things are shown in detail.", "Edge smoothing": "Removes jagged edges.", "Visual effects": "Sparks, fog and particles.", "Post-processing": "Camera bloom and motion blur.", "Render scaling (FSR)": "Draws the game smaller and sharpens it back up." };

  const store = {
    onboarded: false,
    preferences: { temp_limit_c: 87, test_length: "normal", max_tests: 12, refresh_hz: 0 },
    results: [
      { id: "r-old", game_id: "palworld", goal: "balanced", quality_floor: 80, started_at: "2026-09-28T19:02:00", finished_at: "2026-09-28T19:21:00", engine: "Smart Tuner (TabPFN, offline)", status: "completed", applied: true,
        baseline: { index: 1, levels: [3, 3, 3, 3, 3, 3, 3, 3], fps_avg: 47.2, fps_low: 31.0, quality: 100, safe: true, note: "", settings_held: true, kind: "search" },
        best: { index: 9, levels: [2, 1, 1, 3, 3, 1, 2, 3], fps_avg: 68.9, fps_low: 49.5, quality: 84.4, safe: true, note: "", settings_held: true, kind: "search" },
        runs: [{ kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "search" }, { kind: "check" }, { kind: "check" }],
        confirmed: { baseline_fps: 47.6, baseline_low: 31.4, best_fps: 68.1, best_low: 48.9, noise_pct: 2.3 } }
    ]
  };
  // Preview-only deep links: ?screen=home|setup|live|result|history|settings|help
  const screen = window.__FF_SCREEN__ || new URLSearchParams(location.search).get("screen");
  if (screen) store.onboarded = true;
  let status = { active: false, phase: "", runs: [] };
  if (screen === "live") {
    status = { active: true, game_id: "palworld", game_name: "Palworld", phase: "measuring", headline: "Measuring… keep playing normally", detail: "You'll see nothing and hear nothing. FrameForge is just counting frames.", test_index: 5, max_tests: 12, seconds_done: 38, seconds_total: 60, gpu_temp: 71, engine: "Smart Tuner (TabPFN, offline)", baseline_fps: 52.4, best_fps: 78.9,
      temp_guard: true,
      runs: [{ index: 1, fps_avg: 52.4, fps_low: 34.1, quality: 100, safe: true, note: "", kind: "search" }, { index: 2, fps_avg: 71.8, fps_low: 49.0, quality: 86.2, safe: true, note: "", kind: "search" }, { index: 3, fps_avg: 66.1, fps_low: 45.3, quality: 91.4, safe: true, note: "", kind: "search" }, { index: 4, fps_avg: 78.9, fps_low: 55.7, quality: 87.9, safe: true, note: "", kind: "search" }] };
  }
  let timer = null, ready = false, cancel = false;

  const hw = { cpu: "AMD Ryzen 7 5800X3D 8-Core Processor", cpu_cores: 8, cpu_threads: 16, ram_gb: 32, gpu: "NVIDIA GeForce RTX 4070", gpu_vendor: "NVIDIA", vram_gb: 12, os: "Windows 11 (26100)", live_sensors: true, tier: "Strong gaming PC" };

  function games() {
    return Object.entries(ADAPTERS).map(([id, a]) => ({
      id, name: a.name, short: a.short, installed: true, settings_found: true, setting_count: a.settings.length, support: "unverified",
      tips: ["Use the same spot and the same movement every test, so results are fair."], where_to_test: a.where,
      last_result: [...store.results].reverse().find((r) => r.game_id === id && r.best) || null, has_backup: id === "palworld"
    }));
  }

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const set = (patch) => { Object.assign(status, patch); emit("session", JSON.parse(JSON.stringify(status))); };

  async function wait(cond) { while (!cond()) { if (cancel) throw "cancelled"; await sleep(150); } }
  async function pause(ms) { const end = Date.now() + ms; while (Date.now() < end) { if (cancel) throw "cancelled"; await sleep(100); } }

  async function runSession(gameId, goal, floor) {
    const a = ADAPTERS[gameId];
    const n = a.settings.length;
    const maxTests = store.preferences.max_tests;
    const quality = (lv) => lv.reduce((s, l, i) => s + (l + 1) / a.settings[i][1].length, 0) / n * 100;
    const fpsOf = (lv) => {
      const cost = lv.reduce((s, l, i) => s + l * (0.05 + (i % 3) * 0.035), 0);
      return a.base / (0.55 + cost * 0.55) * (1 + (Math.random() - 0.5) * 0.03);
    };
    let lv = a.settings.map((s) => s[1].length - 1);
    const runs = [];
    set({ active: true, game_id: gameId, game_name: a.name, phase: "preparing", headline: "Getting ready…", detail: "Saving a copy of your current settings so we can always put them back.", max_tests: maxTests, seconds_total: 60, seconds_done: 0, engine: "Smart Tuner (TabPFN, offline)", runs: [], result_id: null, error_code: null, best_fps: null, baseline_fps: null, test_index: 0 });
    await pause(1200);
    let reason = "First we measure your current settings, so we know where you're starting from.";
    const tests = Math.min(maxTests, 7 + Math.floor(Math.random() * 3));
    for (let i = 0; i < tests; i++) {
      set({ test_index: i + 1, phase: "applying", headline: `Test ${i + 1} of up to ${maxTests}`, detail: reason, seconds_done: 0 });
      await pause(900);
      set({ phase: "launching", headline: `Starting ${a.name}…` });
      await pause(1100);
      ready = false;
      set({ phase: "waiting", headline: "Your turn: go to the test spot and press F9", detail: a.where });
      await wait(() => ready);
      set({ phase: "warmup", headline: "Hold on, getting a steady reading…", detail: "Keep doing the same thing as in the last test." });
      await pause(900);
      set({ phase: "measuring", headline: "Measuring… keep playing normally", detail: "You'll see nothing and hear nothing. FrameForge is just counting frames." });
      for (let s = 0; s <= 60; s += 6) { set({ seconds_done: s, gpu_temp: 62 + Math.round(Math.random() * 10) }); await pause(160); }
      const fps = fpsOf(lv);
      const rec = { index: i + 1, levels: lv.slice(), fps_avg: +fps.toFixed(1), fps_low: +(fps * 0.69).toFixed(1), quality: +quality(lv).toFixed(1), vram_gb: 7.2, max_gpu_temp: 71, safe: true, note: "", settings_held: true, kind: "search" };
      runs.push(rec);
      const ok = runs.filter((r) => r.quality >= floor);
      set({ runs: runs.slice(), baseline_fps: runs[0].fps_avg, best_fps: ok.length ? Math.max(...ok.map((r) => r.fps_avg)) : null, phase: "thinking", headline: "Thinking about what to try next…", detail: "The Smart Tuner is learning how your PC handles this game." });
      await pause(1000);
      // next config: lower the most expensive setting(s) that keep quality above floor
      const next = lv.slice();
      for (let k = 0; k < 3; k++) {
        const j = Math.floor(Math.random() * n);
        if (next[j] > 0) next[j]--;
        if (quality(next) < floor) next[j]++;
        if (Math.random() < 0.25 && next[j] < a.settings[j][1].length - 1) next[j]++;
      }
      lv = next;
      const pred = fpsOf(lv);
      reason = `The Smart Tuner thinks this could reach about ${pred.toFixed(0)} FPS (${Math.max(8, 70 - i * 9)}% chance it beats your best so far).`;
    }
    const ok = runs.filter((r) => r.quality >= floor - 0.01);
    const best = ok.slice().sort((x, y) => (y.fps_avg + 0.5 * y.fps_low) - (x.fps_avg + 0.5 * x.fps_low))[0] || runs[0];
    set({ phase: "checking", headline: "Double-checking the winner", detail: "We measure the best settings and your original settings one more time." });
    await pause(1500);
    const again = (r) => r.fps_avg * (1 + (Math.random() - 0.5) * 0.04);
    const cb = again(best), co = again(runs[0]);
    const confirmed = { baseline_fps: (runs[0].fps_avg + co) / 2, baseline_low: runs[0].fps_low, best_fps: (best.fps_avg + cb) / 2, best_low: best.fps_low, noise_pct: Math.max(Math.abs(cb - best.fps_avg) / best.fps_avg, Math.abs(co - runs[0].fps_avg) / runs[0].fps_avg) * 100 };
    runs.push({ index: runs.length + 1, kind: "check", fps_avg: +cb.toFixed(1), fps_low: best.fps_low, quality: best.quality, safe: true, note: "", settings_held: true });
    runs.push({ index: runs.length + 1, kind: "check", fps_avg: +co.toFixed(1), fps_low: runs[0].fps_low, quality: runs[0].quality, safe: true, note: "", settings_held: true });
    const result = { id: "r-" + Date.now(), game_id: gameId, goal, quality_floor: floor, started_at: new Date().toISOString(), finished_at: new Date().toISOString(), engine: "Smart Tuner (TabPFN, offline)", status: "completed", baseline: runs[0], best, runs, applied: false, confirmed };
    store.results.push(result);
    const gain = (confirmed.best_fps / confirmed.baseline_fps - 1) * 100;
    set({ active: false, runs: runs.slice(), phase: "finished", headline: gain > Math.max(3, confirmed.noise_pct) ? `Done! ${gain.toFixed(0)}% more frames per second, confirmed.` : "Done. Your current settings are already about as fast as it gets.", detail: "Your original settings are back in place.", result_id: result.id });
  }

  const commands = {
    get_bootstrap: () => ({ version: "1.0.0 (preview)", hardware: hw, games: games(), preferences: store.preferences, onboarded: store.onboarded, ai_engine_installed: true, measuring_tool_installed: true, status, recovered_session: false }),
    refresh_games: () => games(),
    start_session: ({ gameId, goal, qualityFloor }) => {
      if (status.active) throw "A test session is already running.";
      cancel = false;
      runSession(gameId, goal, qualityFloor).catch((e) => {
        set({ active: false, phase: e === "cancelled" ? "stopped" : "failed", headline: e === "cancelled" ? "Stopped. Your game is back to how it was." : "Something went wrong.", detail: "Nothing was changed. You can start again any time.", error_code: String(e) });
      });
      return null;
    },
    player_ready: () => { ready = true; },
    stop_session: () => { cancel = true; },
    session_status: () => status,
    get_result: ({ id }) => {
      const r = store.results.find((x) => x.id === id);
      if (!r) throw "This result no longer exists.";
      const a = ADAPTERS[r.game_id];
      const changes = []; let unchanged = 0;
      a.settings.forEach(([label, opts], i) => {
        const b = r.baseline.levels[i], n = r.best.levels[i];
        if (b === n) unchanged++; else changes.push({ label, help: HELP[label] || "", before: opts[b], after: opts[n] });
      });
      return { result: r, game_name: a.name, changes, unchanged };
    },
    history: () => [...store.results].reverse(),
    apply_result: ({ id }) => { store.results.forEach((r) => { const t = store.results.find((x) => x.id === id); if (r.game_id === t.game_id) r.applied = r.id === id; }); return "Done. Start the game and enjoy."; },
    restore_original: ({ gameId }) => { store.results.forEach((r) => { if (r.game_id === gameId) r.applied = false; }); return "Your original settings are back."; },
    save_preferences: ({ preferences }) => { store.preferences = preferences; return preferences; },
    set_onboarded: () => { store.onboarded = true; },
    open_folder: () => null,
    delete_history: () => { store.results = []; }
  };

  window.__FF_MOCK__ = {
    invoke: async (cmd, args) => {
      await sleep(60);
      if (!commands[cmd]) throw "unknown command " + cmd;
      return commands[cmd](args || {});
    },
    listen: async (ev, cb) => { (listeners[ev] = listeners[ev] || []).push(cb); return () => {}; },
    pressF9: () => { ready = true; emit("hotkey", "F9"); },
    screen
  };
  // Visible warning so nobody mistakes preview numbers for real measurements.
  document.addEventListener("DOMContentLoaded", () => {
    const b = document.createElement("div");
    b.setAttribute("role", "note");
    b.style.cssText = "position:fixed;left:0;right:0;bottom:0;z-index:100;background:#f2b33d;color:#111;font:600 12.5px Segoe UI,system-ui,sans-serif;padding:6px 14px;text-align:center";
    b.textContent = "Design preview in a browser · all numbers are made up · the installed app only shows values measured on your PC";
    document.body.appendChild(b);
    const st = document.createElement("style");
    st.textContent = ".main{padding-bottom:30px}.sticky-actions{bottom:0}";
    document.head.appendChild(st);
  });
  // In the preview, F9 on the keyboard works too.
  window.addEventListener("keydown", (e) => { if (e.key === "F9") { e.preventDefault(); window.__FF_MOCK__.pressF9(); } });
})();
