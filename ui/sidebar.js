// Burrow's sidebar. Rust pushes the whole view with burrow.render(view); we send commands back by
// logging `__burrow__{json}` (D-009). Text from web pages (titles, URLs) is always set with
// textContent, never innerHTML.
"use strict";

const $ = (id) => document.getElementById(id);
const send = (msg) => console.log("__burrow__" + JSON.stringify(msg));
const cmd = (name, args = {}) => send({ cmd: name, ...args });
const act = (name, args = {}) => send({ act: name, ...args });

let view = null; // { state, live, shown, downloads, toast, arc_available, agent_active, … }
let busy = 0; // >0 while renaming or dragging: renders wait so they don't clobber the UI
let pendingRender = false;
let addressMode = null; // null | "edit" | "new"
let renameNext = null; // { parent } — rename the folder that appears next
let lastShown = null;

// ---------------------------------------------------------------------------------------------
// Helpers

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}
function icon(name, cls = "ico") {
  const e = el("span", cls);
  e.innerHTML = ICONS[name]; // static markup from icons.js
  return e;
}
/** Shows icon `name` in an existing element, reparsing the SVG only when it changes. */
function setIcon(e, name) {
  if (e.dataset.icon === name) return;
  e.dataset.icon = name;
  e.innerHTML = ICONS[name]; // static markup from icons.js
}
const space = () => view.state.spaces.find((s) => s.id === view.state.active_space) || view.state.spaces[0];
function hostOf(url) {
  try {
    const u = new URL(url);
    return u.hostname.replace(/^www\./, "") || url;
  } catch {
    return url;
  }
}
const titleOf = (t) => t.custom_title || t.title || hostOf(t.url) || "New Tab";
const isLive = (id) => !!(view.live && view.live[id]);
const isLoading = (id) => !!(view.live && view.live[id] && view.live[id].loading);
const drifted = (t) => t.home_url && t.home_url !== t.url;
const inSplit = (id) => !!(view.visible && view.visible.length > 1 && view.visible.includes(id));
function findTab(id) {
  for (const s of view.state.spaces) {
    const walk = (nodes) => {
      for (const n of nodes) {
        if (n.kind === "tab" && n.id === id) return n;
        if (n.kind === "folder") {
          const f = walk(n.children);
          if (f) return f;
        }
      }
    };
    const t = s.favorites.find((x) => x.id === id) || walk(s.pinned) || s.today.find((x) => x.id === id);
    if (t) return t;
  }
  return null;
}

const GLOBE_IMG =
  "data:image/svg+xml," +
  encodeURIComponent(ICONS.globe.replace('stroke="currentColor"', 'stroke="#6b6880"').replace('width="16" height="16"', 'width="24" height="24"'));

/** The icon Burrow saved on disk for a URL's site (D-025), if any. */
function savedIcon(url) {
  if (!url || !/^https?:/.test(url) || !view.icons) return null;
  const file = view.icons.files[hostOf(url).toLowerCase()];
  return file ? view.icons.base + file : null;
}

// Icon URLs that failed to load. Every render rebuilds the rows, so without this each one
// would download the same missing icons again.
const brokenIcons = new Set();

function favicon(tab, size = 16) {
  if (isLoading(tab.id)) return el("span", "spinner");
  // Best first: the saved icon of the site the tab is on, of its home site, the icon the page
  // reported, the site's /favicon.ico; a globe if none loads.
  const sources = [savedIcon(tab.url), savedIcon(tab.home_url)];
  if (tab.favicon && /^(https?:|data:image\/)/.test(tab.favicon)) sources.push(tab.favicon);
  if (/^https?:/.test(tab.url)) {
    try { sources.push(new URL(tab.url).origin + "/favicon.ico"); } catch { /* not a URL */ }
  }
  const list = [...new Set(sources.filter(Boolean))].filter((u) => !brokenIcons.has(u));
  if (!list.length) return icon("globe", "ico");
  const img = el("img", "favicon");
  img.width = img.height = size;
  let i = 0;
  // Swap the source rather than the element: errors can fire before the img is attached.
  img.onerror = () => {
    brokenIcons.add(list[i]);
    i += 1;
    if (i < list.length) img.src = list[i];
    else {
      img.onerror = null;
      img.src = GLOBE_IMG;
    }
  };
  img.src = list[0];
  return img;
}

function copy(text) {
  const ta = el("textarea");
  ta.value = text;
  document.body.append(ta);
  ta.select();
  document.execCommand("copy");
  ta.remove();
  toast("Link copied");
}

// ---------------------------------------------------------------------------------------------
// Rendering

// Where every node sits, for drag and drop: id -> { section, parent, index, folder }.
let positions = {};

function render() {
  pendingRender = false;
  if (!view) return;
  const sp = space();
  document.documentElement.style.setProperty("--space", sp.color || "#c9c2f0");
  document.body.classList.toggle("collapsed", !!view.sidebar_collapsed);
  peeking = !!view.peeking;
  positions = {};

  renderToolbar();
  renderAddress();
  renderFind();
  renderPrompts();
  renderFavorites(sp);
  $("space-icon").textContent = sp.icon || "";
  $("space-name").textContent = sp.name;
  renderTree($("pinned"), sp.pinned, "pinned", null, 0);
  renderToday(sp);
  renderSpaces();
  renderDownloadsButton();

  if (renameNext) {
    // Rename the folder that appeared since "New Folder" was chosen (not just the last one).
    const { parent, known, until } = renameNext;
    const list = parent ? findFolder(sp.pinned, parent)?.children : sp.pinned;
    const folder = list && list.find((n) => n.kind === "folder" && !known.has(n.id));
    if (folder || Date.now() > until) renameNext = null;
    if (folder) startRename(document.querySelector(`.row[data-id="${folder.id}"]`), folder);
  }
  if (view.shown !== lastShown) {
    lastShown = view.shown;
    document.querySelector(".row.active")?.scrollIntoView({ block: "nearest" });
  }
}

// A page load sends several views in a row (address, title, icon, loading): draw only the last
// one, once per frame.
let renderQueued = false;
function scheduleRender() {
  if (busy > 0) pendingRender = true;
  else if (!renderQueued) {
    renderQueued = true;
    requestAnimationFrame(() => {
      renderQueued = false;
      if (busy > 0) pendingRender = true;
      else render();
    });
  }
}

function findFolder(nodes, id) {
  for (const n of nodes) {
    if (n.kind !== "folder") continue;
    if (n.id === id) return n;
    const f = findFolder(n.children, id);
    if (f) return f;
  }
  return null;
}

function renderToolbar() {
  setIcon($("btn-toggle"), "sidebar");
  $("agent-badge").hidden = !view.agent_active;
  const nav = (view.shown && view.live[view.shown]) || {};
  const back = $("btn-back"), fwd = $("btn-forward"), reload = $("btn-reload");
  setIcon(back, "back");
  setIcon(fwd, "forward");
  back.disabled = !nav.can_back;
  fwd.disabled = !nav.can_forward;
  setIcon(reload, nav.loading ? "stop" : "reload");
  reload.title = nav.loading ? "Stop" : "Reload (Ctrl+R)";
  reload.disabled = !view.shown;
  setIcon($("btn-extensions"), "puzzle");
  $("btn-extensions").classList.toggle("on", !!view.extensions.popup);
}

function renderAddress() {
  const input = $("address-input");
  const tab = view.shown ? findTab(view.shown) : null;
  const zoom = $("zoom");
  zoom.hidden = !tab || !view.zoom || view.zoom === 100;
  zoom.textContent = `${view.zoom}%`;
  setIcon($("address-lock"), "lock");
  $("address-lock").classList.toggle("on", !!tab && tab.url.startsWith("https:") && addressMode === null);
  if (addressMode !== null) return; // the user is typing
  input.value = tab ? hostOf(tab.url) : "";
  input.placeholder = "Search or enter address";
}

