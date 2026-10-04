/* FrameForge desktop UI — plain JS, no external resources (works fully offline). */
(function () {
  "use strict";

  // ---------------------------------------------------------------- backend bridge
  const T = window.__TAURI__;
  const invoke = T ? T.core.invoke : window.__FF_MOCK__.invoke;
  const listen = T ? T.event.listen : window.__FF_MOCK__.listen;

  const $main = document.getElementById("main");
  const $overlay = document.getElementById("overlay-root");
  const esc = (s) => String(s == null ? "" : s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const fmt = (n, d = 0) => (n == null || isNaN(n) ? "–" : Number(n).toFixed(d));
  const pct = (a, b) => (a && b ? (a / b - 1) * 100 : 0);

  const state = { boot: null, view: "home", gameId: null, goal: "balanced", floor: 85, status: null, resultId: null };

  const GOALS = {
    smooth: { t: "Smoothest", tag: "For competitive games", d: "As many frames as possible. Great for shooters and fast games. Some visual detail is traded away." },
    balanced: { t: "Balanced", tag: "Recommended", d: "Smooth and good-looking. The best of both for most people." },
    beautiful: { t: "Best looking", tag: "For story games", d: "Keeps the game as pretty as possible while making sure it stays smooth enough to enjoy." }
  };
  const GOAL_FLOOR = { smooth: 65, balanced: 85, beautiful: 93 };

  // Refresh rate of the screen the app is on (rough, but good enough to set the target).
  async function measureHz() {
    return new Promise((resolve) => {
      let n = 0; const start = performance.now();
      const step = () => { n++; if (n < 60) requestAnimationFrame(step); else { const hz = 60000 / (performance.now() - start); resolve([60, 75, 120, 144, 165, 240, 360].reduce((a, b) => Math.abs(b - hz) < Math.abs(a - hz) ? b : a)); } };
      requestAnimationFrame(step);
    });
  }

  // ---------------------------------------------------------------- toasts & modals
  function toast(msg, kind) {
    const el = document.createElement("div");
    el.className = "toast " + (kind || "");
    el.setAttribute("role", "status");
    el.textContent = msg;
    document.body.appendChild(el);
    setTimeout(() => el.remove(), 4200);
  }
  function confirmBox(title, body, okText, okClass) {
    return new Promise((resolve) => {
      $overlay.innerHTML = `<div class="modal-bg"><div class="modal" role="dialog" aria-modal="true" aria-labelledby="m-title">
        <h2 id="m-title">${esc(title)}</h2><p>${esc(body)}</p>
        <div class="btn-row"><button class="btn ${okClass || "btn-primary"}" data-ok>${esc(okText || "OK")}</button><button class="btn btn-ghost" data-cancel>Cancel</button></div></div></div>`;
      const close = (v) => { $overlay.innerHTML = ""; resolve(v); };
      $overlay.querySelector("[data-ok]").onclick = () => close(true);
      $overlay.querySelector("[data-cancel]").onclick = () => close(false);
      $overlay.querySelector("[data-ok]").focus();
    });
  }
  async function call(cmd, args) {
    try { return await invoke(cmd, args); } catch (e) { toast(typeof e === "string" ? e : "Something went wrong.", ""); throw e; }
  }

  // ---------------------------------------------------------------- navigation
  function go(view, extra) {
    Object.assign(state, extra || {});
    state.view = view;
    document.querySelectorAll(".nav-item").forEach((b) => b.classList.toggle("active", b.dataset.view === view || (view === "setup" && b.dataset.view === "home") || (view === "result" && b.dataset.view === "history" && state.fromHistory)));
    render();
    $main.scrollTop = 0;
    $main.focus({ preventScroll: true });
  }
  document.getElementById("nav").addEventListener("click", (e) => {
    const b = e.target.closest(".nav-item"); if (!b) return;
    state.fromHistory = b.dataset.view === "history";
    go(b.dataset.view);
  });

  function render() {
    const v = state.view;
    if (v === "home") return renderHome();
    if (v === "setup") return renderSetup();
    if (v === "live") return renderLive();
    if (v === "result") return renderResult();
    if (v === "history") return renderHistory();
    if (v === "settings") return renderSettings();
    if (v === "help") return renderHelp();
  }

  // ---------------------------------------------------------------- onboarding
  function onboarding() {
    const slides = [
      { h: "More FPS. You decide how it looks.", lead: "FrameForge tests graphics settings on your own PC, measures the real frame rate of each, and shows you the fastest combination that still meets the picture quality you chose. No guessing, no forum guides.",
        pts: [["1", "You pick a game and what you care about", "Smoothest, balanced, or best looking."], ["2", "You play for a minute, a few times", "FrameForge changes settings between tests and counts your frames."], ["3", "You get measured results, not promises", "The winner is measured twice. You see before/after numbers and every setting that changed, then decide."]] },
      { h: "Safe by design.", lead: "Your games and your PC are always protected.",
        pts: [["✓", "Your settings are backed up first", "Your original settings are put back after every session. If FrameForge or your PC shuts down mid-test, they're put back the next time FrameForge starts."], ["✓", "Temperature watch (NVIDIA cards)", "On NVIDIA graphics cards FrameForge checks the temperature every 2 seconds and stops a test above your limit. On other cards the card's own built-in protection applies."], ["✓", "Doesn't touch the game", "Frames are counted with Intel's PresentMon, which reads timing from Windows. Nothing is injected into the game. Still, only test in offline or practice modes."]] },
      { h: "100% on your PC.", lead: "No account. No internet needed. Nothing is uploaded.",
        pts: [["◼", "The Smart Tuner runs on your computer", "A built-in AI model (TabPFN) learns from your own tests, right on your PC."], ["◼", "Your data stays here", "Results are saved in a file on this PC. Delete them any time in Settings."], ["◼", "Plan on 15–30 minutes", "Each test is about a minute of play plus the game's loading time. You can stop at any point."]] }
    ];
    let i = 0;
    const draw = () => {
      const s = slides[i];
      $overlay.innerHTML = `<div class="onboard"><div class="onboard-card" role="dialog" aria-modal="true" aria-labelledby="ob-h">
        <div class="onboard-body"><h1 id="ob-h">${esc(s.h)}</h1><p class="lead">${esc(s.lead)}</p>
        <div class="onboard-points">${s.pts.map((p) => `<div><span class="ic">${esc(p[0])}</span><p><b>${esc(p[1])}</b><span>${esc(p[2])}</span></p></div>`).join("")}</div></div>
        <div class="onboard-foot"><div class="dots">${slides.map((_, k) => `<i class="${k === i ? "on" : ""}"></i>`).join("")}</div>
        <div class="btn-row">${i > 0 ? '<button class="btn btn-ghost" data-back>Back</button>' : ""}<button class="btn btn-primary" data-next>${i < slides.length - 1 ? "Next" : "Let's go"}</button></div></div></div></div>`;
      $overlay.querySelector("[data-next]").onclick = async () => { if (i < slides.length - 1) { i++; draw(); } else { $overlay.innerHTML = ""; await invoke("set_onboarded"); } };
      const back = $overlay.querySelector("[data-back]"); if (back) back.onclick = () => { i--; draw(); };
      $overlay.querySelector("[data-next]").focus();
    };
    draw();
  }

  // ---------------------------------------------------------------- home
  function renderHome() {
    const b = state.boot, hw = b.hardware;
    const notices = [];
    if (b.recovered_session) notices.push(`<div class="notice good"><div><b>Your game settings were restored.</b>FrameForge was closed during a test last time, so we put your original settings back.</div></div>`);
    if (!b.measuring_tool_installed) notices.push(`<div class="notice"><div><b>A part of FrameForge is missing.</b>Please reinstall FrameForge to fix this. Your games were not changed.</div></div>`);
    if (!b.ai_engine_installed) notices.push(`<div class="notice warn"><div><b>Running in Basic mode.</b>The Smart Tuner AI isn't installed, so FrameForge uses a simpler method. It still works, it just needs a few more tests.</div></div>`);
    if (!hw.live_sensors) notices.push(`<div class="notice warn"><div><b>We can't read your graphics card's temperature.</b>Testing still works. Your graphics card's own safety limits stay active.</div></div>`);

    $main.innerHTML = `<section class="view">
      <div class="view-head"><div><h1>My games</h1><p class="muted">Pick a game. FrameForge finds the best settings for this PC.</p></div>
        <button class="btn btn-sm" id="refresh">Look for games again</button></div>
      ${notices.join("")}
      <div class="pc" aria-label="Your PC">
        <div><div class="k">Your PC</div><div class="tier">${esc(hw.tier)}</div></div>
        <div><div class="k">Graphics card</div><div class="v" title="${esc(hw.gpu)}">${esc(hw.gpu)}</div><div class="dim">${fmt(hw.vram_gb, 0)} GB memory</div></div>
        <div><div class="k">Processor</div><div class="v" title="${esc(hw.cpu)}">${esc(hw.cpu.replace(/\s+\d+-Core Processor/i, ""))}</div><div class="dim">${hw.cpu_cores} cores</div></div>
        <div><div class="k">Memory</div><div class="v">${fmt(hw.ram_gb, 0)} GB</div><div class="dim">${esc(hw.os)}</div></div>
      </div>
      <div class="games">${b.games.map(gameCard).join("")}</div>
      <p class="dim" style="margin-top:20px;font-size:13px">Don't see your game? FrameForge currently supports ${b.games.length} games. More are added with each update.</p>
    </section>`;
    $main.querySelector("#refresh").onclick = async () => { b.games = await call("refresh_games"); renderHome(); toast("Game list updated.", "good"); };
    $main.querySelectorAll("[data-start]").forEach((el) => el.onclick = () => go("setup", { gameId: el.dataset.start }));
    $main.querySelectorAll("[data-result]").forEach((el) => el.onclick = () => go("result", { resultId: el.dataset.result, fromHistory: false }));
  }

  function gameCard(g) {
    const initials = g.name.split(/\s+/).map((w) => w[0]).join("").slice(0, 3);
    const r = g.last_result;
    let status = "", action = "";
    if (!g.installed) {
      status = '<span class="badge">Not installed</span>';
      action = '<button class="btn btn-sm" disabled>Install it in Steam first</button>';
    } else if (!g.settings_found) {
      status = '<span class="badge badge-warn">Start it once first</span>';
      action = `<button class="btn btn-sm btn-primary" data-start="${esc(g.id)}">Find best settings</button>`;
    } else {
      status = r && r.applied ? '<span class="badge badge-good">Optimized</span>' : '<span class="badge">Ready</span>';
      action = `<button class="btn btn-sm btn-primary" data-start="${esc(g.id)}">${r ? "Test again" : "Find best settings"}</button>`;
    }
    const c = r && r.confirmed;
    const gain = r ? (c ? pct(c.best_fps, c.baseline_fps) : pct(r.best.fps_avg, r.baseline.fps_avg)) : 0;
    return `<article class="game">
      <div class="game-art"><span class="accent"></span><span class="initials" aria-hidden="true">${esc(initials)}</span><h3>${esc(g.name)}</h3></div>
      ${g.support !== "verified" ? '<div class="dim" style="padding:8px 18px 0;font-size:12.5px">Early support: settings keys not yet confirmed on every game version. FrameForge checks after each test and stops if the game ignores a change.</div>' : ""}
      <div class="game-body"><p>${esc(g.short)}</p>
        ${r ? `<div class="game-result"><span class="big">+${fmt(Math.max(0, gain))}%</span><span class="muted">FPS last time · ${fmt(c ? c.baseline_fps : r.baseline.fps_avg)} → ${fmt(c ? c.best_fps : r.best.fps_avg)}</span></div>` : '<p class="dim">Not tested yet.</p>'}
      </div>
      <div class="game-foot">${status}<div class="btn-row">${r ? `<button class="btn btn-sm btn-ghost" data-result="${esc(r.id)}">See result</button>` : ""}${action}</div></div>
    </article>`;
  }

  // ---------------------------------------------------------------- setup
  function renderSetup() {
    const g = state.boot.games.find((x) => x.id === state.gameId);
    if (!g) return go("home");
    const testMin = { short: 40, normal: 60, thorough: 90 }[state.boot.preferences.test_length] || 60;
    const maxT = state.boot.preferences.max_tests;
    const est = Math.round((Math.min(10, maxT) * (testMin + 45)) / 60);
    $main.innerHTML = `<section class="view">
      <button class="back" id="back">← My games</button>
      <div class="view-head"><div><h1>${esc(g.name)}</h1><p class="muted">Two quick choices, then we start.</p></div></div>
      ${!g.settings_found ? `<div class="notice warn"><div><b>Start ${esc(g.name)} once before testing.</b>We need the game to create its settings file. Open the game, go to the graphics menu, change anything, and close it. Then come back here.</div></div>` : ""}
      <div class="panel"><div class="panel-title">1 · What matters most to you?</div>
        <div class="choices" role="radiogroup">${Object.entries(GOALS).map(([k, v]) => `<button class="choice ${state.goal === k ? "selected" : ""}" role="radio" aria-checked="${state.goal === k}" data-goal="${k}"><span class="tag">${esc(v.tag)}</span><span class="t">${esc(v.t)}</span><span class="d">${esc(v.d)}</span></button>`).join("")}</div>
      </div>
      <div class="panel"><div class="panel-title">2 · How much picture quality must we keep?</div>
        <p class="muted">FrameForge will never pick settings that look worse than this. <span class="slider-value" id="floor-txt"></span></p>
        <div class="slider-wrap"><input type="range" id="floor" min="50" max="98" step="1" value="${state.floor}" aria-label="Minimum picture quality"></div>
        <div class="slider-labels"><span>Speed first</span><span>Keep almost everything</span></div>
      </div>
      <div class="panel"><div class="panel-title">What will happen</div>
        <ol class="steps-list">
          <li><div><b>We save a copy of your current settings.</b> <span class="muted">You can always go back with one click.</span></div></li>
          <li><div><b>FrameForge starts ${esc(g.name)} for you.</b> <span class="muted">Each test uses slightly different settings.</span></div></li>
          <li><div><b>Go to the test spot and press <span class="kbd">F9</span>.</b> <span class="muted">${esc(g.where_to_test)}</span></div></li>
          <li><div><b>Keep playing for about ${testMin} seconds.</b> <span class="muted">FrameForge counts frames, then closes the game and picks the next test.</span></div></li>
          <li><div><b>Repeat a few times.</b> <span class="muted">Up to ${maxT} tests plus 2 check tests. Roughly ${est}–${est * 2} minutes depending on how long ${esc(g.name)} takes to load. You can stop whenever you like.</span></div></li>
        </ol>
      </div>
      <div class="sticky-actions"><button class="btn btn-primary btn-lg" id="go" ${g.settings_found && state.boot.measuring_tool_installed ? "" : "disabled"}>Start testing</button>
        <span class="muted">Save any progress in ${esc(g.name)} and close it first. We'll start it for you.</span></div>
    </section>`;
    const floorTxt = () => {
      const v = state.floor;
      const label = v >= 93 ? "Practically the same as max settings" : v >= 85 ? "Very close to max settings" : v >= 72 ? "A small, noticeable step down" : "Clearly simpler graphics";
      $main.querySelector("#floor-txt").textContent = `${v}% · ${label}`;
    };
    floorTxt();
    $main.querySelector("#back").onclick = () => go("home");
    $main.querySelectorAll("[data-goal]").forEach((el) => el.onclick = () => { state.goal = el.dataset.goal; state.floor = GOAL_FLOOR[state.goal]; renderSetup(); });
    $main.querySelector("#floor").oninput = (e) => { state.floor = Number(e.target.value); floorTxt(); };
    $main.querySelector("#go").onclick = async () => {
      const hz = state.boot.preferences.refresh_hz || (await measureHz());
      await call("start_session", { gameId: g.id, goal: state.goal, qualityFloor: state.floor, refreshHz: hz });
      document.getElementById("nav-live").classList.remove("hidden");
      go("live");
    };
  }

  // ---------------------------------------------------------------- live session
  const PHASE_LABEL = { preparing: "Getting ready", applying: "Changing settings", launching: "Starting the game", waiting: "Waiting for you", warmup: "Warming up", measuring: "Measuring", thinking: "Smart Tuner", checking: "Double-checking", finished: "Finished", stopped: "Stopped", failed: "Problem" };

  function renderLive() {
    const s = state.status || { phase: "preparing", runs: [] };
    if (!s.active && !["finished", "stopped", "failed"].includes(s.phase)) return go("home");
    const runs = s.runs || [];
    const maxFps = Math.max(1, ...runs.map((r) => r.fps_avg));
    const bestRun = runs.filter((r) => r.safe && r.settings_held !== false && r.kind !== "check").reduce((a, r) => (!a || r.fps_avg > a.fps_avg ? r : a), null);
    const gain = s.baseline_fps && s.best_fps ? pct(s.best_fps, s.baseline_fps) : null;
    const progress = s.phase === "measuring" ? (s.seconds_done / Math.max(1, s.seconds_total)) * 100 : s.phase === "finished" ? 100 : 0;
    const ended = ["finished", "stopped", "failed"].includes(s.phase);

    $main.innerHTML = `<section class="view">
      <div class="view-head"><div><h1>${esc(s.game_name || "Testing")}</h1><p class="muted">${s.max_tests ? `Test ${s.test_index || 0} of up to ${s.max_tests}` : ""} · ${esc(s.engine || "")}</p></div>
        ${ended ? "" : '<button class="btn btn-danger" id="stop">Stop and put everything back</button>'}</div>
      <div class="live">
        <div>
          <div class="live-hero" aria-live="polite">
            <div class="phase-label">${esc(PHASE_LABEL[s.phase] || s.phase)}</div>
            <h1>${esc(s.headline)}</h1>
            <p class="detail">${esc(s.detail)}</p>
            ${s.phase === "waiting" ? `<div class="press"><span class="kbd">F9</span><div><b>Press F9 in the game when you're at the test spot.</b><div class="muted" style="font-size:13.5px">F9 only works for FrameForge while it's waiting for you.</div></div></div>
              <div class="btn-row" style="margin-top:12px"><button class="btn btn-sm" id="ready">I'm in position (instead of F9)</button></div>` : ""}
            ${s.phase === "measuring" ? `<div class="progress" role="progressbar" aria-valuenow="${Math.round(progress)}" aria-valuemin="0" aria-valuemax="100"><div style="width:${progress}%"></div></div>
              <div class="progress-meta"><span>${s.seconds_done || 0} of ${s.seconds_total} seconds</span><span>${s.gpu_temp ? "Graphics card " + fmt(s.gpu_temp) + " °C" : s.temp_guard === false ? "Temperature not readable on this card" : ""}</span></div>` : ""}
            ${s.phase === "finished" ? `<div class="btn-row" style="margin-top:22px"><button class="btn btn-primary btn-lg" id="see">See your result</button></div>` : ""}
            ${s.phase === "stopped" || s.phase === "failed" ? `<div class="btn-row" style="margin-top:22px"><button class="btn btn-primary" id="again">Try again</button><button class="btn" id="home">Back to my games</button>${s.phase === "failed" ? '<button class="btn btn-ghost" id="logs">Open log folder</button>' : ""}</div>` : ""}
          </div>
          <div class="runs"><div class="runs-head"><span>Tests so far</span><span class="dim">frames per second</span></div>
            ${runs.length ? runs.map((r) => `<div class="run ${bestRun && r.index === bestRun.index ? "best" : ""}"><span class="n">#${r.index}</span>
              <div><div>${r.kind === "check" ? "Check run" : r.index === 1 ? "Your current settings" : "Est. picture quality " + fmt(r.quality) + "%"} ${r.note ? `<span class="badge badge-warn">${esc(r.note)}</span>` : ""}</div><div class="bar"><div style="width:${(r.fps_avg / maxFps) * 100}%"></div></div></div>
              <span class="fps">${fmt(r.fps_avg)}</span></div>`).join("") : '<div class="empty">Results appear here after the first test.</div>'}
          </div>
        </div>
        <aside>
          <div class="live-stats">
            <div><div class="k">Where you started</div><div class="v">${fmt(s.baseline_fps)}</div></div>
            <div><div class="k">Best so far</div><div class="v" style="color:var(--crimson)">${fmt(s.best_fps)}</div></div>
            <div><div class="k">Improvement</div><div class="v">${gain != null ? (gain >= 0 ? "+" : "") + fmt(gain) + "%" : "–"}</div></div>
            <div><div class="k">Tests done</div><div class="v">${runs.filter((r) => r.kind !== "check").length}</div></div>
          </div>
          <div class="panel" style="margin-top:16px"><div class="panel-title">Good to know</div>
            <p class="muted" style="font-size:14px">Do the same thing in every test, in the same spot. That keeps the comparison fair.</p>
            <p class="muted" style="font-size:14px;margin-top:10px">Your original settings come back automatically when testing ends, even if you stop early.</p>
            <p class="muted" style="font-size:14px;margin-top:10px">"Best so far" is a single measurement. The winner is measured again at the end before we show the final result.</p>
          </div>
        </aside>
      </div>
    </section>`;
    const on = (id, fn) => { const el = $main.querySelector(id); if (el) el.onclick = fn; };
    on("#stop", async () => { if (await confirmBox("Stop testing?", "Your original settings will be put back. Results so far will not be saved.", "Stop testing", "btn-danger")) call("stop_session"); });
    on("#ready", () => call("player_ready"));
    on("#see", () => go("result", { resultId: s.result_id, fromHistory: false }));
    on("#again", () => go("setup", { gameId: s.game_id }));
    on("#home", () => go("home"));
    on("#logs", () => call("open_folder", { which: "logs" }));
  }

  // ---------------------------------------------------------------- result
  async function renderResult() {
    $main.innerHTML = '<section class="view"><p class="muted">Loading…</p></section>';
    let v;
    try { v = await invoke("get_result", { id: state.resultId }); } catch (e) { $main.innerHTML = `<section class="view"><div class="notice"><div><b>We couldn't open this result.</b>${esc(e)}</div></div></section>`; return; }
    const r = v.result, b = r.baseline, n = r.best, c = r.confirmed;
    // Prefer the confirmed (two-run average) numbers whenever they exist.
    const bf = c ? c.baseline_fps : b.fps_avg, nf = c ? c.best_fps : n.fps_avg;
    const bl = c ? c.baseline_low : b.fps_low, nl = c ? c.best_low : n.fps_low;
    const gain = pct(nf, bf), lowGain = pct(nl, bl);
    const noise = c ? Math.max(3, c.noise_pct) : 5;
    const qLoss = b.quality - n.quality;
    const already = n.index === b.index || gain <= noise;
    const searchRuns = r.runs.filter((x) => x.kind !== "check").length;
    $main.innerHTML = `<section class="view">
      <button class="back" id="back">← ${state.fromHistory ? "Past results" : "My games"}</button>
      <div class="view-head"><div><h1>${esc(v.game_name)}</h1><p class="muted">Tested ${new Date(r.finished_at).toLocaleString()} · ${searchRuns ? searchRuns + " tests · " : ""}${esc(GOALS[r.goal] ? GOALS[r.goal].t : r.goal)} · ${esc(r.engine)}</p></div>
        ${r.applied ? '<span class="badge badge-good">In use</span>' : ""}</div>
      <div class="result-hero">
        <div class="result-side"><div class="k">Before</div><div class="fps">${fmt(bf)}</div><div class="unit">frames per second</div></div>
        <div class="result-arrow" aria-hidden="true">→</div>
        <div class="result-side after"><div class="k">After</div><div class="fps">${fmt(nf)}</div><div class="unit">frames per second</div></div>
      </div>
      <p class="gain-line">${already ? "Your current settings are already about as fast as this PC gets at the picture quality you chose. Nothing to change." : `<b>${fmt(gain)}% more frames per second</b>, and the slowest moments are ${lowGain > noise ? fmt(lowGain) + "% faster" : "about the same"}.`}</p>
      <div class="metrics">
        <div><div class="k">Average frame rate</div><div class="v">${gain >= 0 ? "+" : ""}${fmt(gain)}%</div><div class="s">${fmt(bf)} → ${fmt(nf)} FPS</div></div>
        <div><div class="k">Slowest 1% of frames</div><div class="v">${lowGain >= 0 ? "+" : ""}${fmt(lowGain)}%</div><div class="s">${fmt(bl)} → ${fmt(nl)} FPS · this is what you feel as stutter</div></div>
        <div><div class="k">Estimated picture quality</div><div class="v">${fmt(n.quality)}%</div><div class="s">${qLoss > 0.5 ? "of max settings · you chose at least " + fmt(r.quality_floor) + "%" : "same as before"}</div></div>
      </div>
      <p class="dim" style="font-size:13px;margin:-4px 0 16px">${c ? `Both settings were measured twice and averaged. Two runs of the same settings differed by up to ${fmt(c.noise_pct, 1)}%, so we only count a gain above ${fmt(noise, 1)}% as real.` : "Measured once. Gains under 5% may just be normal run-to-run variation."}</p>
      <div class="panel"><div class="panel-title">What changed (${v.changes.length} setting${v.changes.length === 1 ? "" : "s"})</div>
        ${v.changes.length ? `<table class="changes"><thead><tr><th>Setting</th><th>Before</th><th>After</th></tr></thead><tbody>
          ${v.changes.map((c) => `<tr><td>${esc(c.label)}<div class="help">${esc(c.help)}</div></td><td>${esc(c.before)}</td><td class="after">${esc(c.after)}</td></tr>`).join("")}</tbody></table>
          <p class="dim" style="margin-top:10px;font-size:13px">${v.unchanged} other setting${v.unchanged === 1 ? "" : "s"} stay the same.</p>` : '<p class="muted">Nothing needs to change.</p>'}
      </div>
      <div class="sticky-actions">
        ${already ? "" : `<button class="btn btn-primary btn-lg" id="apply" ${r.applied ? "disabled" : ""}>${r.applied ? "These settings are in use" : "Use these settings"}</button>`}
        <button class="btn" id="restore">Put my original settings back</button>
        <span class="dim" style="font-size:13px">Close ${esc(v.game_name)} first. You can switch back any time.</span>
      </div>
    </section>`;
    $main.querySelector("#back").onclick = () => go(state.fromHistory ? "history" : "home");
    const apply = $main.querySelector("#apply");
    if (apply) apply.onclick = async () => { const m = await call("apply_result", { id: r.id }); toast(m, "good"); state.boot.games = await invoke("refresh_games"); renderResult(); };
    $main.querySelector("#restore").onclick = async () => {
      if (!(await confirmBox("Put your original settings back?", "This brings back the settings you had before you first used FrameForge with this game.", "Put them back"))) return;
      const m = await call("restore_original", { gameId: r.game_id }); toast(m, "good"); state.boot.games = await invoke("refresh_games"); renderResult();
    };
  }

  // ---------------------------------------------------------------- history
  async function renderHistory() {
    const list = await call("history");
    const name = (id) => (state.boot.games.find((g) => g.id === id) || { name: id }).name;
    $main.innerHTML = `<section class="view">
      <div class="view-head"><div><h1>Past results</h1><p class="muted">Every test session you've finished. Stored only on this PC.</p></div></div>
      <div class="list">${list.filter((r) => r.best).length ? list.filter((r) => r.best).map((r) => `<div class="list-row">
        <div><b>${esc(name(r.game_id))}</b><div class="dim" style="font-size:13px">${new Date(r.finished_at).toLocaleString()}</div></div>
        <div>${fmt(r.baseline.fps_avg)} → <b>${fmt(r.best.fps_avg)}</b> FPS <span class="muted">(+${fmt(Math.max(0, pct(r.best.fps_avg, r.baseline.fps_avg)))}%)</span></div>
        <div>${r.applied ? '<span class="badge badge-good">In use</span>' : '<span class="badge">Not in use</span>'}</div>
        <button class="btn btn-sm" data-id="${esc(r.id)}">Open</button></div>`).join("") : '<div class="empty">No results yet. Pick a game on the My games page to start.</div>'}</div>
    </section>`;
    $main.querySelectorAll("[data-id]").forEach((el) => el.onclick = () => go("result", { resultId: el.dataset.id, fromHistory: true }));
  }

  // ---------------------------------------------------------------- settings
  function renderSettings() {
    const p = Object.assign({}, state.boot.preferences);
    const seg = (name, opts) => `<div class="seg" data-seg="${name}">${opts.map(([v, l]) => `<button type="button" data-v="${v}" class="${String(p[name]) === String(v) ? "on" : ""}">${esc(l)}</button>`).join("")}</div>`;
    $main.innerHTML = `<section class="view">
      <div class="view-head"><div><h1>Settings</h1><p class="muted">The defaults work well for almost everyone.</p></div></div>
      <div class="panel">
        <div class="field"><div><b>Length of each test</b><p>Longer tests give steadier results but take more time.</p></div>${seg("test_length", [["short", "Short"], ["normal", "Normal"], ["thorough", "Thorough"]])}</div>
        <div class="field"><div><b>Maximum number of tests</b><p>FrameForge usually stops earlier, once it's confident it found the best.</p></div>${seg("max_tests", [[8, "8"], [12, "12"], [20, "20"]])}</div>
        <div class="field"><div><b>Temperature safety limit</b><p>${state.boot.hardware.live_sensors ? "A test stops if your graphics card gets hotter than this (checked every 2 seconds)." : "Only works on NVIDIA graphics cards. Your card's own built-in protection still applies."}</p></div>${seg("temp_limit_c", [[80, "80 °C"], [87, "87 °C"], [92, "92 °C"]])}</div>
        <div class="field"><div><b>Screen refresh rate</b><p>Used to set a smoothness target. "Automatic" detects it for you.</p></div>${seg("refresh_hz", [[0, "Automatic"], [60, "60"], [144, "144"], [240, "240"]])}</div>
      </div>
      <div class="panel"><div class="panel-title">Your data</div>
        <p class="muted">FrameForge works fully offline. It has no account and sends nothing anywhere. Results, backups and logs are saved in a folder on this PC.</p>
        <div class="btn-row" style="margin-top:14px"><button class="btn btn-sm" data-open="data">Open data folder</button><button class="btn btn-sm" data-open="backups">Open settings backups</button><button class="btn btn-sm" data-open="logs">Open log folder</button><button class="btn btn-sm btn-danger" id="del">Delete all results</button></div>
      </div>
      <div class="panel"><div class="panel-title">About</div>
        <p class="muted">FrameForge ${esc(state.boot.version)} · Smart Tuner: ${state.boot.ai_engine_installed ? "AI (TabPFN by Prior Labs), running on this PC" : "Basic mode"} · Frame counting: PresentMon (Intel, MIT licence).</p>
      </div>
    </section>`;
    $main.querySelectorAll("[data-seg]").forEach((g) => g.addEventListener("click", async (e) => {
      const b = e.target.closest("button"); if (!b) return;
      const key = g.dataset.seg; p[key] = key === "test_length" ? b.dataset.v : Number(b.dataset.v);
      state.boot.preferences = await call("save_preferences", { preferences: p });
      renderSettings(); toast("Saved.", "good");
    }));
    $main.querySelectorAll("[data-open]").forEach((b) => b.onclick = () => call("open_folder", { which: b.dataset.open }));
    $main.querySelector("#del").onclick = async () => { if (await confirmBox("Delete all results?", "Your game settings stay as they are. Only the result history is removed.", "Delete", "btn-danger")) { await call("delete_history"); state.boot.games = await invoke("refresh_games"); toast("All results deleted.", "good"); } };
  }

  // ---------------------------------------------------------------- help
  function renderHelp() {
    const qa = [
      ["Is this safe for my games?", "Yes. Before changing anything, FrameForge saves a copy of your settings. When testing ends, or if you stop, your original settings are put back. If FrameForge or your PC shuts down mid-test, they're put back the next time you open FrameForge. Nothing is changed for good until you click \"Use these settings\"."],
      ["Can I get banned?", "FrameForge doesn't read or change the game's memory and injects nothing. Frames are counted with Intel's PresentMon, which reads timing events from Windows, the same mechanism Windows' own performance tools use. FrameForge only edits the game's normal settings file, the same file the in-game menu writes. We can't promise how every anti-cheat system behaves, so only test in offline modes or practice maps, never in ranked matches."],
      ["Does it need the internet?", "No. Everything, including the AI, runs on your PC. FrameForge has no account and uploads nothing."],
      ["What does the Smart Tuner do?", "It's an AI model (TabPFN) that is very good at learning from just a handful of examples. After each test it predicts how fast thousands of other setting combinations would run on your PC, and picks the most promising one to test next. So each test you play is chosen to teach it as much as possible, instead of trying settings one by one. TabPFN was published in the journal Nature in 2025. Its authors showed it beats tuned standard methods on small data tables, which is exactly what a handful of game tests is."],
      ["What is \"picture quality %\"?", "An estimate of how much visual detail is kept compared with every setting on max (100%). It's calculated from how much each setting typically affects the picture, not measured from your screen, so treat it as a guide. FrameForge never picks settings below the score you choose, and the result page lists every change so you can judge for yourself."],
      ["Why press F9?", "So every test measures the same moment in the game. Go to the same spot, press F9, and do the same thing each time. F9 only works for FrameForge while it's waiting for you, so it won't clash with the game."],
      ["It says \"Windows blocked the frame counter\".", "Restart your PC once after installing. Windows needs a restart to give FrameForge permission to count frames."],
      ["My result didn't change much.", "Then your settings were already close to the fastest your PC can manage at that picture quality. This often happens when the processor, not the graphics card, is what limits your frame rate. Lowering graphics settings can't fix that."],
      ["How do I know the numbers are real?", "Every number in FrameForge is measured on your PC while you play. Nothing is predicted or estimated. At the end, the winner and your original settings are both measured a second time. You only see an improvement if it's bigger than the difference between two runs of the same settings."]
    ];
    $main.innerHTML = `<section class="view"><div class="view-head"><div><h1>Help</h1><p class="muted">Quick answers to common questions.</p></div></div>
      <div class="list">${qa.map(([q, a]) => `<details class="list-row" style="display:block"><summary style="cursor:pointer;font-weight:600">${esc(q)}</summary><p class="muted" style="margin-top:8px">${esc(a)}</p></details>`).join("")}</div>
      <div class="btn-row" style="margin-top:18px"><button class="btn" id="tour">Show the welcome tour again</button><button class="btn btn-ghost" id="logs">Open log folder</button></div></section>`;
    $main.querySelector("#tour").onclick = onboarding;
    $main.querySelector("#logs").onclick = () => call("open_folder", { which: "logs" });
  }

  // ---------------------------------------------------------------- boot
  async function boot() {
    state.boot = await invoke("get_bootstrap");
    document.getElementById("app-version").textContent = "Version " + state.boot.version;
    state.status = state.boot.status;
    if (state.status && state.status.active) { document.getElementById("nav-live").classList.remove("hidden"); go("live"); } else go("home");
    if (!state.boot.onboarded) onboarding();
    // Browser preview only: jump straight to a screen (?screen=...).
    const pv = !T && window.__FF_MOCK__.screen;
    if (pv === "setup") go("setup", { gameId: "palworld" });
    else if (pv === "result") go("result", { resultId: "r-old" });
    else if (["history", "settings", "help"].includes(pv)) go(pv);

    await listen("session", (e) => {
      state.status = e.payload;
      const navLive = document.getElementById("nav-live");
      navLive.classList.toggle("hidden", !state.status.active && !["finished", "stopped", "failed"].includes(state.status.phase));
      if (state.status.phase === "finished") invoke("refresh_games").then((g) => (state.boot.games = g));
      if (state.view === "live") renderLive();
    });
    await listen("hotkey", () => { if (state.view !== "live") toast("F9 received. Measuring will start shortly.", "good"); });
  }
  boot().catch((e) => { $main.innerHTML = `<section class="view"><div class="notice"><div><b>FrameForge couldn't start.</b>${esc(e)}</div></div></section>`; });
})();
