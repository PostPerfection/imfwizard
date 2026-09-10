const RECENT_KEY = "imfwizard-recent-projects";
const RECENT_COLLAPSED_KEY = "imfwizard-recent-projects-collapsed";
const MAX_RECENT = 20;

let configuration = null;

export function initRecentProjects(options) {
  configuration = options;
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

export function renderRecentProjects() {
  const { section, list, onOpen, onQueue, onRetitle, onDelete, afterRender, setStatus } = configuration;
  if (!section || !list) return;
  applyRecentProjectsCollapsed();
  const recent = getRecentProjects();
  if (recent.length === 0) { section.hidden = true; return; }
  section.hidden = false;
  list.innerHTML = recent.map(r => `
    <div class="recent-item" data-path="${r.path}" title="${r.path}">
      <div class="recent-item-text">
        <span class="recent-title">${r.title || r.path.split(/[/\\]/).pop()}</span>
        <span class="recent-path">${r.path}</span>
      </div>
      <button class="recent-queue" data-path="${r.path}" title="Add this IMP to the playlist">+</button>
      <button class="recent-retitle" data-path="${r.path}" title="Give this IMP a new content title">✎</button>
      <button class="recent-delete" data-path="${r.path}" title="Delete this IMP from disk">✕</button>
    </div>
  `).join('');
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