function renderFavorites(sp) {
  const box = $("favorites");
  box.replaceChildren();
  sp.favorites.forEach((t, i) => {
    positions[t.id] = { section: "favorites", parent: null, index: i, folder: false };
    const b = el("button", "fav");
    b.dataset.id = t.id;
    b.title = titleOf(t);
    b.classList.toggle("active", t.id === view.shown || inSplit(t.id));
    b.classList.toggle("live", isLive(t.id));
    b.append(favicon(t, 20));
    b.onclick = () => cmd("select_tab", { tab_id: t.id });
    b.onauxclick = (e) => e.button === 1 && cmd("close_tab", { tab_id: t.id });
    b.oncontextmenu = (e) => tabMenu(e, t, "favorites");
    makeDraggable(b, t.id, t.url);
    makeDropTarget(b, t.id);
    box.append(b);
  });
}

function tabRow(t, section, depth) {
  const row = el("div", "row tab");
  row.dataset.id = t.id;
  row.style.setProperty("--depth", depth);
  row.classList.toggle("active", t.id === view.shown);
  row.classList.toggle("unloaded", section !== "today" && !isLive(t.id));
  row.classList.toggle("in-split", inSplit(t.id));
  const ico = el("span", "ico");
  ico.append(favicon(t));
  row.append(ico, el("span", "label", titleOf(t)));
  if (view.live[t.id]?.muted) {
    const m = icon("muted", "flag");
    m.title = "Muted — click to unmute";
    m.onclick = (e) => (e.stopPropagation(), act("set_muted", { tab_id: t.id, muted: false }));
    row.append(m);
  }
  if (view.pip === t.id) {
    const p = icon("pip", "flag");
    p.title = "Playing in picture in picture — click to put the video back";
    p.onclick = (e) => (e.stopPropagation(), act("toggle_pip", { tab_id: t.id }));
    row.append(p);
  }
  if (inSplit(t.id)) {
    const sp = icon("split", "flag");
    sp.title = "In Split View";
    row.append(sp);
  }
  row.title = `${titleOf(t)}\n${t.url}`;

  const actions = el("span", "actions");
  if (drifted(t)) {
    const home = icon("home", "act");
    home.title = "Back to pinned page";
    home.onclick = (e) => (e.stopPropagation(), cmd("reset_tab", { tab_id: t.id }));
    actions.append(home);
  }
  if (section === "today" || isLive(t.id)) {
    const x = icon(section === "today" ? "close" : "minus", "act");
    x.title = section === "today" ? "Close tab" : "Unload tab (stays pinned)";
    x.onclick = (e) => (e.stopPropagation(), cmd("close_tab", { tab_id: t.id }));
    actions.append(x);
  }
  row.append(actions);

  row.onclick = () => cmd("select_tab", { tab_id: t.id });
  row.onauxclick = (e) => e.button === 1 && cmd("close_tab", { tab_id: t.id });
  row.ondblclick = () => startRename(row, t);
  row.oncontextmenu = (e) => tabMenu(e, t, section);
  makeDraggable(row, t.id, t.url);
  makeDropTarget(row, t.id);
  return row;
}

function renderTree(box, nodes, section, parent, depth) {
  box.replaceChildren();
  nodes.forEach((n, i) => {
    positions[n.id] = { section, parent, index: i, folder: n.kind === "folder" };
    if (n.kind === "tab") {
      box.append(tabRow(n, section, depth));
      return;
    }
    const wrap = el("div", "node" + (n.collapsed ? " collapsed" : ""));
    wrap.dataset.id = n.id;
    const row = el("div", "row folder");
    row.dataset.id = n.id;
    row.style.setProperty("--depth", depth);
    row.append(icon(n.collapsed ? "folder" : "folderOpen"), el("span", "label", n.name));
    row.onclick = () => cmd("toggle_folder", { folder_id: n.id });
    row.ondblclick = (e) => (e.stopPropagation(), startRename(row, n));
    row.oncontextmenu = (e) => folderMenu(e, n);
    makeDraggable(row, n.id, null);
    makeDropTarget(row, n.id);
    const kids = el("div", "children");
    renderTree(kids, n.children, section, n.id, depth + 1);
    if (!n.children.length) {
      const empty = el("div", "empty-folder", "Empty — drag tabs here");
      empty.style.setProperty("--depth", depth + 1);
      makeAreaDropTarget(empty, () => ({ section: "pinned", parent: n.id, index: null }));
      kids.append(empty);
    }
    wrap.append(row, kids);
    box.append(wrap);
  });
}

function renderToday(sp) {
  const box = $("today");
  box.replaceChildren();
  sp.today.forEach((t, i) => {
    positions[t.id] = { section: "today", parent: null, index: i, folder: false };
    box.append(tabRow(t, "today", 0));
  });
  $("btn-clear").hidden = sp.today.length === 0;
}

function renderSpaces() {
  const box = $("spaces");
  box.replaceChildren();
  view.state.spaces.forEach((s, i) => {
    const b = el("button", "space-dot" + (s.icon ? " emoji" : ""));
    b.classList.toggle("active", s.id === view.state.active_space);
    b.title = `${s.name}  (Ctrl+${i + 1})`;
    if (s.icon) b.textContent = s.icon;
    else b.append(el("span", "dot"));
    b.onclick = () => cmd("switch_space", { space_id: s.id });
    b.oncontextmenu = (e) => spaceMenu(e, s);
    makeSpaceDropTarget(b, s);
    box.append(b);
  });
}

function renderDownloadsButton() {
  const b = $("btn-downloads");
  setIcon(b, "download");
  const d = view.downloads || [];
  b.classList.toggle("has", d.length > 0);
  b.classList.toggle("busy", d.some((x) => !x.done && !x.canceled));
}

// ---------------------------------------------------------------------------------------------
// Address box

const addressInput = $("address-input");
let suggestions = []; // [{ tab, space } | { visit }] matching what's typed
let historyHits = { q: "", list: [] }; // the last answer to act("suggest")
let suggestIndex = -1; // highlighted suggestion, -1 = none (Enter navigates)

function allTabs() {
  const out = [];
  for (const s of view.state.spaces) {
    const walk = (nodes) => nodes.forEach((n) => (n.kind === "tab" ? out.push([n, s]) : walk(n.children)));
    s.favorites.forEach((t) => out.push([t, s]));
    walk(s.pinned);
    s.today.forEach((t) => out.push([t, s]));
  }
  return out;
}

function updateSuggestions() {
  const q = addressInput.value.trim().toLowerCase();
  if (addressMode !== null && addressMode !== "find" && q.length >= 2 && q !== historyHits.q) act("suggest", { query: q });
  renderSuggestions();
}

/** Open tabs first (any space), then pages from history that aren't open. */
function renderSuggestions() {
  const q = addressInput.value.trim().toLowerCase();
  const box = $("suggest");
  suggestions = [];
  suggestIndex = -1;
  if (addressMode === null || addressMode === "find" || q.length < 2) {
    box.hidden = true;
    return;
  }
  const words = q.split(/\s+/);
  const active = view.state.active_space;
  const tabs = allTabs()
    .filter(([t]) => words.every((w) => titleOf(t).toLowerCase().includes(w) || t.url.toLowerCase().includes(w)))
    .sort(([, a], [, b]) => (a.id === active ? 0 : 1) - (b.id === active ? 0 : 1))
    .slice(0, 5);
  const open = new Set(allTabs().map(([t]) => t.url));
  const visits = historyHits.q === q ? historyHits.list.filter((v) => !open.has(v.url)).slice(0, 8 - tabs.length) : [];
  suggestions = [...tabs.map(([tab, space]) => ({ tab, space })), ...visits.map((visit) => ({ visit }))];
  box.replaceChildren();
  let headed = false;
  suggestions.forEach((sg, i) => {
    if (sg.visit && !headed) {
      headed = true;
      box.append(el("div", "hint", "History"));
    }
    const row = el("div", "row");
    const ico = el("span", "ico");
    if (sg.tab) {
      ico.append(favicon(sg.tab));
      row.append(ico, el("span", "label", titleOf(sg.tab)));
      if (sg.space.id !== active) row.append(el("span", "space-tag", (sg.space.icon ? sg.space.icon + " " : "") + sg.space.name));
    } else {
      ico.append(favicon({ id: "", url: sg.visit.url }));
      row.append(ico, el("span", "label", sg.visit.title || sg.visit.url), el("span", "space-tag", hostOf(sg.visit.url)));
    }
    row.title = sg.tab ? sg.tab.url : sg.visit.url;
    row.onmousedown = (e) => {
      e.preventDefault();
      pickSuggestion(i);
    };
    box.append(row);
  });
  if (suggestions.length) box.append(el("div", "hint", "↑↓ to pick · Enter to search"));
  box.hidden = suggestions.length === 0;
}

