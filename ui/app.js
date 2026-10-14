// Slotboard – setup & help window
(() => {
  "use strict";

  const tauri = window.__TAURI__;
  const invoke = tauri ? tauri.core.invoke : async () => null;
  const listen = tauri ? tauri.event.listen : async () => () => {};

  const TEXT = {
    de: {
      welcomeTitle: "Zehn Fächer für deine Zwischenablage.",
      welcomeLead: "Kopieren und Einfügen wie immer – nur mit einer Ziffer dazu.",
      welcomeNote: "Ohne Ziffer funktioniert alles wie gewohnt.",
      store: "ablegen",
      paste: "einfügen",
      setUp: "Einrichten",
      accessTitle: "Slotboard braucht Zugriff auf die Tastatur.",
      accessLead: "macOS fragt dafür nach einer Freigabe unter „Bedienungshilfen“.",
      accessPrivacy: "Slotboard reagiert nur auf {mod}C und {mod}V mit einer Ziffer. Es speichert keine Eingaben und verbindet sich nie mit dem Internet.",
      accessStep1: "Im Dialog auf „Systemeinstellungen öffnen“ klicken.",
      accessStep2: "Slotboard in der Liste einschalten.",
      accessStep3: "Hierher zurückkommen – es geht von allein weiter.",
      allowAccess: "Zugriff erlauben",
      waiting: "Warte auf Freigabe …",
      granted: "Zugriff erteilt",
      inputMonitoring: "Bitte zusätzlich „Eingabeüberwachung“ erlauben.",
      troubleTitle: "Es tut sich nichts?",
      troubleText: "Steht Slotboard schon in der Liste, ist aber eingeschaltet? Dann den Eintrag mit „–“ entfernen und erneut auf „Zugriff erlauben“ klicken. Hilft das nicht, Slotboard einmal neu starten.",
      openSettings: "Einstellungen öffnen",
      restart: "Neu starten",
      waylandTitle: "Unter Wayland nicht verfügbar.",
      waylandLead: "Wayland erlaubt keine globalen Tastenkürzel. Melde dich mit einer X11-Sitzung an (z. B. „Ubuntu on Xorg“), dann funktioniert Slotboard.",
      close: "Schließen",
      tryTitle: "Probier es aus.",
      tryStore: "Text markieren, <kbd>{mod}</kbd><kbd>C</kbd> drücken, {mod} halten und <kbd>1</kbd> tippen.",
      tryPaste: "Ins leere Feld klicken, <kbd>{mod}</kbd><kbd>V</kbd> drücken, {mod} halten und <kbd>1</kbd> tippen.",
      sample: "Hallo aus Fach 1",
      continue: "Weiter",
      doneTitle: "Fertig. Slotboard läuft jetzt in der Menüleiste.",
      doneTitleTray: "Fertig. Slotboard läuft jetzt im Infobereich der Taskleiste.",
      helpTitle: "Slotboard",
      active: "Aktiv",
      paused: "Pausiert – über das Menü wieder einschalten",
      inactive: "Noch nicht aktiv",
      menuHint: "Ein Klick auf das Symbol zeigt deine Fächer. Ein Klick auf ein Fach legt es in die normale Zwischenablage.",
      autostart: "Beim Anmelden starten",
      done: "Fertig",
      clearSlots: "Fächer leeren",
      replay: "Einrichtung ansehen",
      version: "Version {v}",
      ctrl: "Strg",
    },
    en: {
      welcomeTitle: "Ten slots for your clipboard.",
      welcomeLead: "Copy and paste as always – just add a digit.",
      welcomeNote: "Without a digit, everything works as usual.",
      store: "store",
      paste: "paste",
      setUp: "Set up",
      accessTitle: "Slotboard needs access to the keyboard.",
      accessLead: "macOS asks for permission under “Accessibility”.",
      accessPrivacy: "Slotboard only reacts to {mod}C and {mod}V with a digit. It stores no keystrokes and never connects to the internet.",
      accessStep1: "Click “Open System Settings” in the dialog.",
      accessStep2: "Turn Slotboard on in the list.",
      accessStep3: "Come back here – it continues by itself.",
      allowAccess: "Allow access",
      waiting: "Waiting for permission …",
      granted: "Access granted",
      inputMonitoring: "Please also allow “Input Monitoring”.",
      troubleTitle: "Nothing happens?",
      troubleText: "Is Slotboard already in the list and switched on? Remove the entry with “–” and click “Allow access” again. If that doesn’t help, restart Slotboard once.",
      openSettings: "Open Settings",
      restart: "Restart",
      waylandTitle: "Not available on Wayland.",
      waylandLead: "Wayland does not allow global shortcuts. Log in with an X11 session (e.g. “Ubuntu on Xorg”) and Slotboard works.",
      close: "Close",
      tryTitle: "Try it.",
      tryStore: "Select the text, press <kbd>{mod}</kbd><kbd>C</kbd>, keep holding {mod} and tap <kbd>1</kbd>.",
      tryPaste: "Click the empty field, press <kbd>{mod}</kbd><kbd>V</kbd>, keep holding {mod} and tap <kbd>1</kbd>.",
      sample: "Hello from slot 1",
      continue: "Continue",
      doneTitle: "Done. Slotboard now lives in the menu bar.",
      doneTitleTray: "Done. Slotboard now lives in the system tray.",
      helpTitle: "Slotboard",
      active: "Active",
      paused: "Paused – switch it back on from the menu",
      inactive: "Not active yet",
      menuHint: "Click the icon to see your slots. Click a slot to put it on the normal clipboard.",
      autostart: "Open at login",
      done: "Done",
      clearSlots: "Clear slots",
      replay: "View setup",
      version: "Version {v}",
      ctrl: "Ctrl",
    },
  };

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

  let state = null;
  let t = TEXT.en;
  let flow = [];
  let current = null;
  let advanceTimer = null;
  let pasteDone = false;
  let previousFilled = [];
  let helpMode = false;

  const modLabel = () => (state && state.platform === "macos" ? "⌘" : t.ctrl);
  const fill = (s) => s.replaceAll("{mod}", modLabel());
  const escapeHtml = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

  function applyTexts() {
    document.documentElement.lang = state.lang;
    $$("[data-t]").forEach((el) => { el.textContent = fill(t[el.dataset.t] || ""); });
    $$("[data-t-html]").forEach((el) => {
      // Only our own strings; {mod} is a plain symbol.
      el.innerHTML = fill(t[el.dataset.tHtml] || "").replace(/<(?!\/?kbd>)/g, "&lt;");
    });
    $$("[data-mod]").forEach((el) => { el.textContent = modLabel(); });
    $("[data-version]").textContent = fill(t.version.replace("{v}", escapeHtml(state.version)));
    const sample = $("[data-sample]");
    if (sample && !sample.value) sample.value = t.sample;
    sample.setAttribute("aria-label", t.sample);
    $("[data-target]").setAttribute("aria-label", t.paste);
  }

  function buildFlow() {
    const unsupported = state.status.permission === "unsupported";
    if (unsupported) return ["welcome", "unsupported"];
    const steps = ["welcome"];
    if (state.platform === "macos") steps.push("access");
    steps.push("try", "done");
    return steps;
  }

  function show(page) {
    current = page;
    $$(".page").forEach((p) => p.classList.toggle("active", p.dataset.page === page));
    const idx = flow.indexOf(page);
    $$(".progress").forEach((el) => {
      el.textContent = idx >= 0 && flow.length > 1 && !helpMode ? `${idx + 1} / ${flow.length}` : "";
    });
    render();
  }

  function next() {
    const idx = flow.indexOf(current);
    let target = flow[Math.min(idx + 1, flow.length - 1)];
    // Skip the access step when access is already there.
    if (target === "access" && state.status.active) target = flow[flow.indexOf("access") + 1];
    show(target);
  }


  function renderBoards() {
    const filled = state.slots.map((s) => s.kind !== "empty");
    $$("[data-board]").forEach((board) => {
      if (board.children.length !== 10) {
        board.innerHTML = state.slots.map((s) => `<span class="key">${escapeHtml(s.key)}</span>`).join("");
      }
      [...board.children].forEach((key, i) => {
        key.classList.toggle("filled", filled[i]);
        if (filled[i] && previousFilled.length && !previousFilled[i]) {
          key.classList.add("just");
          setTimeout(() => key.classList.remove("just"), 140);
        }
        const s = state.slots[i];
        key.title = s.kind === "empty" ? "" : (s.text || s.kind);
      });
    });
    previousFilled = filled;
  }

  function renderStatus() {
    const st = state.status;
    $$("[data-status]").forEach((el) => {
      el.classList.remove("waiting", "ok", "warn", "paused");
      let text;
      if (current === "access") {
        if (st.active) { el.classList.add("ok"); text = t.granted; }
        else if (st.permission === "input_monitoring") { el.classList.add("warn"); text = t.inputMonitoring; }
        else { el.classList.add("waiting"); text = t.waiting; }
      } else if (st.paused) { el.classList.add("paused"); text = t.paused; }
      else if (st.active) { el.classList.add("ok"); text = t.active; }
      else { el.classList.add("warn"); text = t.inactive; }
      $("[data-status-text]", el).textContent = text;
    });
  }

  function renderTry() {
    const slot = state.slots[0];
    const stored = slot && slot.kind === "text" && slot.text.includes(t.sample);
    $('[data-task="store"]').classList.toggle("done", Boolean(stored));
    $('[data-task="paste"]').classList.toggle("done", pasteDone);
  }

  function renderDone() {
    const title = $("[data-done-title]");
    if (helpMode) title.textContent = t.helpTitle;
    else title.textContent = state.platform === "macos" ? t.doneTitle : t.doneTitleTray;
    $("[data-autostart]").checked = Boolean(state.autostart);
    $('[data-action="finish"]').textContent = t.done;
    $('[data-action="clear"]').hidden = !helpMode || state.slots.every((s) => s.kind === "empty");
    $('[data-action="replay"]').hidden = !helpMode;
    $('.page[data-page="done"] [data-status]').hidden = !helpMode;
  }

  function render() {
    if (!state) return;
    renderBoards();
    renderStatus();
    renderTry();
    renderDone();
    if (current === "access" && state.status.active && !advanceTimer) {
      advanceTimer = setTimeout(() => { advanceTimer = null; if (current === "access") next(); }, 900);
    }
  }

  async function refresh(newState) {
    state = newState || (await invoke("get_state"));
    if (!state) return;
    t = TEXT[state.lang] || TEXT.en;
    applyTexts();
    render();
  }

  // ---- actions ----

  document.addEventListener("click", async (ev) => {
    const btn = ev.target.closest("[data-action]");
    if (!btn) return;
    switch (btn.dataset.action) {
      case "next": next(); break;
      case "request": await invoke("request_permission"); break;
      case "open-settings": await invoke("open_permission_settings"); break;
      case "restart": await invoke("restart_app"); break;
      case "close": await invoke("close_window"); break;
      case "website": await invoke("open_website"); break;
      case "clear": await invoke("clear_slots"); break;
      case "replay":
        helpMode = false;
        flow = buildFlow();
        show("welcome");
        break;
      case "finish":
        if (state.onboarding_done && helpMode) await invoke("close_window");
        else await invoke("finish_setup");
        break;
    }
  });

  $("[data-autostart]").addEventListener("change", async (ev) => {
    try {
      const enabled = await invoke("set_autostart", { enabled: ev.target.checked });
      ev.target.checked = Boolean(enabled);
    } catch (_) {
      ev.target.checked = !ev.target.checked;
    }
  });

  // Enter = main button of the current page (except while typing in a field).
  document.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter" || ev.target.closest("input, button, summary")) return;
    const primary = $(`.page[data-page="${current}"] .primary`);
    if (primary) primary.click();
  });

  const sample = $("[data-sample]");
  sample.addEventListener("focus", () => setTimeout(() => sample.select(), 0));
  sample.addEventListener("mouseup", (e) => e.preventDefault());

  const target = $("[data-target]");
  target.addEventListener("input", () => {
    if (!pasteDone && target.value.includes(t.sample)) {
      pasteDone = true;
      renderTry();
    }
  });

  // ---- start ----

  (async () => {
    await refresh();
    if (!state) {
      // Opened in a plain browser (design preview).
      state = {
        platform: "macos", lang: (navigator.language || "de").startsWith("de") ? "de" : "en", version: "0.1.0",
        status: { permission: "missing", active: false, paused: false, notice: null },
        slots: Array.from({ length: 10 }, (_, i) => ({ index: i, key: String((i + 1) % 10), kind: i === 0 || i === 3 ? "text" : "empty", text: "" })),
        autostart: true, onboarding_done: false,
      };
      t = TEXT[state.lang];
      applyTexts();
    }
    flow = buildFlow();
    const preview = new URLSearchParams(location.search).get("page");
    if (preview) {
      helpMode = preview === "help";
      if (helpMode) { state.onboarding_done = true; state.status.active = true; }
      show(helpMode ? "done" : preview);
    } else if (state.onboarding_done && state.status.active) {
      helpMode = true;
      show("done");
    } else {
      show("welcome");
    }
    await listen("state", (event) => refresh(event.payload));
  })();
})();
