/* Toki's lines for the dialogue bar. {t} = game title, {v} = version, {n} = a number.
   Add or change lines freely: one is picked at random each time. */
"use strict";

const TOKI_LINES = {
  greet: [
    "Welcome back, Sensei. C&C call sign 04, Asuma Toki. I will be looking after your games today.",
    "Sensei. Your library has been cleaned and sorted. As expected of the best agent.",
    "Good to see you, Sensei. All systems are ready. I am also ready.",
  ],
  idle: [
    "I am smiling right now. On the inside.",
    "Pyon. ...That was a bunny joke. You may laugh, Sensei.",
    "Every version of every game, kept tidy. Cleaning is part of the job.",
    "If you need me, I am right here. I am always right here.",
    "Victory pose. ✌ ...Was that convincing?",
    "Add more games to the catalog and I will look after those too. That is a promise.",
    "C&C stands for Cleaning & Clearing. Today, I am clearing your backlog.",
  ],
  select: [
    "{t}. An excellent choice, Sensei.",
    "{t}, standing by.",
    "{t}. I have prepared it. Mentally.",
  ],
  installStart: [
    "Retrieving {t} {v}. I am fast, but the internet is not. Please wait.",
    "Downloading {t} {v}. Leave it to me.",
  ],
  installDone: [
    "{t} {v} is installed and ready. Pyon. ✌",
    "Download complete. {t} {v} is ready whenever you are, Sensei.",
    "Ready to play! {t} {v} is all set, Sensei. Pyon, pyon. ✌",
    "{t} {v}: installed. Ready to play! ...I may have done a small hop just now.",
  ],
  apkDownloaded: [
    "{t} {v} is downloaded. Tap Install in Android's prompt and it is ready to play. Pyon. ✌",
  ],
  installError: [
    "The download of {t} {v} did not go to plan. I am... mildly frustrated. The details are in the notice.",
  ],
  installCancelled: ["Download cancelled. I have tidied up the leftovers."],
  launch: [
    "Launching {t} {v}. Have fun, Sensei. I will keep watch here.",
    "{t} {v}, deploying now.",
  ],
  exitOk: [
    "Welcome back from {t}, Sensei. Did you win? I will assume you won.",
    "{t} has closed. Mission complete.",
  ],
  exitCrash: ["{t} {v} closed unexpectedly. I saved its log for you."],
  stopped: ["{t} has been stopped, as you asked. Mission aborted. Cleanly."],
  uninstall: ["{t} {v} has been tidied away. Cleaning is part of the job, after all."],
  settingsSaved: ["Settings saved. I have memorised them perfectly."],
  goodbye: ["Understood, Sensei. I will tidy up after myself. ...It was an honour. ✌"],
  reload: ["Catalog reloaded. {n} game(s) on file."],
  problems: ["Sensei, the catalog has {n} problem(s). I have marked them for you."],
  empty: ["The library is empty. Add a game to the catalog and I will take care of the rest."],
  error: ["That did not work, Sensei. I have put the details in a notice."],
  updateAvailable: ["Sensei, a newer version of me is available: {v}. I recommend it. I may be biased.",
                    "Toki Hub {v} is out. The update button is glowing at the top. Gold suits it."],
  updateCurrent: ["I am already the newest version, {v}. Peak performance."],
  updateDownloading: ["Downloading version {v} of myself. This is a strange feeling."],
  updateInstalling: ["Installing the update. I will be right back, Sensei. Please don't go anywhere."],
  updated: ["Update complete. I am now version {v}. I feel... newer. Pyon."],
  gameUpdateAvailable: ["Sensei, {t} {v} is out. The gold button is waiting for you. I polished it.",
                        "{t} has an update: {v}. I recommend it. The button is gold, so it must be good."],
  gameUpdatesWaiting: ["Sensei, {n} of your games have updates waiting. Look for the gold badges."],
  gameUpdateStart: ["Updating {t} to {v}. Out with the old, in with the new. Cleaning is part of the job."],
  gameUpdated: ["{t} is now {v}. I have tidied away {o}. Pyon. ✌",
                "Ready to play! {t} {v} is in, and {o} is tidied away."],
};