function highlightSuggestion(i) {
  suggestIndex = Math.max(-1, Math.min(suggestions.length - 1, i));
  $("suggest").querySelectorAll(".row").forEach((r, k) => r.classList.toggle("sel", k === suggestIndex));
}

function pickSuggestion(i) {
  const sg = suggestions[i];
  const newTab = addressMode === "new" || !view.shown;
  leaveAddress();
  if (sg.tab) cmd("select_tab", { tab_id: sg.tab.id });
  else act("navigate", { input: sg.visit.url, new_tab: newTab });
}

function leaveAddress() {
  if (addressMode === "find") act("find_stop");
  addressMode = null;
  $("suggest").hidden = true;
  $("find-count").hidden = true;
  $("address").classList.remove("find");
  addressInput.blur();
}

addressInput.addEventListener("focus", () => {
  if (addressMode === null) {
    addressMode = "edit";
    const tab = view && view.shown ? findTab(view.shown) : null;
    addressInput.value = tab ? tab.url : "";
  }
  $("address-lock").classList.remove("on");
  requestAnimationFrame(() => addressInput.select());
});
addressInput.addEventListener("blur", () => {
  if (addressMode === "find") act("find_stop");
  addressMode = null;
  $("suggest").hidden = true;
  $("find-count").hidden = true;
  $("address").classList.remove("find");
  if (view) renderAddress();
});
addressInput.addEventListener("input", () => {
  if (addressMode === "find") act("find", { text: addressInput.value, forward: true, next: false });
  else updateSuggestions();
});
addressInput.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    leaveAddress();
    act("focus_content");
  } else if (e.key === "ArrowDown" && suggestions.length) {
    e.preventDefault();
    highlightSuggestion(suggestIndex + 1);
  } else if (e.key === "ArrowUp" && suggestions.length) {
    e.preventDefault();
    highlightSuggestion(suggestIndex - 1);
  }
});
$("address").addEventListener("submit", (e) => {
  e.preventDefault();
  if (addressMode === "find") {
    act("find", { text: addressInput.value, forward: !e.submitter && !shiftDown, next: true });
    return;
  }
  if (suggestIndex >= 0) return pickSuggestion(suggestIndex);
  const input = addressInput.value;
  const newTab = addressMode === "new" || !view.shown;
  leaveAddress();
  act("navigate", { input, new_tab: newTab });
});
let shiftDown = false;
addEventListener("keydown", (e) => (shiftDown = e.shiftKey), true);
addEventListener("keyup", (e) => (shiftDown = e.shiftKey), true);

function renderFind() {
  const c = $("find-count");
  if (addressMode !== "find") return;
  const f = view.find || { count: 0, current: 0 };
  c.hidden = !addressInput.value;
  c.textContent = f.count ? `${Math.max(1, f.current)}/${f.count}` : "0/0";
}

window.burrow = {
  render(v) {
    view = v;
    if (v.toast) toast(v.toast);
    scheduleRender();
    if (dialogRefresh && !backdrop.hidden) dialogRefresh();
  },
  /** History matches for the address box (answer to act("suggest")). */
  suggestions(q, list) {
    historyHits = { q, list };
    if (addressInput.value.trim().toLowerCase() === q) renderSuggestions();
  },
  /** The History dialog's list (answer to act("history_list")). */
  history(q, list) {
    if (historyView) historyView(q, list);
  },
  openHistory() {
    document.body.classList.remove("collapsed");
    historyDialog();
  },
  focusInput(newTab) {
    document.body.classList.remove("collapsed"); // hidden elements can't take focus
    addressMode = newTab ? "new" : "edit";
    $("address").classList.remove("find");
    const tab = view && view.shown ? findTab(view.shown) : null;
    addressInput.value = newTab || !tab ? "" : tab.url;
    addressInput.placeholder = newTab ? "New tab — search, address, or a tab's name" : "Search or enter address";
    addressInput.focus();
    addressInput.select();
    updateSuggestions();
  },
  findMode() {
    document.body.classList.remove("collapsed");
    addressMode = "find";
    $("address").classList.add("find");
    $("suggest").hidden = true;
    addressInput.value = "";
    addressInput.placeholder = "Find in page";
    addressInput.focus();
  },
};

// ---------------------------------------------------------------------------------------------
// Permission prompts

const PERMISSION_TEXT = {
  camera: "use your camera", microphone: "use your microphone", screen: "share your screen",
  location: "know your location", notifications: "show notifications", clipboard: "read your clipboard",
  midi: "use MIDI devices", downloads: "download multiple files", storage: "use storage across sites",
  fonts: "use your fonts", windows: "manage windows", idle: "know when you're idle", files: "edit files",
  keyboard: "lock the keyboard", pointer: "lock the mouse pointer", "protected-media": "play protected media",
  "protocol-handler": "open links of a type", "local-network": "access your local network", sensors: "use motion sensors",
  other: "use a browser feature",
};

function renderPrompts() {
  const box = $("prompts");
  box.replaceChildren();
  renderUpdatePrompt(box);
  renderExtensionPrompts(box);
  for (const p of view.permissions || []) {
    const card = el("div", "prompt");
    const what = el("div", "what");
    let host = p.origin;
    try { host = new URL(p.origin).host; } catch { /* keep origin */ }
    const wants = p.names.map((n) => PERMISSION_TEXT[n] || n);
    const list = wants.length > 1 ? wants.slice(0, -1).join(", ") + " and " + wants[wants.length - 1] : wants[0];
    what.append(el("b", null, host), document.createTextNode(` wants to ${list}.`));
    card.append(what);
    if (p.tab_title) card.append(el("div", "from", p.tab_title));
    const buttons = el("div", "buttons");
    const rememberL = el("label");
    const remember = el("input");
    remember.type = "checkbox";
    remember.checked = true;
    rememberL.append(remember, document.createTextNode("Remember"));
    const block = el("button", "btn", "Block");
    const allow = el("button", "btn allow", "Allow");
    block.onclick = () => act("permission", { id: p.id, allow: false, remember: remember.checked });
    allow.onclick = () => act("permission", { id: p.id, allow: true, remember: remember.checked });
    buttons.append(rememberL, block, allow);
    card.append(buttons);
    box.append(card);
  }
}

// ---------------------------------------------------------------------------------------------
// Inline rename

function startRename(row, node) {
  if (!row) return;
  const label = row.querySelector(".label");
  if (!label) return;
  const isFolder = node.kind === "folder";
  const input = el("input", "rename");
  input.value = isFolder ? node.name : titleOf(node);
  label.replaceWith(input);
  busy++;
  let done = false;
  const finish = (save) => {
    if (done) return;
    done = true;
    busy--;
    const value = input.value.trim();
    if (save && isFolder && value && value !== node.name) cmd("rename_folder", { folder_id: node.id, name: value });
    else if (save && !isFolder && value !== titleOf(node)) cmd("rename_tab", { tab_id: node.id, title: value || null });
    input.replaceWith(label);
    if (pendingRender || busy === 0) render();
  };
  input.onkeydown = (e) => {
    if (e.key === "Enter") finish(true);
    if (e.key === "Escape") finish(false);
    e.stopPropagation();
  };
  input.onblur = () => finish(true);
  input.onclick = (e) => e.stopPropagation();
  input.focus();
  input.select();
}

// ---------------------------------------------------------------------------------------------
// Drag and drop

let dragged = null; // { id, folder, section }

function makeDraggable(node, id, url) {
  node.draggable = true;
  node.addEventListener("dragstart", (e) => {
    dragged = { id, ...positions[id] };
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", url || id);
    if (url) e.dataTransfer.setData("text/uri-list", url);
    busy++;
    requestAnimationFrame(() => (node.classList.add("dragging"), document.body.classList.add("drag-active")));
    e.stopPropagation();
  });
  node.addEventListener("dragend", () => {
    node.classList.remove("dragging");
    document.body.classList.remove("drag-active");
    clearDropMarks();
    dragged = null;
    busy--;
    if (pendingRender) render();
  });
}

