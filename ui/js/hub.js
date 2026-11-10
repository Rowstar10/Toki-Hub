/* Toki Hub UI: draws the library from the hub's state and calls the hub's commands in Rust. */
"use strict";

const ICON_PATHS = {
  play: '<path d="M8 5.5v13l11-6.5z" fill="currentColor" stroke="none"/>',
  download: '<path d="M12 4v11m0 0-4.5-4.5M12 15l4.5-4.5M5 19.5h14"/>',
  folder: '<path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
  log: '<path d="M7 3.5h7l4 4v13H7a1.5 1.5 0 0 1-1.5-1.5V5A1.5 1.5 0 0 1 7 3.5zM14 3.5V8h4M9 12h6M9 15.5h6"/>',
  trash: '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5M10 11v5M14 11v5"/>',
  stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="2" fill="currentColor" stroke="none"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  settings: '<path d="M4 7h9M18 7h2M4 17h3M12 17h8"/><circle cx="15.5" cy="7" r="2.3"/><circle cx="9.5" cy="17" r="2.3"/>',
  refresh: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4h-4"/>',
  warning: '<path d="M12 4 2.8 19.5h18.4zM12 10v4.5M12 17.2v.3"/>',
  source: '<path d="M7 18.5a4.5 4.5 0 0 1-.6-8.96A6 6 0 0 1 18 9.5a4.5 4.5 0 0 1-.5 9z"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8v.2"/>',
};

const PLATFORM_NAMES = { windows: "Windows", macos: "macOS", linux: "Linux", any: "any platform" };

const ui = {
  data: null,               // last full state from the hub
  gameId: null,             // selected game
  versionFor: {},           // game id -> selected version
  lastEvent: 0,
  taskStatus: {},           // task id -> last status seen, to notice when one finishes
  renderKey: "",
  connected: true,
  pollTimer: null,
  changelogs: {},           // game id -> "loading", then {versions, from, error} from the hub
};

/* ---------- Talking to the hub ---------- */

// Each command's argument names, in the order the calls below pass them (see src-tauri/src/main.rs).
const COMMAND_ARGS = {
  state: [], poll: ["afterEvent"], reload: [], changelog: ["gameId"],
  install: ["gameId", "version"], cancel: ["taskId"], uninstall: ["gameId", "version"],
  launch: ["gameId", "version"], stop: ["gameId", "version"],
  open_folder: ["gameId", "version"], open_log: ["gameId", "version"], open_place: ["which"],
  save_settings: ["changes"], clear_token: [], pick_folder: [],
  check_for_updates: [], download_update: [], apply_update: [],
};

async function api(name, ...args) {
  const named = {};
  COMMAND_ARGS[name].forEach((key, i) => { named[key] = args[i]; });
  try {
    return await window.__TAURI__.core.invoke(name, named);
  } catch (error) {
    throw new Error(typeof error === "string" ? error : error?.message || "Something went wrong.");
  }
}

// Catalog images come through the hub's own "tokicatalog" protocol.
const catalogUrl = (relative) => window.__TAURI__.core.convertFileSrc(relative, "tokicatalog");

async function attempt(action, failTitle) {
  try {
    return await action();
  } catch (error) {
    toast({ kind: "error", title: failTitle, text: error.message });
    Toki.say("error");
    return undefined;
  }
}