const Toki = (() => {
  let lineEl, boxEl, portraitEl, barEl, toggleEl;
  let timer = null;
  let fullText = "";
  let lastPick = {};

  function pick(kind) {
    const options = TOKI_LINES[kind] || TOKI_LINES.idle;
    let i = Math.floor(Math.random() * options.length);
    if (options.length > 1 && i === lastPick[kind]) i = (i + 1) % options.length;
    lastPick[kind] = i;
    return options[i];
  }

  function fill(text, vars) {
    return text.replace(/\{(\w+)\}/g, (m, key) => (vars && key in vars ? vars[key] : m));
  }

  function finish() {
    clearInterval(timer);
    timer = null;
    lineEl.textContent = fullText;
    boxEl.classList.add("done");
  }

  // cheer: a happy bounce instead of the usual bob, for good news like a finished install.
  function say(kind, vars, cheer = false) {
    fullText = fill(pick(kind), vars);
    clearInterval(timer);
    boxEl.classList.remove("done");
    portraitEl.classList.remove("talking", "cheer");
    void portraitEl.offsetWidth;          // restart the animation
    portraitEl.classList.add(cheer ? "cheer" : "talking");
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return finish();
    let shown = 0;
    lineEl.textContent = "";
    timer = setInterval(() => {
      shown += 1;
      lineEl.textContent = fullText.slice(0, shown);
      if (shown >= fullText.length) finish();
    }, 22);
  }

  // Collapsed, the bar is one slim line (Toki keeps talking in it), leaving more room for the games.
  // Remembered between runs.
  const collapsed = () => barEl.classList.contains("collapsed");
  function setCollapsed(on, remember = true) {
    barEl.classList.toggle("collapsed", on);
    const label = on ? "Expand Toki's text box" : "Collapse Toki's text box";
    toggleEl.setAttribute("aria-expanded", on ? "false" : "true");
    toggleEl.setAttribute("aria-label", label);
    toggleEl.title = label;
    portraitEl.title = on ? "Expand Toki's text box" : "Talk to Toki";
    portraitEl.setAttribute("aria-label", portraitEl.title);
    if (remember) {
      try { localStorage.setItem("tokihub.tokiCollapsed", on ? "1" : ""); } catch { /* storage unavailable: fine */ }
    }
  }

  function init() {
    lineEl = document.getElementById("toki-line");
    boxEl = document.getElementById("dialogue-box");
    portraitEl = document.getElementById("toki-portrait");
    barEl = document.getElementById("dialogue");
    toggleEl = document.getElementById("toki-toggle");
    // A tap on the slim bar opens it again; on the full one, it finishes Toki's line or gets a new one
    const talk = () => (collapsed() ? setCollapsed(false) : timer ? finish() : say("idle"));
    boxEl.addEventListener("click", talk);
    portraitEl.addEventListener("click", talk);
    toggleEl.addEventListener("click", (event) => {
      event.stopPropagation();
      setCollapsed(!collapsed());
    });
    let saved = null;
    try { saved = localStorage.getItem("tokihub.tokiCollapsed"); } catch { /* storage unavailable: fine */ }
    setCollapsed(saved === "1", false);

    // Use ui/img/toki.png if it exists, otherwise keep the emblem.
    const art = document.getElementById("toki-art");
    const emblem = document.getElementById("toki-emblem");
    art.addEventListener("load", () => { art.hidden = false; emblem.hidden = true; });
    art.addEventListener("error", () => { art.remove(); });
    if (art.complete && art.naturalWidth) { art.hidden = false; emblem.hidden = true; }
  }

  const cheer = (kind, vars) => say(kind, vars, true);

  return { init, say, cheer };
})();