function clearDropMarks() {
  document.querySelectorAll(".drop-before,.drop-after,.drop-into").forEach((n) => n.classList.remove("drop-before", "drop-after", "drop-into"));
}

/** True if `node` is inside the folder being dragged (a folder can't go into itself). */
function insideDragged(node) {
  const wrap = dragged && dragged.folder && document.querySelector(`.node[data-id="${dragged.id}"]`);
  return !!wrap && wrap.contains(node);
}

/** Where a drop on `targetId` at this pointer position would go, or null if not allowed. */
function dropPlacement(e, node, targetId) {
  if (!dragged || dragged.id === targetId || insideDragged(node)) return null;
  const p = positions[targetId];
  const r = node.getBoundingClientRect();
  if (p.section === "favorites") {
    if (dragged.folder) return null;
    const after = e.clientX > r.left + r.width / 2;
    return { mark: after ? "drop-after" : "drop-before", to: { section: "favorites", parent: null, index: p.index + (after ? 1 : 0) } };
  }
  const y = (e.clientY - r.top) / r.height;
  if (p.folder) {
    if (y > 0.25 && y < 0.75) return { mark: "drop-into", to: { section: "pinned", parent: targetId, index: null } };
  }
  if (dragged.folder && p.section !== "pinned") return null;
  const after = y >= 0.5;
  return { mark: after ? "drop-after" : "drop-before", to: { section: p.section, parent: p.parent, index: p.index + (after ? 1 : 0) } };
}

function moveDragged(to) {
  const src = positions[dragged.id];
  // The node is removed before it's inserted, so moving down a list shifts the index by one.
  if (src && to.index !== null && src.section === to.section && (src.parent || null) === (to.parent || null) && !to.space_id && src.index < to.index) {
    to.index -= 1;
  }
  cmd("move_node", { node_id: dragged.id, to: { space_id: null, parent: null, ...to } });
}

function makeDropTarget(node, targetId) {
  // Rows always stop propagation: a drop a row refuses (e.g. onto itself) must not fall
  // through to the area behind it, which would move the item to the end of the list.
  node.addEventListener("dragover", (e) => {
    if (!dragged) return;
    e.stopPropagation();
    clearDropMarks();
    const place = dropPlacement(e, node, targetId);
    if (!place) return;
    e.preventDefault();
    node.classList.add(place.mark);
  });
  node.addEventListener("dragleave", () => node.classList.remove("drop-before", "drop-after", "drop-into"));
  node.addEventListener("drop", (e) => {
    if (!dragged) return;
    e.stopPropagation();
    const place = dropPlacement(e, node, targetId);
    if (!place) return;
    e.preventDefault();
    moveDragged(place.to);
  });
}

/** Drops on empty areas: the end of favorites, pinned or today; or the top of today. */
function makeAreaDropTarget(node, toFn) {
  const target = () => dragged && !insideDragged(node) && toFn();
  node.addEventListener("dragover", (e) => {
    if (!dragged) return;
    e.stopPropagation();
    clearDropMarks();
    if (!target()) return;
    e.preventDefault();
    node.classList.add("drop-into");
  });
  node.addEventListener("dragleave", (e) => {
    if (!node.contains(e.relatedTarget)) node.classList.remove("drop-into");
  });
  node.addEventListener("drop", (e) => {
    if (!dragged) return;
    e.stopPropagation();
    const to = target();
    if (!to) return;
    e.preventDefault();
    moveDragged(to);
  });
}
makeAreaDropTarget($("favorites"), () => !dragged.folder && { section: "favorites", index: null });
makeAreaDropTarget($("pinned"), () => ({ section: "pinned", index: null }));
makeAreaDropTarget($("space-header"), () => ({ section: "pinned", index: 0 }));
makeAreaDropTarget($("divider"), () => !dragged.folder && { section: "today", index: 0 });
makeAreaDropTarget($("new-tab"), () => !dragged.folder && { section: "today", index: 0 });
makeAreaDropTarget($("today"), () => !dragged.folder && { section: "today", index: null });

function makeSpaceDropTarget(node, s) {
  node.addEventListener("dragover", (e) => {
    if (!dragged || s.id === view.state.active_space) return;
    e.preventDefault();
    clearDropMarks();
    node.classList.add("drop-into");
  });
  node.addEventListener("dragleave", () => node.classList.remove("drop-into"));
  node.addEventListener("drop", (e) => {
    if (!dragged || s.id === view.state.active_space) return;
    e.preventDefault();
    const section = dragged.folder ? "pinned" : dragged.section;
    cmd("move_node", { node_id: dragged.id, to: { space_id: s.id, section, parent: null, index: section === "today" ? 0 : null } });
  });
}

// ---------------------------------------------------------------------------------------------
// Menus

const menu = $("menu");

function showMenu(e, items) {
  e.preventDefault();
  e.stopPropagation();
  fillMenu(items);
  menu.hidden = false;
  const x = Math.min(e.clientX, window.innerWidth - menu.offsetWidth - 6);
  const y = Math.min(e.clientY, window.innerHeight - menu.offsetHeight - 6);
  menu.style.left = Math.max(6, x) + "px";
  menu.style.top = Math.max(6, y) + "px";
}

function fillMenu(items, parentItems) {
  menu.replaceChildren();
  if (parentItems) {
    const back = el("button", "item", "‹ Back");
    back.onclick = (e) => (e.stopPropagation(), fillMenu(parentItems));
    menu.append(back, el("div", "sep"));
  }
  for (const it of items) {
    if (!it) continue;
    if (it.sep) { menu.append(el("div", "sep")); continue; }
    if (it.heading) { menu.append(el("div", "heading", it.heading)); continue; }
    if (it.swatches) {
      const row = el("div", "swatches");
      for (const c of it.swatches) {
        const sw = el("button", "swatch" + (c === it.current ? " on" : ""));
        sw.style.background = c;
        sw.onclick = () => (hideMenu(), it.pick(c));
        row.append(sw);
      }
      menu.append(row);
      continue;
    }
    const b = el("button", "item" + (it.danger ? " danger" : ""), it.label);
    if (it.img !== undefined) b.prepend(extImage(it.img, "mimg"));
    if (it.sub) {
      b.append(el("span", "chev", "›"));
      b.onclick = (e) => (e.stopPropagation(), fillMenu(it.sub, items));
    } else {
      b.onclick = () => (hideMenu(), it.action());
    }
    menu.append(b);
  }
}

function hideMenu() {
  menu.hidden = true;
}
document.addEventListener("click", (e) => !menu.contains(e.target) && hideMenu());
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  hideMenu();
  closeDialog();
  if (view && view.extensions.popup) act("close_extension_popup");
});
// Clicking anywhere in the sidebar closes an extension popup, as clicking outside one does in Chrome.
document.addEventListener("mousedown", (e) => {
  if (view && view.extensions.popup && !e.target.closest("#btn-extensions, #menu")) act("close_extension_popup");
});
window.addEventListener("blur", hideMenu);

function moveToSpaceItems(id, section) {
  const others = view.state.spaces.filter((s) => s.id !== view.state.active_space);
  if (!others.length) return null;
  return {
    label: "Move to Space",
    sub: others.map((s) => ({
      label: `${s.icon ? s.icon + "  " : ""}${s.name}`,
      action: () => cmd("move_node", { node_id: id, to: { space_id: s.id, section, parent: null, index: section === "today" ? 0 : null } }),
    })),
  };
}

