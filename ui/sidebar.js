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

function favicon(tab, size = 16) {
  if (isLoading(tab.id)) return el("span", "spinner");
  // Tabs that never loaded in Burrow have no favicon yet: try the site's /favicon.ico.
  let src = tab.favicon && /^(https?:|data:image\/)/.test(tab.favicon) ? tab.favicon : null;
  if (!src && /^https?:/.test(tab.url)) {
    try { src = new URL(tab.url).origin + "/favicon.ico"; } catch { /* not a URL */ }
  }
  if (src) {
    const img = el("img", "favicon");
    img.width = img.height = size;
    // Swap the source rather than the element: errors can fire before the img is attached.
    img.onerror = () => {
      img.onerror = null;
      img.src = GLOBE_IMG;
    };
    img.src = src;
    return img;
  }
  return icon("globe", "ico");
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

function scheduleRender() {
  if (busy > 0) pendingRender = true;
  else render();
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
  $("btn-toggle").innerHTML = ICONS.sidebar;
  $("window-buttons").hidden = !view.frameless;
  $("agent-badge").hidden = !view.agent_active;
  $("btn-min").innerHTML = ICONS.winMin;
  $("btn-max").innerHTML = view.maximized ? ICONS.winRestore : ICONS.winMax;
  $("btn-max").title = view.maximized ? "Restore" : "Maximize";
  $("btn-close").innerHTML = ICONS.winClose;
  const nav = (view.shown && view.live[view.shown]) || {};
  const back = $("btn-back"), fwd = $("btn-forward"), reload = $("btn-reload");
  back.innerHTML = ICONS.back;
  fwd.innerHTML = ICONS.forward;
  back.disabled = !nav.can_back;
  fwd.disabled = !nav.can_forward;
  reload.innerHTML = nav.loading ? ICONS.stop : ICONS.reload;
  reload.title = nav.loading ? "Stop" : "Reload (Ctrl+R)";
  reload.disabled = !view.shown;
}

function renderAddress() {
  const input = $("address-input");
  const tab = view.shown ? findTab(view.shown) : null;
  $("address-lock").innerHTML = ICONS.lock;
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
    b.classList.toggle("active", t.id === view.shown);
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
  const ico = el("span", "ico");
  ico.append(favicon(t));
  row.append(ico, el("span", "label", titleOf(t)));
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
  b.innerHTML = ICONS.download;
  const d = view.downloads || [];
  b.classList.toggle("has", d.length > 0);
  b.classList.toggle("busy", d.some((x) => !x.done && !x.canceled));
}

// ---------------------------------------------------------------------------------------------
// Address box

const addressInput = $("address-input");
let suggestions = []; // tabs matching what's typed
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
  const box = $("suggest");
  suggestions = [];
  suggestIndex = -1;
  if (addressMode === null || addressMode === "find" || q.length < 2) {
    box.hidden = true;
    return;
  }
  const active = view.state.active_space;
  suggestions = allTabs()
    .filter(([t]) => titleOf(t).toLowerCase().includes(q) || t.url.toLowerCase().includes(q))
    .sort(([, a], [, b]) => (a.id === active ? 0 : 1) - (b.id === active ? 0 : 1))
    .slice(0, 6);
  box.replaceChildren();
  suggestions.forEach(([t, s], i) => {
    const row = el("div", "row");
    const ico = el("span", "ico");
    ico.append(favicon(t));
    row.append(ico, el("span", "label", titleOf(t)));
    if (s.id !== active) row.append(el("span", "space-tag", (s.icon ? s.icon + " " : "") + s.name));
    row.onmousedown = (e) => {
      e.preventDefault();
      pickSuggestion(i);
    };
    box.append(row);
  });
  if (suggestions.length) box.append(el("div", "hint", "↑↓ to pick a tab · Enter to search"));
  box.hidden = suggestions.length === 0;
}

function highlightSuggestion(i) {
  suggestIndex = Math.max(-1, Math.min(suggestions.length - 1, i));
  $("suggest").querySelectorAll(".row").forEach((r, k) => r.classList.toggle("sel", k === suggestIndex));
}

function pickSuggestion(i) {
  const [t] = suggestions[i];
  leaveAddress();
  cmd("select_tab", { tab_id: t.id });
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
document.addEventListener("keydown", (e) => e.key === "Escape" && (hideMenu(), closeDialog()));
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
  showMenu(e, [
    section === "today" && { label: "Pin Tab", action: () => move({ section: "pinned" }) },
    section === "pinned" && { label: "Unpin Tab", action: () => move({ section: "today", index: 0 }) },
    section !== "favorites" && { label: "Add to Favorites", action: () => move({ section: "favorites" }) },
    section === "favorites" && { label: "Remove from Favorites", action: () => move({ section: "today", index: 0 }) },
    drifted(t) && { label: "Back to Pinned Page", action: () => cmd("reset_tab", { tab_id: t.id }) },
    moveToSpaceItems(t.id, section),
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

function addMenu(e) {
  showMenu(e, [
    { label: "New Space…", action: newSpaceDialog },
    view.arc_available && { label: "Import from Arc…", action: importArcDialog },
    { sep: true },
    recentlyClosedItems(),
    { label: "Search Engine…", action: searchDialog },
    { label: "Auto-Archive…", action: archiveDialog },
    { label: "Keyboard Shortcuts", action: shortcutsDialog },
    {
      label: view.state.settings.agent_page_access ? "Block Agents from Pages" : "Let Agents Read and Use Pages",
      action: () => act("set_agent_page_access", { on: !view.state.settings.agent_page_access }),
    },
    {
      label: view.state.settings.system_title_bar ? "Use Arc-Style Window" : "Use Windows Title Bar",
      action: () => act("set_system_title_bar", { on: !view.state.settings.system_title_bar }),
    },
    { label: "Sidebar DevTools", action: () => act("dev_tools", { sidebar: true }) },
  ]);
}

// ---------------------------------------------------------------------------------------------
// Dialogs

const backdrop = $("dialog-backdrop");
const dialog = $("dialog");

function openDialog(build) {
  hideMenu();
  dialog.replaceChildren();
  build(dialog);
  backdrop.hidden = false;
  busy++;
  dialog.querySelector("input,select,.btn.primary")?.focus();
}
function closeDialog() {
  if (backdrop.hidden) return;
  backdrop.hidden = true;
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
    ["Ctrl+= / − / 0", "Zoom"], ["Ctrl+F", "Find in page"], ["Ctrl+Shift+T", "Reopen closed tab"], ["F12", "DevTools"],
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
$("btn-min").onclick = () => act("window_minimize");
$("btn-max").onclick = () => act("window_maximize");
$("btn-close").onclick = () => act("window_close");
$("toolbar").ondblclick = (e) => e.target === $("toolbar") && act("window_maximize");
$("btn-back").onclick = () => act("back");
$("btn-forward").onclick = () => act("forward");
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
$("new-tab").querySelector(".ico").innerHTML = ICONS.plus;
document.addEventListener("contextmenu", (e) => e.preventDefault());

act("ready");