/* ---------- Small helpers ---------- */

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key === "icon") node.append(icon(value));
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (key === "style") node.style.cssText = value;
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function icon(name) {
  const span = document.createElement("span");
  span.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"
    stroke-linejoin="round" aria-hidden="true">${ICON_PATHS[name]}</svg>`;
  return span.firstElementChild;
}

function button(label, { kind = "ghost", iconName, onclick, disabled, title, small } = {}) {
  return el("button", { type: "button", class: `btn btn-${kind}${small ? " btn-small" : ""}`, onclick, disabled, title },
    el("span", {}, iconName ? icon(iconName) : null, label));
}

const vlabel = (v) => (/^\d/.test(v) ? `v${v}` : v);
const platformName = (p) => PLATFORM_NAMES[p] || p;

function formatBytes(n) {
  if (!n) return "";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i += 1; }
  return `${n.toFixed(n >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

function formatDate(iso) {
  if (!iso) return "";
  const date = new Date(/^\d{4}-\d{2}-\d{2}$/.test(iso) ? `${iso}T12:00:00` : iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

function store(key, value) {
  try { localStorage.setItem(`tokihub.${key}`, value); } catch { /* storage unavailable: fine */ }
}
function recall(key) {
  try { return localStorage.getItem(`tokihub.${key}`); } catch { return null; }
}

/* ---------- Reading the state ---------- */

const games = () => (ui.data ? ui.data.games : []);
const gameById = (id) => games().find((g) => g.id === id);
const installedInfo = (gameId, version) => ui.data.installed[gameId]?.[version];
const isRunning = (gameId, version) => ui.data.running.some((r) => r.game_id === gameId && r.version === version);

function taskFor(gameId, version) {
  return ui.data.tasks.find((t) => t.game_id === gameId && t.version === version);
}

function activeTask(gameId, version) {
  const task = taskFor(gameId, version);
  return task && ["queued", "downloading", "installing"].includes(task.status) ? task : null;
}

function selectedVersion(game) {
  const chosen = game.versions.find((v) => v.version === ui.versionFor[game.id]);
  if (chosen) return chosen;
  return game.versions.find((v) => isRunning(game.id, v.version))
    || game.versions.find((v) => installedInfo(game.id, v.version))
    || game.versions.find((v) => v.build)      // newest one that runs on this computer
    || game.versions[0] || null;
}

function gameStatus(game) {
  const running = game.versions.find((v) => isRunning(game.id, v.version));
  if (running) return { text: `Running ${vlabel(running.version)}`, cls: "running" };
  const busy = game.versions.map((v) => activeTask(game.id, v.version)).find(Boolean);
  if (busy) return { text: `${taskVerb(busy)} ${vlabel(busy.version)}${progressText(busy, true)}`, cls: "busy" };
  const installed = game.versions.filter((v) => installedInfo(game.id, v.version));
  if (installed.length === 1) return { text: `Installed ${vlabel(installed[0].version)}`, cls: "installed" };
  if (installed.length > 1) return { text: `${installed.length} versions installed`, cls: "installed" };
  if (!game.versions.length) return { text: "Coming soon", cls: "" };
  return { text: "Not installed", cls: "" };
}

const taskVerb = (task) => (task.status === "installing" ? "Installing" : "Downloading");

function progressText(task, short = false) {
  if (task.status === "queued") return short ? "…" : "Preparing…";
  if (task.status === "installing") return short ? "…" : task.stage || "Installing…";
  if (task.total) {
    const pct = Math.floor((task.received / task.total) * 100);
    return short ? ` ${pct}%` : `${pct}% · ${formatBytes(task.received)} of ${formatBytes(task.total)}`;
  }
  return short ? "…" : `${formatBytes(task.received) || "0 B"} downloaded`;
}

/* ---------- Rendering ---------- */

function render(force = false) {
  if (!ui.data) return;
  const key = JSON.stringify([ui.gameId, ui.versionFor, ui.data.installed, ui.data.running.map((r) => [r.game_id, r.version]),
    ui.data.tasks.map((t) => [t.id, t.status]), ui.data.games.length, ui.data.problems.length]);
  if (!force && key === ui.renderKey) return updateProgress();
  ui.renderKey = key;
  renderLibrary();
  renderDetail();
}

function renderLibrary() {
  const list = document.getElementById("game-list");
  document.getElementById("lib-count").textContent = games().length;
  list.replaceChildren(...games().map((game) => {
    const status = gameStatus(game);
    return el("li", {},
      el("button", {
        type: "button", class: `game-card${game.id === ui.gameId ? " selected" : ""}`,
        "aria-current": game.id === ui.gameId ? "true" : null,
        onclick: () => selectGame(game.id),
      },
      gameIcon(game),
      el("span", { class: "game-card-text" },
        el("span", { class: "game-card-title", text: game.title }),
        el("span", { class: `game-card-status ${status.cls}`, text: status.text, "data-status-for": game.id }))));
  }));

  const problems = document.getElementById("problems");
  const count = ui.data.problems.length;
  problems.hidden = count === 0;
  problems.replaceChildren(icon("warning"), `${count} catalog problem${count === 1 ? "" : "s"}. Show`);
}

function gameIcon(game) {
  if (game.icon) return el("img", { class: "game-icon", src: catalogUrl(game.icon), alt: "" });
  return el("span", { class: "game-icon placeholder", text: game.title.slice(0, 1).toUpperCase() });
}

function renderDetail() {
  const detail = document.getElementById("detail");
  const game = gameById(ui.gameId);
  if (!game) {
    detail.replaceChildren(el("div", { class: "empty" }, el("div", {},
      el("img", { src: "img/toki-emblem.svg", alt: "" }),
      el("h2", { text: "No games yet" }),
      el("p", { text: "Toki Hub lists every game in its catalog folder. Add your first one and it will show up here." }),
      button("Add a game", { kind: "primary", onclick: () => openDialog("help-dialog") }))));
    return;
  }
  const version = selectedVersion(game);
  if (game.changelog) loadChangelog(game.id);
  const scroll = detail.scrollTop;
  detail.replaceChildren(
    el("section", { class: "hero" },
      game.cover ? el("img", { class: "hero-art", src: catalogUrl(game.cover), alt: "" })
        : el("div", { class: "hero-fallback" }, el("img", { src: "img/halo.svg", alt: "" }))),
    el("section", { class: "panel headline", style: game.accent ? `--accent: ${game.accent}` : null },
      gameIcon(game),
      el("div", { class: "headline-text" },
        el("h1", { class: "game-title", text: game.title }),
        game.tagline ? el("p", { class: "tagline", text: game.tagline }) : null,
        game.tags.length ? el("div", { class: "tags" }, game.tags.map((t) => el("span", { class: "tag" }, el("span", { text: t })))) : null)),
    el("section", { class: "panel action-bar" }, actionBar(game, version)),
    el("div", { class: "columns" },
      el("div", { class: "stack" },
        version ? notesPanel(game, version) : null,
        el("section", { class: "panel" },
          el("h2", { class: "section-title", text: "About" }),
          el("p", { class: "about-text", text: game.description || "No description yet." }),
          game.developer ? el("div", { class: "developer", text: `By ${game.developer}` }) : null)),
      versionsPanel(game, version)));
  detail.scrollTop = scroll;
}

function actionBar(game, version) {
  if (!version) {
    return [el("div", { class: "action-version" }, el("span", { class: "label", text: "Versions" }),
      el("span", { class: "value", text: "None yet" })),
    el("div", { class: "build-meta" }, "Add a version to this game's game.json in the catalog, then press reload.")];
  }
  const v = version.version;
  const installed = installedInfo(game.id, v);
  const task = activeTask(game.id, v);
  const running = isRunning(game.id, v);
  const main = [];
  const side = [];

  if (task) {
    main.push(el("div", { class: "progress", "data-task-id": task.id },
      el("div", { class: "progress-top" }, el("span", { class: "progress-label", text: `${taskVerb(task)} ${vlabel(v)}` }),
        el("span", { class: "progress-text", text: progressText(task) })),
      el("div", { class: "progress-track" }, el("div", { class: "progress-fill" }))));
    main.push(button("Cancel", { iconName: "x", onclick: () => cancelTask(task) }));
  } else if (running) {
    main.push(el("div", { class: "status-pill" }, el("span", {}, el("span", { class: "pulse" }), "Running")));
    side.push(button("Stop", { kind: "danger-ghost", iconName: "stop", onclick: () => stopGame(game, v),
      title: "Close the game" }));
  } else if (installed) {
    main.push(button("Launch", { kind: "primary", iconName: "play", onclick: () => launchGame(game, v) }));
  } else if (version.build) {
    main.push(button("Download", { kind: "primary", iconName: "download", onclick: () => installGame(game, v) }));
  } else {
    main.push(button(`Not available for ${platformName(ui.data.platform)}`, { kind: "primary", disabled: true }));
  }

  if (installed && !task) {
    side.push(button("Folder", { iconName: "folder", onclick: () => attempt(() => api("open_folder", game.id, v), "Could not open the folder") }));
    side.push(button("Log", { iconName: "log", onclick: () => attempt(() => api("open_log", game.id, v), "No log to show") }));
    if (!running) {
      if (version.build) side.push(button("Reinstall", { iconName: "download", onclick: () => installGame(game, v) }));
      side.push(button("Uninstall", { kind: "danger-ghost", iconName: "trash", onclick: () => uninstallGame(game, v) }));
    }
  }

  const meta = [];
  if (version.build) {
    meta.push(el("span", {}, icon("source"), version.build.source));
    if (version.build.note) meta.push(el("span", { class: "note" }, icon("info"), version.build.note));
  } else if (version.platforms.length) {
    meta.push(el("span", {}, icon("info"), `Builds exist for ${version.platforms.map(platformName).join(", ")} only.`));
  }
  if (installed) meta.push(el("span", {}, `Installed ${formatDate(installed.installed_at)}${installed.size ? ` · ${formatBytes(installed.size)}` : ""}`));

  const bar = [
    el("div", { class: "action-version" }, el("span", { class: "label", text: "Version" }), el("span", { class: "value", text: vlabel(v) })),
    el("div", { class: "action-main" }, main),
    side.length ? el("div", { class: "action-side" }, side) : null,
    meta.length ? el("div", { class: "build-meta" }, meta) : null,
  ];
  requestAnimationFrame(updateProgress);
  return bar;
}

function notesPanel(game, version) {
  const notes = versionNotes(game, version);
  return el("section", { class: "panel notes" },
    el("div", { class: "notes-head" },
      el("h2", { class: "section-title", text: `What's in ${vlabel(version.version)}` }),
      version.date ? el("span", { class: "notes-date", text: formatDate(version.date) }) : null),
    notes ? formatNotes(notes) : el("p", { class: "muted", text: "No release notes for this version." }));
}

// The version's section of the game's changelog once it has arrived, otherwise the catalog's notes.
function versionNotes(game, version) {
  const changelog = ui.changelogs[game.id];
  return (typeof changelog === "object" && changelog.versions[version.version]) || version.notes;
}

// Fetch a game's changelog once (the hub keeps a copy for offline use), then redraw its page.
async function loadChangelog(gameId) {
  if (ui.changelogs[gameId]) return;
  ui.changelogs[gameId] = "loading";
  let changelog;
  try {
    changelog = await api("changelog", gameId);
  } catch (error) {
    changelog = { versions: {}, from: "none", error: error.message };
  }
  ui.changelogs[gameId] = changelog;
  if (ui.gameId === gameId) renderDetail();
}

function formatNotes(text) {
  const fragment = document.createDocumentFragment();
  let list = null;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) { list = null; continue; }
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const bullet = line.match(/^[-*•]\s+(.*)$/);
    if (heading) {
      list = null;
      fragment.append(el("h3", { class: "notes-heading", text: plainText(heading[1]) }));
    } else if (bullet) {
      if (!list) { list = el("ul"); fragment.append(list); }
      list.append(el("li", { text: plainText(bullet[1]) }));
    } else {
      list = null;
      fragment.append(el("p", { text: plainText(line) }));
    }
  }
  return fragment;
}

// Markdown's inline marks, dropped: **bold**, `code`, and [links](address) keep just their text.
function plainText(text) {
  return text.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1").replace(/\*\*|__|`/g, "");
}

function versionsPanel(game, selected) {
  const latest = game.versions[0];
  return el("section", { class: "panel" },
    el("h2", { class: "section-title" }, "Versions", el("span", { class: "count", text: game.versions.length })),
    game.versions.length === 0 ? el("p", { class: "muted", text: "No versions in the catalog yet." }) :
      el("ul", { class: "version-list" }, game.versions.map((v) => {
        const badges = [];
        if (v === latest) badges.push(["Latest", "latest"]);
        if (isRunning(game.id, v.version)) badges.push(["Running", "running"]);
        else if (activeTask(game.id, v.version)) badges.push(["Downloading", "busy"]);
        else if (installedInfo(game.id, v.version)) badges.push(["Installed", "installed"]);
        if (!v.build) badges.push([`No ${platformName(ui.data.platform)} build`, "unavailable"]);
        return el("li", {}, el("button", {
          type: "button", class: `version-row${v === selected ? " selected" : ""}`,
          "aria-pressed": v === selected ? "true" : "false",
          onclick: () => { ui.versionFor[game.id] = v.version; render(); },
        },
        el("span", { class: "version-name", text: vlabel(v.version) }),
        v.date ? el("span", { class: "version-date", text: formatDate(v.date) }) : null,
        el("span", { class: "badges" }, badges.map(([text, kind]) => el("span", { class: `badge badge-${kind}` }, el("span", { text }))))));
      })));
}

function updateProgress() {
  for (const node of document.querySelectorAll("[data-task-id]")) {
    const task = ui.data.tasks.find((t) => t.id === node.dataset.taskId);
    if (!task) continue;
    node.querySelector(".progress-label").textContent = `${taskVerb(task)} ${vlabel(task.version)}`;
    node.querySelector(".progress-text").textContent = progressText(task);
    const fill = node.querySelector(".progress-fill");
    const known = task.status === "downloading" && task.total;
    fill.classList.toggle("indeterminate", !known && task.status !== "installing");
    fill.style.width = task.status === "installing" ? "100%" : known ? `${(task.received / task.total) * 100}%` : "";
  }
  for (const game of games()) {
    const node = document.querySelector(`[data-status-for="${CSS.escape(game.id)}"]`);
    if (node) node.textContent = gameStatus(game).text;
  }
}

/* ---------- Actions ---------- */

function selectGame(id) {
  if (ui.gameId === id) return;
  ui.gameId = id;
  store("game", id);
  render();
  const game = gameById(id);
  if (game) Toki.say("select", { t: game.title });
}

async function installGame(game, version) {
  const task = await attempt(() => api("install", game.id, version), `Could not download ${game.title} ${vlabel(version)}`);
  if (!task) return;
  ui.taskStatus[task.id] = task.status;
  Toki.say("installStart", { t: game.title, v: vlabel(version) });
  refreshSoon(0);
}

async function cancelTask(task) {
  await attempt(() => api("cancel", task.id), "Could not cancel");
  refreshSoon(0);
}

async function launchGame(game, version) {
  const result = await attempt(() => api("launch", game.id, version), `Could not launch ${game.title} ${vlabel(version)}`);
  if (!result) return;
  Toki.say("launch", { t: game.title, v: vlabel(version) });
  refreshSoon(0);
}

async function stopGame(game, version) {
  await attempt(() => api("stop", game.id, version), "Could not stop the game");
  refreshSoon(300);
}

function uninstallGame(game, version) {
  const dialog = document.getElementById("confirm-dialog");
  document.getElementById("confirm-title").textContent = `Uninstall ${game.title} ${vlabel(version)}?`;
  document.getElementById("confirm-text").textContent =
    "Its files are deleted from this computer. You can download it again at any time. Save data the game keeps elsewhere is not touched.";
  document.getElementById("confirm-ok").onclick = async () => {
    dialog.close();
    const ok = await attempt(() => api("uninstall", game.id, version).then(() => true), "Could not uninstall");
    if (ok) Toki.say("uninstall", { t: game.title, v: vlabel(version) });
    refreshSoon(0);
  };
  dialog.showModal();
}

async function reloadCatalog() {
  const btn = document.getElementById("btn-reload");
  btn.classList.remove("spinning");
  void btn.offsetWidth;
  btn.classList.add("spinning");
  const data = await attempt(() => api("reload"), "Could not reload the catalog");
  if (!data) return;
  ui.changelogs = {};
  applyState(data);
  if (data.problems.length) Toki.say("problems", { n: data.problems.length });
  else Toki.say("reload", { n: data.games.length });
}

/* ---------- Polling ---------- */

function refreshSoon(delay) {
  clearTimeout(ui.pollTimer);
  ui.pollTimer = setTimeout(poll, delay);
}

async function poll() {
  let delay = 1500;
  try {
    const update = await api("poll", ui.lastEvent);
    setConnected(true);
    Object.assign(ui.data, { installed: update.installed, tasks: update.tasks, running: update.running, update: update.update });
    noticeTaskChanges();
    for (const event of update.events) handleEvent(event);
    render();
    renderUpdate();
    await finishUpdate();
    if (update.tasks.some((t) => ["queued", "downloading", "installing"].includes(t.status))) delay = 400;
    else if (update.update?.status === "downloading") delay = 400;
    else if (document.hidden) delay = 5000;
  } catch {
    setConnected(false);
    delay = 3000;
  }
  refreshSoon(delay);
}

function noticeTaskChanges() {
  for (const task of ui.data.tasks) {
    const before = ui.taskStatus[task.id];
    ui.taskStatus[task.id] = task.status;
    if (!before || before === task.status) continue;
    const vars = { t: task.title, v: vlabel(task.version) };
    if (task.status === "done") {
      toast({ kind: "success", title: `${task.title} ${vlabel(task.version)} installed`, text: "Ready to launch." });
      Toki.say("installDone", vars);
    } else if (task.status === "error") {
      toast({ kind: "error", title: `Could not install ${task.title} ${vlabel(task.version)}`, text: task.error, sticky: true });
      Toki.say("installError", vars);
    } else if (task.status === "cancelled") {
      Toki.say("installCancelled", vars);
    }
  }
}

function handleEvent(event) {
  ui.lastEvent = Math.max(ui.lastEvent, event.id);
  if (event.type !== "exited") return;
  const vars = { t: event.title, v: vlabel(event.version) };
  if (event.stopped) {
    Toki.say("stopped", vars);
    return;
  }
  if (event.code === 0 || event.code === null) {
    Toki.say("exitOk", vars);
    return;
  }
  toast({
    kind: "error", sticky: true,
    title: `${event.title} ${vlabel(event.version)} closed unexpectedly`,
    text: `Exit code ${event.code} after ${event.seconds}s.`,
    detail: event.log_tail,
    action: ["Open full log", () => attempt(() => api("open_log", event.game_id, event.version), "No log to show")],
  });
  Toki.say("exitCrash", vars);
}

function setConnected(connected) {
  if (connected === ui.connected) return;
  ui.connected = connected;
  if (!connected) toast({ kind: "error", title: "Toki Hub is not responding", text: "Retrying…", id: "offline", sticky: true });
  else document.querySelector('[data-toast-id="offline"]')?.remove();
}

/* ---------- Updating Toki Hub itself ---------- */

function updateStatusText(update) {
  switch (update.status) {
    case "unsupported": return "Updates are not available for this computer.";
    case "checking": return "Checking for updates…";
    case "current": return `You have the newest version (${update.current}).`;
    case "available": return `Version ${update.version} is available.`;
    case "downloading": return `Downloading version ${update.version}…`;
    case "ready": return `Version ${update.version} is ready to install.`;
    case "error": return `Could not check for updates. ${update.error}`;
    default: return `Toki Hub ${update.current || ""}`;
  }
}

function renderUpdate() {
  const update = ui.data.update || {};
  const button = document.getElementById("btn-update");
  const shown = ["available", "downloading", "ready"].includes(update.status);
  button.hidden = !shown;
  if (shown && !button.querySelector("svg")) button.prepend(icon("download"));
  button.querySelector(".update-label").textContent =
    update.status === "available" ? `Update to ${update.version}` : "Updating…";
  document.getElementById("update-status").textContent = updateStatusText(update);
  const working = update.status === "downloading" || update.status === "ready" || ui.updating;
  document.getElementById("update-progress").hidden = !working;
  document.getElementById("update-now").disabled = working;
  if (working) {
    const fill = document.getElementById("update-progress-fill");
    const known = update.total > 0;
    fill.classList.toggle("indeterminate", !known);
    fill.style.width = known ? `${Math.min(100, (update.received / update.total) * 100)}%` : "";
    document.getElementById("update-progress-text").textContent = update.status === "ready" ? "Installing…"
      : known ? `${Math.floor((update.received / update.total) * 100)}% · ${formatBytes(update.received)} of ${formatBytes(update.total)}`
      : `${formatBytes(update.received) || "0 B"} downloaded`;
  }
}

async function checkForUpdates(manual = false) {
  if (manual) document.getElementById("update-status").textContent = "Checking for updates…";
  const update = manual
    ? await attempt(() => api("check_for_updates"), "Could not check for updates")
    : await api("check_for_updates").catch(() => null);
  if (!update) return;
  const before = ui.data.update?.status;
  ui.data.update = update;
  renderUpdate();
  if (update.status === "available" && (manual || before !== "available")) Toki.say("updateAvailable", { v: update.version });
  else if (manual && update.status === "current") Toki.say("updateCurrent", { v: update.current });
}

function openUpdate() {
  const update = ui.data.update || {};
  document.getElementById("update-title").textContent = `Toki Hub ${update.version}`;
  document.getElementById("update-subtitle").textContent =
    `You have version ${update.current}. Updating takes a few seconds; Toki Hub restarts by itself.`;
  const notes = (update.notes || "").replace(/\*\*/g, "").trim();
  document.getElementById("update-notes").replaceChildren(notes ? formatNotes(notes) : el("p", { class: "muted", text: "No release notes." }));
  renderUpdate();
  openDialog("update-dialog");
}

async function startUpdate() {
  ui.updating = true;
  renderUpdate();
  const update = await attempt(() => api("download_update"), "Could not download the update");
  if (!update) {
    ui.updating = false;
    renderUpdate();
    return;
  }
  ui.data.update = update;
  Toki.say("updateDownloading", { v: update.version });
  refreshSoon(0);
}

/** Once an update the player asked for has downloaded, put it in place and restart. */
async function finishUpdate() {
  const update = ui.data.update || {};
  if (!ui.updating) return;
  if (update.status === "error") {
    ui.updating = false;
    renderUpdate();
    toast({ kind: "error", title: "The update did not download", text: update.error, sticky: true });
    Toki.say("error");
  } else if (update.status === "ready") {
    ui.updating = false;
    Toki.say("updateInstalling");
    const done = await attempt(() => api("apply_update"), "Could not install the update");
    if (done === undefined) renderUpdate();
  }
}

/* ---------- Toasts ---------- */

function toast({ kind = "info", title, text, detail, action, sticky = false, id }) {
  const box = document.getElementById("toasts");
  if (id) box.querySelector(`[data-toast-id="${id}"]`)?.remove();
  const node = el("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status", "data-toast-id": id },
    el("div", { class: "toast-title", text: title }),
    text ? el("div", { class: "toast-text", text }) : null,
    detail ? el("pre", { text: detail }) : null,
    action ? button(action[0], { small: true, iconName: "log", onclick: action[1] }) : null,
    el("button", { type: "button", class: "toast-close", "aria-label": "Dismiss", text: "×", onclick: () => node.remove() }));
  box.prepend(node);
  while (box.children.length > 4) box.lastElementChild.remove();
  if (!sticky) setTimeout(() => node.remove(), kind === "error" ? 10000 : 5000);
}

/* ---------- Dialogs ---------- */

function openDialog(id) {
  document.getElementById(id).showModal();
}

function openSettings() {
  const s = ui.data.settings;
  const installDir = document.getElementById("set-install-dir");
  installDir.value = s.install_dir;
  installDir.placeholder = s.install_dir_effective;
  const python = document.getElementById("set-python");
  python.value = s.python;
  python.placeholder = s.python_effective;
  const token = document.getElementById("set-token");
  token.value = "";
  token.placeholder = s.has_token ? "Saved. Type a new token to replace it."
    : s.token_from_env ? "Using GITHUB_TOKEN from the environment." : "Not set";
  document.getElementById("set-clear-token").hidden = !s.has_token;
  document.getElementById("set-browse").hidden = !ui.data.native_window;
  document.getElementById("place-catalog").textContent = ui.data.places.catalog;
  document.getElementById("place-catalog").title = ui.data.places.catalog;
  document.getElementById("place-data").textContent = ui.data.places.data;
  document.getElementById("place-data").title = ui.data.places.data;
  document.getElementById("hub-version").textContent = `Toki Hub ${ui.data.hub_version}`;
  renderUpdate();
  openDialog("settings-dialog");
}

async function saveSettings(event) {
  event.preventDefault();
  const settings = await attempt(() => api("save_settings", {
    install_dir: document.getElementById("set-install-dir").value,
    python: document.getElementById("set-python").value,
    github_token: document.getElementById("set-token").value,
  }), "Settings were not saved");
  if (!settings) return;
  ui.data.settings = settings;
  document.getElementById("settings-dialog").close();
  Toki.say("settingsSaved");
  refreshSoon(0);
}

/* ---------- Start-up ---------- */

function applyState(data) {
  ui.data = data;
  for (const task of data.tasks) ui.taskStatus[task.id] = task.status;
  for (const event of data.events) ui.lastEvent = Math.max(ui.lastEvent, event.id);
  if (!gameById(ui.gameId)) {
    const remembered = recall("game");
    ui.gameId = gameById(remembered) ? remembered : data.games[0]?.id ?? null;
  }
  render(true);
}

function wireUp() {
  const reload = document.getElementById("btn-reload");
  reload.append(icon("refresh"));
  reload.addEventListener("click", reloadCatalog);
  const settings = document.getElementById("btn-settings");
  settings.append(icon("settings"));
  settings.addEventListener("click", openSettings);
  document.getElementById("btn-add-game").addEventListener("click", () => openDialog("help-dialog"));
  document.getElementById("problems").addEventListener("click", () => {
    document.getElementById("problem-list").replaceChildren(...ui.data.problems.map((p) => el("li", { text: p })));
    openDialog("problems-dialog");
  });
  document.getElementById("settings-form").addEventListener("submit", saveSettings);
  document.getElementById("set-cancel").addEventListener("click", () => document.getElementById("settings-dialog").close());
  document.getElementById("set-browse").addEventListener("click", async () => {
    const folder = await attempt(() => api("pick_folder"), "Could not open the folder picker");
    if (folder) document.getElementById("set-install-dir").value = folder;
  });
  document.getElementById("set-clear-token").addEventListener("click", async () => {
    const settings = await attempt(() => api("clear_token"), "Could not remove the token");
    if (!settings) return;
    ui.data.settings = settings;
    openSettings();
  });
  for (const node of document.querySelectorAll("[data-close]")) {
    node.addEventListener("click", () => node.closest("dialog").close());
  }
  for (const node of document.querySelectorAll("[data-open-place]")) {
    node.addEventListener("click", () => attempt(() => api("open_place", node.dataset.openPlace), "Could not open the folder"));
  }
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refreshSoon(0); });
  document.getElementById("btn-update").addEventListener("click", openUpdate);
  document.getElementById("update-now").addEventListener("click", startUpdate);
  document.getElementById("btn-check-updates").addEventListener("click", () => checkForUpdates(true));
}

async function start() {
  Toki.init();
  wireUp();
  try {
    applyState(await api("state"));
  } catch (error) {
    toast({ kind: "error", title: "Toki Hub is not responding", text: error.message, sticky: true });
    return;
  }
  if (ui.data.just_updated) Toki.say("updated", { v: ui.data.hub_version });
  else if (ui.data.problems.length) Toki.say("problems", { n: ui.data.problems.length });
  else if (!ui.data.games.length) Toki.say("empty");
  else Toki.say("greet");
  renderUpdate();
  refreshSoon(1500);
  // Look for a new version of the hub shortly after start-up, then every six hours.
  setTimeout(() => checkForUpdates(false), 4000);
  setInterval(() => checkForUpdates(false), 6 * 60 * 60 * 1000);
}

start();