function tabMenu(e, t, section) {
  const move = (to) => cmd("move_node", { node_id: t.id, to: { space_id: null, parent: null, index: null, ...to } });
  const live = view.live[t.id];
  const web = /^https?:/.test(t.url);
  const canSplit = view.shown && view.shown !== t.id && !inSplit(t.id) && space().id === view.state.active_space;
  showMenu(e, [
    section === "today" && { label: "Pin Tab", action: () => move({ section: "pinned" }) },
    section === "pinned" && { label: "Unpin Tab", action: () => move({ section: "today", index: 0 }) },
    section !== "favorites" && { label: "Add to Favorites", action: () => move({ section: "favorites" }) },
    section === "favorites" && { label: "Remove from Favorites", action: () => move({ section: "today", index: 0 }) },
    drifted(t) && { label: "Back to Pinned Page", action: () => cmd("reset_tab", { tab_id: t.id }) },
    moveToSpaceItems(t.id, section),
    { sep: true },
    canSplit && { label: "Open in Split View", action: () => cmd("split_view", { tab_id: t.id }) },
    inSplit(t.id) && { label: "Close Split View", action: () => cmd("unsplit", { space_id: null }) },
    { label: "Duplicate Tab", action: () => cmd("duplicate_tab", { tab_id: t.id }) },
    live && { label: live.muted ? "Unmute Tab" : "Mute Tab", action: () => act("set_muted", { tab_id: t.id, muted: !live.muted }) },
    live && { label: view.pip === t.id ? "Exit Picture in Picture" : "Picture in Picture", action: () => act("toggle_pip", { tab_id: t.id }) },
    web && { label: "Send This Site to a Space…", action: () => atcDialog(hostOf(t.url)) },
    web && { label: "Boost This Site…", action: () => boostDialog(hostOf(t.url)) },
    { sep: true },
    section !== "favorites" && { label: "Rename…", action: () => startRename(document.querySelector(`.row[data-id="${t.id}"]`), t) },
    t.custom_title && { label: "Reset Name", action: () => cmd("rename_tab", { tab_id: t.id, title: null }) },
    { label: "Copy Link", action: () => copy(t.url) },
    { sep: true },
    section === "today" && { label: "Close Tab", danger: true, action: () => cmd("close_tab", { tab_id: t.id }) },
    section !== "today" && isLive(t.id) && { label: "Unload Tab", action: () => cmd("close_tab", { tab_id: t.id }) },
    section !== "today" && {
      label: "Delete Tab",
      danger: true,
      action: () => {
        move({ section: "today", index: 0 });
        cmd("close_tab", { tab_id: t.id });
      },
    },
  ]);
}

function folderMenu(e, f) {
  showMenu(e, [
    { label: "Rename…", action: () => startRename(document.querySelector(`.row[data-id="${f.id}"]`), f) },
    { label: "New Folder Inside", action: () => newFolder(f.id) },
    { label: f.collapsed ? "Expand" : "Collapse", action: () => cmd("toggle_folder", { folder_id: f.id }) },
    moveToSpaceItems(f.id, "pinned"),
    { sep: true },
    { label: "Delete Folder (keep tabs)", danger: true, action: () => cmd("delete_folder", { folder_id: f.id }) },
  ]);
}

function folderIds(nodes, out = new Set()) {
  for (const n of nodes) if (n.kind === "folder") (out.add(n.id), folderIds(n.children, out));
  return out;
}

function newFolder(parent) {
  const sp = space();
  renameNext = { parent, known: folderIds(sp.pinned), until: Date.now() + 3000 };
  const p = parent && findFolder(sp.pinned, parent);
  if (p && p.collapsed) cmd("toggle_folder", { folder_id: parent });
  cmd("new_folder", { space_id: null, parent, name: "New Folder" });
}

const SPACE_COLORS = ["#c9c2f0", "#f3c4c4", "#bfe3d0", "#f6dcb0", "#bcd6f2", "#e8c6ea", "#d6d6d6", "#f5e6a8", "#b8e0e6", "#2d2a3e"];

function spaceMenu(e, s) {
  const deletable = view.state.spaces.length > 1;
  showMenu(e, [
    { heading: s.name },
    { label: "New Folder", action: () => (s.id === view.state.active_space ? newFolder(null) : cmd("new_folder", { space_id: s.id, parent: null, name: "New Folder" })) },
    { label: "Rename Space…", action: () => renameSpaceDialog(s) },
    { label: "Change Icon…", action: () => iconDialog(s) },
    { heading: "Color" },
    { swatches: SPACE_COLORS, current: s.color, pick: (c) => cmd("update_space", { space_id: s.id, name: null, icon: null, color: c }) },
    { sep: true },
    deletable && { label: "Delete Space…", danger: true, action: () => deleteSpaceDialog(s) },
  ]);
}

function recentlyClosedItems() {
  const items = (view.state.archive || []).slice(0, 15);
  if (!items.length) return null;
  return {
    label: "Recently Closed",
    sub: [
      { label: "Reopen Last (Ctrl+Shift+T)", action: () => cmd("reopen_closed") },
      { sep: true },
      ...items.map((a, i) => ({ label: a.title || hostOf(a.url), action: () => cmd("reopen_archived", { index: i }) })),
    ],
  };
}

function archiveDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Auto-Archive"));
    d.append(el("p", null, "Unpinned tabs you haven't used for a while are closed and kept under Recently Closed. Pinned tabs and favorites are never archived."));
    const sel = el("select");
    for (const [h, label] of [[0, "Never"], [12, "After 12 hours"], [24, "After 1 day"], [168, "After 7 days"], [720, "After 30 days"]]) {
      const o = el("option", null, label);
      o.value = h;
      o.selected = h === view.state.settings.archive_after_hours;
      sel.append(o);
    }
    d.append(sel);
    buttons(d, "Save", () => act("set_archive_hours", { hours: Number(sel.value) }));
  });
}

function updateDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Burrow " + view.update.current));
    const status = el("p");
    const check = el("button", "btn", "Check Now");
    check.onclick = () => act("check_for_updates");
    const fill = () => {
      const u = view.update;
      check.disabled = u.checking || u.downloading;
      status.textContent = u.checking ? "Checking for updates…"
        : u.ready ? `Burrow ${u.available} is downloaded. It installs when you restart or quit Burrow.`
        : u.downloading ? `Downloading Burrow ${u.available}…`
        : u.available ? `Burrow ${u.available} is out.` + (u.installable ? "" : " This copy can't update itself; download it from the release page.")
        : "Burrow checks GitHub for new versions after it starts and every six hours.";
    };
    fill();
    dialogRefresh = fill;
    d.append(status);
    const l = el("label", "check");
    const auto = el("input");
    auto.type = "checkbox";
    auto.checked = view.state.settings.auto_update;
    auto.onchange = () => act("set_auto_update", { on: auto.checked });
    l.append(auto, document.createTextNode("Check for updates automatically"));
    d.append(l);
    const row = el("div", "buttons");
    const close = el("button", "btn primary", "Close");
    close.onclick = closeDialog;
    row.append(check, close);
    d.append(row);
  });
}

function addMenu(e) {
  showMenu(e, [
    { label: "New Space…", action: newSpaceDialog },
    view.arc_available && { label: "Import from Arc…", action: importArcDialog },
    { sep: true },
    recentlyClosedItems(),
    { label: "History…  (Ctrl+H)", action: historyDialog },
    { label: "Air Traffic Control…", action: () => atcDialog("") },
    { label: "Links and Peek…", action: peekDialog },
    { label: "Search Engine…", action: searchDialog },
    { label: "Auto-Archive…", action: archiveDialog },
    {
      label: view.state.settings.auto_pip ? "Turn Off Auto Picture in Picture" : "Turn On Auto Picture in Picture",
      action: () => act("set_auto_pip", { on: !view.state.settings.auto_pip }),
    },
    { label: "Extensions…", action: extensionsDialog },
    { label: "Keyboard Shortcuts", action: shortcutsDialog },
    { label: view.update.ready ? `Restart to Update to ${view.update.available}` : "About and Updates…", action: view.update.ready ? () => act("install_update") : updateDialog },
    {
      label: view.state.settings.agent_page_access ? "Block Agents from Pages" : "Let Agents Read and Use Pages",
      action: () => act("set_agent_page_access", { on: !view.state.settings.agent_page_access }),
    },
    {
      label: view.state.settings.system_title_bar ? "Use Arc-Style Window" : "Use Windows Title Bar",
      action: () => act("set_system_title_bar", { on: !view.state.settings.system_title_bar }),
    },
    view.windows && { label: "Make Burrow Your Default Browser…", action: () => act("make_default_browser") },
    { label: "Sidebar DevTools", action: () => act("dev_tools", { sidebar: true }) },
  ]);
}

// ---------------------------------------------------------------------------------------------
// Dialogs

const backdrop = $("dialog-backdrop");
const dialog = $("dialog");

let dialogRefresh = null; // rebuilds the open dialog's live parts when the state changes
let historyView = null; // fills the History dialog's list

