import { escapeHtml } from "../../extern/guikit/src/html.js";

const RECENT_KEY = "imfwizard-recent-projects";
const RECENT_COLLAPSED_KEY = "imfwizard-recent-projects-collapsed";
const MAX_RECENT = 20;

let configuration = null;
// rows keep the order they were first shown in, only the stored order tracks recency
let shownOrder = [];

export function initRecentProjects(options) {
  configuration = options;
  shownOrder = [];
  options.header?.addEventListener("click", () => {
    localStorage.setItem(RECENT_COLLAPSED_KEY, String(!recentProjectsCollapsed()));
    applyRecentProjectsCollapsed();
  });
}

function recentProjectsCollapsed() {
  return localStorage.getItem(RECENT_COLLAPSED_KEY) !== "false";
}

function applyRecentProjectsCollapsed() {
  const { section, toggle } = configuration;
  if (!section) return;
  const collapsed = recentProjectsCollapsed();
  section.classList.toggle("collapsed", collapsed);
  if (toggle) {
    toggle.textContent = collapsed ? "▶" : "▼";
    toggle.setAttribute("aria-expanded", String(!collapsed));
  }
}

export function getRecentProjects() {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY)) || []; }
  catch { return []; }
}

export function addRecentProject(path, title) {
  let recent = getRecentProjects().filter(r => r.path !== path);
  recent.unshift({ path, title, time: Date.now() });
  if (recent.length > MAX_RECENT) recent = recent.slice(0, MAX_RECENT);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

export function removeRecentProject(path) {
  const recent = getRecentProjects().filter(r => r.path !== path);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

function rowsInShownOrder(recent) {
  const rank = new Map(shownOrder.map((path, index) => [path, index]));
  const fresh = recent.filter(entry => !rank.has(entry.path));
  const known = recent.filter(entry => rank.has(entry.path)).sort((a, b) => rank.get(a.path) - rank.get(b.path));
  const rows = [...fresh, ...known];
  shownOrder = rows.map(entry => entry.path);
  return rows;
}

export function renderRecentProjects() {
  const { section, list, onOpen, onQueue, onRetitle, onDelete, afterRender, setStatus } = configuration;
  if (!section || !list) return;
  applyRecentProjectsCollapsed();
  const recent = getRecentProjects();
  if (recent.length === 0) { section.hidden = true; return; }
  section.hidden = false;
  list.innerHTML = rowsInShownOrder(recent).map(r => {
    const path = escapeHtml(r.path);
    const title = escapeHtml(r.title || r.path.split(/[/\\]/).pop());
    return `
    <div class="recent-item" data-path="${path}" title="${path}">
      <div class="recent-item-text">
        <span class="recent-title">${title}</span>
        <span class="recent-path">${path}</span>
      </div>
      <button class="recent-queue" data-path="${path}" title="Add this IMP to the playlist">+</button>
      <button class="recent-retitle" data-path="${path}" title="Give this IMP a new content title">✎</button>
      <button class="recent-delete" data-path="${path}" title="Delete this IMP from disk">✕</button>
    </div>
  `;
  }).join('');
  list.querySelectorAll('.recent-queue').forEach(el => {
    el.addEventListener('click', (event) => {
      event.stopPropagation();
      onQueue(el.dataset.path);
      setStatus(`Queued: ${el.dataset.path}`);
    });
  });
  list.querySelectorAll('.recent-retitle').forEach(el => {
    el.addEventListener('click', (event) => {
      event.stopPropagation();
      onRetitle(el.dataset.path);
    });
  });
  list.querySelectorAll('.recent-delete').forEach(el => {
    el.addEventListener('click', (event) => {
      event.stopPropagation();
      onDelete(el.dataset.path);
    });
  });
  list.querySelectorAll('.recent-item').forEach(el => {
    el.addEventListener('click', () => onOpen(el.dataset.path));
  });
  afterRender();
}