function openDialog(build) {
  hideMenu();
  dialogRefresh = null;
  historyView = null;
  dialog.replaceChildren();
  build(dialog);
  backdrop.hidden = false;
  busy++;
  dialog.querySelector("input,select,.btn.primary")?.focus();
}
function closeDialog() {
  if (backdrop.hidden) return;
  backdrop.hidden = true;
  dialog.classList.remove("wide");
  dialogRefresh = null;
  historyView = null;
  busy--;
  if (pendingRender) render();
}
backdrop.addEventListener("mousedown", (e) => e.target === backdrop && closeDialog());

function buttons(d, primaryLabel, onPrimary, danger) {
  const row = el("div", "buttons");
  const cancel = el("button", "btn", "Cancel");
  cancel.onclick = closeDialog;
  const ok = el("button", "btn primary", primaryLabel);
  if (danger) ok.style.background = "#c4314b";
  ok.onclick = () => {
    if (onPrimary() !== false) closeDialog();
  };
  row.append(cancel, ok);
  d.append(row);
  d.onkeydown = (e) => e.key === "Enter" && e.target.tagName !== "BUTTON" && ok.click();
}

const EMOJIS = ["", "🏠", "💼", "💻", "🧪", "📚", "🎨", "🎵", "🛒", "💰", "📈", "✈️", "🎮", "❤️", "⭐", "🔥", "🌱", "🕍", "👨‍👩‍👧", "🤖"];

function emojiPicker(current) {
  let chosen = current || "";
  const row = el("div", "emoji-row");
  for (const em of EMOJIS) {
    const b = el("button", em === chosen ? "on" : "", em || "•");
    b.onclick = () => {
      chosen = em;
      row.querySelectorAll("button").forEach((x) => x.classList.toggle("on", x === b));
    };
    row.append(b);
  }
  return { row, value: () => chosen };
}

function newSpaceDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "New Space"));
    const nameL = el("label", null, "Name");
    const name = el("input");
    name.placeholder = "Work";
    nameL.append(name);
    const picker = emojiPicker("");
    const iconL = el("label", null, "Icon");
    iconL.append(picker.row);
    const profL = el("label", null, "Logins");
    const prof = el("select");
    const fresh = el("option", null, "Separate logins (new profile)");
    fresh.value = "";
    prof.append(fresh);
    for (const p of view.state.profiles) {
      const users = view.state.spaces.filter((s) => s.profile_id === p.id).map((s) => s.name);
      const o = el("option", null, `Share logins with ${users.length ? users.join(", ") : p.name}`);
      o.value = p.id;
      prof.append(o);
    }
    profL.append(prof);
    d.append(nameL, iconL, profL, el("p", null, "Separate logins let you be signed in to different accounts in each space, all in this one window."));
    buttons(d, "Create Space", () => {
      cmd("new_space", { name: name.value.trim() || "New Space", icon: picker.value(), profile_id: prof.value || null });
    });
  });
}

function renameSpaceDialog(s) {
  openDialog((d) => {
    d.append(el("h2", null, "Rename Space"));
    const name = el("input");
    name.value = s.name;
    d.append(name);
    buttons(d, "Save", () => {
      if (name.value.trim()) cmd("update_space", { space_id: s.id, name: name.value.trim(), icon: null, color: null });
    });
  });
}

function iconDialog(s) {
  openDialog((d) => {
    d.append(el("h2", null, "Space Icon"));
    const picker = emojiPicker(s.icon);
    const custom = el("input");
    custom.placeholder = "…or paste any emoji";
    d.append(picker.row, custom);
    buttons(d, "Save", () => {
      const icon = custom.value.trim() || picker.value();
      cmd("update_space", { space_id: s.id, name: null, icon, color: null });
    });
  });
}

function deleteSpaceDialog(s) {
  openDialog((d) => {
    d.append(el("h2", null, `Delete “${s.name}”?`));
    d.append(el("p", null, "Its pinned tabs, folders and favorites are removed. Logins in its profile are kept."));
    buttons(d, "Delete", () => cmd("delete_space", { space_id: s.id }), true);
  });
}

function importArcDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Import from Arc"));
    d.append(el("p", null, "Copies your Arc spaces, folders, pinned tabs and favorites into Burrow. Arc is not changed. Logins are not imported — sign in once in each space."));
    buttons(d, "Import", () => act("import_arc"));
  });
}

const ENGINES = [
  ["Google", "https://www.google.com/search?q={}"],
  ["Bing", "https://www.bing.com/search?q={}"],
  ["DuckDuckGo", "https://duckduckgo.com/?q={}"],
  ["Brave Search", "https://search.brave.com/search?q={}"],
  ["Startpage", "https://www.startpage.com/do/search?q={}"],
];

function searchDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Search Engine"));
    const sel = el("select");
    const current = view.state.settings.search_url;
    for (const [n, u] of ENGINES) {
      const o = el("option", null, n);
      o.value = u;
      o.selected = u === current;
      sel.append(o);
    }
    const customO = el("option", null, "Custom…");
    customO.value = "";
    customO.selected = !ENGINES.some(([, u]) => u === current);
    sel.append(customO);
    const custom = el("input");
    custom.placeholder = "https://example.com/search?q={}";
    custom.value = customO.selected ? current : "";
    d.append(sel, custom, el("p", null, "Custom URLs use {} where the search words go."));
    buttons(d, "Save", () => {
      const url = sel.value || custom.value.trim();
      if (!url.includes("{}")) return false;
      act("set_search", { url });
    });
  });
}

function shortcutsDialog() {
  const rows = [
    ["Ctrl+T", "New tab"], ["Ctrl+L", "Edit address"], ["Ctrl+W", "Close tab (pinned: unload)"],
    ["Ctrl+D", "Pin / unpin tab"], ["Ctrl+1…9", "Switch space"], ["Ctrl+Tab", "Next tab"],
    ["Ctrl+S", "Hide / show sidebar"], ["Alt+← / →", "Back / forward"], ["Ctrl+R, F5", "Reload"],
    ["Ctrl+= / − / 0", "Zoom (remembered per site)"], ["Ctrl+F", "Find in page"], ["Ctrl+Shift+T", "Reopen closed tab"],
    ["Ctrl+H", "History"], ["Ctrl+Shift+C", "Copy page address"], ["Ctrl+P", "Print"], ["F11", "Full screen"],
    ["Ctrl+Shift+P", "Picture in picture"],
    ["Esc", "Close a Peek"], ["F12", "DevTools"],
  ];
  openDialog((d) => {
    d.append(el("h2", null, "Keyboard Shortcuts"));
    for (const [k, v] of rows) {
      const p = el("p");
      const b = el("b", null, k);
      b.style.color = "var(--ink)";
      p.append(b, document.createTextNode("  " + v));
      d.append(p);
    }
    const row = el("div", "buttons");
    const ok = el("button", "btn primary", "Done");
    ok.onclick = closeDialog;
    row.append(ok);
    d.append(row);
  });
}

function downloadsDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Downloads"));
    for (const x of view.downloads || []) {
      const item = el("div", "download");
      item.append(el("div", "name", x.name));
      if (!x.done && !x.canceled) {
        const bar = el("div", "bar");
        const fill = el("span");
        fill.style.width = Math.max(0, x.percent) + "%";
        bar.append(fill);
        item.append(bar);
      }
      item.append(el("div", "meta", x.canceled ? "Canceled" : x.done ? "Done — click to open" : `${Math.max(0, x.percent)}%`));
      if (x.done) item.onclick = () => (act("open_download", { path: x.path }), closeDialog());
      d.append(item);
    }
    const row = el("div", "buttons");
    const ok = el("button", "btn primary", "Close");
    ok.onclick = closeDialog;
    row.append(ok);
    d.append(row);
  });
}

// Air Traffic Control (D-021): rules that send a site's links to a space.
function atcDialog(prefill) {
  openDialog((d) => {
    d.append(el("h2", null, "Air Traffic Control"));
    d.append(el("p", null, "Links to these sites open in the space you choose: from other apps, and from tabs in other spaces. A rule with a / (like github.com/my-org) matches only those pages."));
    const list = el("div", "rules");
    d.append(list);
    const fill = () => {
      list.replaceChildren();
      if (!view.state.routes.length) list.append(el("p", null, "No rules yet."));
      for (const r of view.state.routes) {
        const sp = view.state.spaces.find((x) => x.id === r.space_id);
        const row = el("div", "rule");
        row.append(el("span", "pattern", r.pattern), el("span", "to", "→ " + (sp ? (sp.icon ? sp.icon + " " : "") + sp.name : "?")));
        const x = icon("close", "act");
        x.title = "Remove rule";
        x.onclick = () => cmd("remove_route", { route_id: r.id });
        row.append(x);
        list.append(row);
      }
    };
    fill();
    dialogRefresh = fill;
    const pattern = el("input");
    pattern.placeholder = "Site, like figma.com";
    pattern.value = prefill || "";
    const to = el("select");
    for (const sp of view.state.spaces) {
      const o = el("option", null, (sp.icon ? sp.icon + "  " : "") + sp.name);
      o.value = sp.id;
      o.selected = sp.id === view.state.active_space;
      to.append(o);
    }
    const add = el("button", "btn", "Add Rule");
    add.onclick = () => {
      if (!pattern.value.trim()) return pattern.focus();
      cmd("add_route", { pattern: pattern.value, space_id: to.value });
      pattern.value = "";
      pattern.focus();
    };
    const form = el("div", "rule-form");
    form.append(pattern, to, add);
    d.append(form);
    const row = el("div", "buttons");
    const ok = el("button", "btn primary", "Done");
    ok.onclick = closeDialog;
    row.append(ok);
    d.append(row);
    d.onkeydown = (e) => e.key === "Enter" && e.target === pattern && add.click();
  });
}

function peekDialog() {
  openDialog((d) => {
    d.append(el("h2", null, "Links and Peek"));
    d.append(el("p", null, "A Peek shows a page in a card over the one you're on. Press Esc to close it, or Open as Tab to keep it."));
    const check = (text, on) => {
      const l = el("label", "check");
      const c = el("input");
      c.type = "checkbox";
      c.checked = on;
      l.append(c, document.createTextNode(text));
      d.append(l);
      return c;
    };
    const newTabs = check("Open links that would open a new tab in a Peek", view.state.settings.peek_new_tabs);
    const pinned = check("In pinned tabs and favorites, open links to other sites in a Peek", view.state.settings.peek_from_pinned);
    d.append(el("p", null, "Ctrl+click and middle-click still open background tabs. Right-click a link for more choices."));
    buttons(d, "Save", () => act("set_peek", { new_tabs: newTabs.checked, from_pinned: pinned.checked }));
  });
}

function boostDialog(host) {
  openDialog((d) => {
    d.append(el("h2", null, "Boost " + host));
    d.append(el("p", null, "Your own CSS for every page of this site, e.g. body { font-size: 18px } or .ads { display: none }"));
    const css = el("textarea");
    css.spellcheck = false;
    css.value = view.state.boosts[host] || "";
    css.placeholder = "body { … }";
    d.append(css);
    if (view.state.boosts[host]) {
      const remove = el("button", "btn", "Remove Boost");
      remove.onclick = () => (act("set_boost", { host, css: "" }), closeDialog());
      d.append(remove);
    }
    buttons(d, "Save", () => act("set_boost", { host, css: css.value }));
    d.onkeydown = null; // Enter makes new lines here
  });
}

function timeAgo(ms) {
  const m = Math.round((Date.now() - ms) / 60000);
  if (m < 1) return "now";
  if (m < 60) return `${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h`;
  const days = Math.round(h / 24);
  return days < 30 ? `${days} d` : new Date(ms).toLocaleDateString();
}

function historyDialog() {
  openDialog((d) => {
    d.classList.add("wide");
    d.append(el("h2", null, "History"));
    const q = el("input");
    q.placeholder = "Search history";
    const list = el("div", "history");
    d.append(q, list);
    historyView = (query, visits) => {
      if (query !== q.value.trim()) return;
      list.replaceChildren();
      if (!visits.length) list.append(el("p", null, query ? "Nothing found." : "No history yet."));
      for (const v of visits) {
        const row = el("div", "row");
        const ico = el("span", "ico");
        ico.append(favicon({ id: "", url: v.url }));
        const text = el("span", "label");
        text.append(el("span", "t", v.title || v.url), el("span", "u", `${hostOf(v.url)} · ${timeAgo(v.last_ms)}`));
        const x = icon("close", "act");
        x.title = "Remove from history";
        x.onclick = (e) => {
          e.stopPropagation();
          act("delete_history", { url: v.url });
          row.remove();
        };
        row.title = v.url;
        row.append(ico, text, x);
        row.onclick = () => (closeDialog(), act("open_url", { url: v.url }));
        list.append(row);
      }
    };
    let timer = null;
    q.oninput = () => (clearTimeout(timer), (timer = setTimeout(() => act("history_list", { query: q.value.trim() }), 120)));
    act("history_list", { query: "" });
    const row = el("div", "buttons");
    const clear = el("button", "btn", "Clear History…");
    clear.onclick = () => {
      if (clear.dataset.sure) {
        act("clear_history");
        closeDialog();
      } else {
        clear.dataset.sure = "1";
        clear.textContent = "Click again to clear all";
        clear.style.color = "#c4314b";
      }
    };
    const ok = el("button", "btn primary", "Done");
    ok.onclick = closeDialog;
    row.append(clear, ok);
    d.append(row);
  });
}

// ---------------------------------------------------------------------------------------------
// Extensions (D-032)

/** The Web Store id of a store page's address, if it is one. */
function webStoreId(url) {
  const m = /^https:\/\/(?:chromewebstore\.google\.com\/detail\/(?:[^/?#]+\/)?|chrome\.google\.com\/webstore\/detail\/(?:[^/?#]+\/)?)([a-p]{32})(?:[/?#]|$)/.exec(url || "");
  return m ? m[1] : null;
}
const extIconUrl = (x) => (x.icon ? `burrow://ext/${x.dir}/${x.icon.split("/").map(encodeURIComponent).join("/")}` : null);
const extRunning = (x) => view.extensions.running.includes(x.id);

/** An extension's icon, or a puzzle piece if it has none (or it doesn't load). */
function extImage(url, cls) {
  if (!url) return icon("puzzle", "ico");
  const img = el("img", cls);
  img.onerror = () => img.replaceWith(icon("puzzle", "ico"));
  img.src = url;
  return img;
}

let restartLater = null; // the extension list the user said "Later" to
let updateLater = null; // the version the user said "Later" to

function renderUpdatePrompt(box) {
  const u = view.update;
  if (!u.available || updateLater === u.available || (u.installable && !u.ready)) return;
  const card = el("div", "prompt");
  card.append(el("div", "what", u.ready
    ? `Burrow ${u.available} is ready. Restart to update, or it installs when you quit.`
    : `Burrow ${u.available} is out.`));
  const buttons = el("div", "buttons");
  const later = el("button", "btn", "Later");
  later.onclick = () => ((updateLater = u.available), render());
  const go = el("button", "btn allow", u.ready ? "Restart" : "Download");
  go.onclick = () => (u.ready ? act("install_update") : act("open_url", { url: u.page }));
  buttons.append(el("label"), later, go);
  card.append(buttons);
  box.append(card);
}

function renderExtensionPrompts(box) {
  const xv = view.extensions;
  const list = JSON.stringify(view.state.extensions.map((x) => [x.id, x.version, x.enabled]));
  if (xv.restart_needed && restartLater !== list) {
    const card = el("div", "prompt");
    card.append(el("div", "what", "Restart Burrow to apply your extension changes. Your tabs come back as they are."));
    const buttons = el("div", "buttons");
    const later = el("button", "btn", "Later");
    later.onclick = () => ((restartLater = list), render());
    const now = el("button", "btn allow", "Restart");
    now.onclick = () => act("restart");
    buttons.append(el("label"), later, now);
    card.append(buttons);
    box.append(card);
  }
  // On a Chrome Web Store page: its "Add to Chrome" doesn't work here, so offer Burrow's.
  const tab = view.shown ? findTab(view.shown) : null;
  const id = tab && webStoreId(tab.url);
  if (id && !view.state.extensions.some((x) => x.id === id)) {
    const card = el("div", "prompt");
    const installing = xv.installing.length > 0;
    card.append(el("div", "what", installing ? "Installing…" : "Add this extension to Burrow?"));
    const buttons = el("div", "buttons");
    const add = el("button", "btn allow", "Add to Burrow");
    add.disabled = installing;
    add.onclick = () => act("install_extension", { input: tab.url });
    buttons.append(el("label"), add);
    card.append(buttons);
    box.append(card);
  }
}

function extensionsMenu(e) {
  const on = view.state.extensions.filter((x) => x.enabled);
  const items = on.map((x) => ({
    label: extRunning(x) ? x.name : `${x.name} (after restart)`,
    img: extIconUrl(x),
    action: () => act("open_extension", { id: x.id }),
  }));
  if (!on.length) items.push({ heading: view.state.extensions.length ? "All extensions are off" : "No extensions yet" });
  items.push(
    { sep: true },
    { label: "Manage Extensions…", action: extensionsDialog },
    { label: "Chrome Web Store", action: () => act("open_url", { url: "https://chromewebstore.google.com/" }) },
  );
  showMenu(e, items);
}

function extensionsDialog() {
  openDialog((d) => {
    d.classList.add("wide");
    d.append(el("h2", null, "Extensions"));
    d.append(el("p", null, "Extensions from the Chrome Web Store run in every space, each space with its own storage and sign-ins. Adding, removing, updating or turning one on or off takes effect when Burrow restarts."));
    const notice = el("div");
    const list = el("div", "ext-list");
    d.append(notice, list);
    const fill = () => {
      const xv = view.extensions;
      notice.replaceChildren();
      if (xv.restart_needed) {
        const n = el("div", "notice");
        const restart = el("button", "btn primary", "Restart Now");
        restart.onclick = () => act("restart");
        n.append(el("span", null, "Restart Burrow to apply your changes."), restart);
        notice.append(n);
      }
      list.replaceChildren();
      for (const name of xv.installing) {
        const row = el("div", "ext");
        const text = el("div", "text");
        text.append(el("div", "name", name), el("div", "meta", "Installing…"));
        row.append(el("span", "spinner"), text);
        list.append(row);
      }
      if (!view.state.extensions.length && !xv.installing.length) list.append(el("p", null, "No extensions yet. Add one from the Chrome Web Store: open its page and click Add to Burrow, or paste its link below."));
      for (const x of view.state.extensions) {
        const row = el("div", "ext" + (x.enabled ? "" : " off"));
        const text = el("div", "text");
        const status = extRunning(x) ? "" : x.enabled ? " · starts after a restart" : " · off";
        text.append(el("div", "name", x.name), el("div", "meta", `${x.version} · ${x.source === "web_store" ? "Chrome Web Store" : "from a file"}${status}`));
        text.title = x.description || x.name;
        row.append(extImage(extIconUrl(x)), text);
        if (x.options && extRunning(x)) {
          const opts = el("button", "link", "Options");
          opts.onclick = () => (act("open_url", { url: `chrome-extension://${x.id}/${x.options}` }), closeDialog());
          row.append(opts);
        }
        const toggle = el("input");
        toggle.type = "checkbox";
        toggle.checked = x.enabled;
        toggle.title = x.enabled ? "Turn off" : "Turn on";
        toggle.onchange = () => act("set_extension_enabled", { id: x.id, enabled: toggle.checked });
        const remove = icon("close", "act");
        remove.title = "Remove";
        remove.onclick = () => {
          if (remove.classList.contains("sure")) act("remove_extension", { id: x.id });
          else (remove.classList.add("sure"), (remove.title = "Click again to remove " + x.name));
        };
        row.append(toggle, remove);
        list.append(row);
      }
    };
    fill();
    dialogRefresh = fill;
    const input = el("input");
    input.placeholder = "Chrome Web Store link or extension id";
    const shown = view.shown ? findTab(view.shown) : null;
    if (shown && webStoreId(shown.url)) input.value = shown.url;
    const add = el("button", "btn primary", "Add");
    add.onclick = () => {
      if (!input.value.trim()) return input.focus();
      act("install_extension", { input: input.value.trim() });
      input.value = "";
    };
    const form = el("div", "ext-form");
    form.append(input, add);
    d.append(form);
    d.onkeydown = (e) => e.key === "Enter" && e.target === input && add.click();
    const more = el("div", "buttons left");
    const folder = el("button", "btn", "From a Folder…");
    folder.onclick = () => act("choose_extension_file", { folder: true });
    const file = el("button", "btn", "From a .crx File…");
    file.onclick = () => act("choose_extension_file", { folder: false });
    const update = el("button", "btn", "Check for Updates");
    update.onclick = () => act("update_extensions");
    more.append(folder, file, update);
    d.append(more);
    const row = el("div", "buttons");
    const ok = el("button", "btn primary", "Done");
    ok.onclick = closeDialog;
    row.append(ok);
    d.append(row);
  });
}

// ---------------------------------------------------------------------------------------------
// Resizing the sidebar by its right edge (D-033)

const SIDEBAR_DEFAULT_WIDTH = 264;
const handle = $("resize-handle");
let resizing = null; // { x, width, last, frame } while dragging
handle.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  e.preventDefault();
  handle.setPointerCapture(e.pointerId);
  resizing = { x: e.screenX, width: window.innerWidth, last: null, frame: 0 };
  busy++;
  document.body.classList.add("resizing");
});
handle.addEventListener("pointermove", (e) => {
  if (!resizing) return;
  resizing.last = Math.round(resizing.width + e.screenX - resizing.x);
  // At most one resize per frame; Rust lays the window out for each.
  if (!resizing.frame) {
    resizing.frame = requestAnimationFrame(() => {
      if (!resizing) return;
      resizing.frame = 0;
      act("set_sidebar_width", { width: resizing.last, save: false });
    });
  }
});
function endResize() {
  if (!resizing) return;
  cancelAnimationFrame(resizing.frame);
  if (resizing.last !== null) act("set_sidebar_width", { width: resizing.last, save: true });
  resizing = null;
  busy--;
  document.body.classList.remove("resizing");
  if (pendingRender) render();
}
handle.addEventListener("pointerup", endResize);
handle.addEventListener("lostpointercapture", endResize);
handle.addEventListener("dblclick", () => act("set_sidebar_width", { width: SIDEBAR_DEFAULT_WIDTH, save: true }));

// ---------------------------------------------------------------------------------------------
// Toast

let toastTimer = null;
function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 5000);
}
$("toast").onclick = () => ($("toast").hidden = true);

// ---------------------------------------------------------------------------------------------
// Static wiring

$("btn-toggle").onclick = () => act("toggle_sidebar");

// Hidden sidebar: hovering its edge strip slides it out; leaving slides it back (Arc-style).
let peeking = false;
document.documentElement.addEventListener("mouseenter", () => {
  if (view && view.sidebar_collapsed && !peeking) {
    peeking = true;
    act("peek_sidebar", { on: true });
  }
});
document.documentElement.addEventListener("mouseleave", () => {
  if (peeking && busy === 0 && menu.hidden && document.activeElement !== addressInput) {
    peeking = false;
    act("peek_sidebar", { on: false });
  }
});
$("toolbar").ondblclick = (e) => e.target === $("toolbar") && act("window_maximize");
$("btn-back").onclick = () => act("back");
$("btn-forward").onclick = () => act("forward");
$("btn-extensions").onclick = (e) => {
  if (view && view.extensions.popup) return act("close_extension_popup");
  extensionsMenu(e);
};
$("btn-reload").onclick = () => act(view && view.shown && view.live[view.shown]?.loading ? "stop" : "reload");
$("new-tab").onclick = () => window.burrow.focusInput(true);
$("btn-clear").onclick = () => cmd("clear_today");
$("btn-space-menu").innerHTML = ICONS.dots;
$("btn-space-menu").onclick = (e) => spaceMenu(e, space());
$("space-header").oncontextmenu = (e) => spaceMenu(e, space());
$("pinned").oncontextmenu = (e) => e.target === $("pinned") && spaceMenu(e, space());
$("btn-add").innerHTML = ICONS.plus;
$("btn-add").onclick = addMenu;
$("btn-downloads").onclick = downloadsDialog;
$("zoom").onclick = (e) => (e.preventDefault(), act("reset_zoom"));
$("new-tab").querySelector(".ico").innerHTML = ICONS.plus;
document.addEventListener("contextmenu", (e) => e.preventDefault());

act("ready");
