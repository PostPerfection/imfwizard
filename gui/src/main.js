import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Command } from "@tauri-apps/plugin-shell";
import { open as _open, confirm as tauriConfirm, message as tauriMessage } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { documentDir, join } from "@tauri-apps/api/path";
import { initPreview, previewFile, previewDcp, previewPlayPause, previewSeek, previewSeekAbsolute, isPreviewVisible, setPreviewCrop, setPreviewSubtitleFile } from "../../extern/guikit/src/preview.js";
import { initPlaylist, addToPlaylist } from "../../extern/guikit/src/playlist.js";
import { initTimeline, loadTimelineFromCpl } from "./timeline.js";
import { initShortcuts, getBinding } from "../../extern/guikit/src/shortcuts.js";

// === Browse wrapper ===
let lastBrowseDir = null;
async function open(opts = {}) {
  const result = await _open({ ...opts, defaultPath: opts.defaultPath || lastBrowseDir || undefined });
  if (result) {
    lastBrowseDir = opts.directory ? result : result.replace(/[/\\][^/\\]*$/, '');
  }
  return result;
}

// === Sidebar navigation ===
document.querySelectorAll(".sidebar-btn[data-view]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".sidebar-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    const view = document.getElementById(`view-${btn.dataset.view}`);
    if (view) view.classList.add("active");

    if (btn.dataset.view === "jobs") { refreshJobs(); startJobsPolling(); }
    else { stopJobsPolling(); }
  });
});

// === Theme toggle ===
document.getElementById("theme-toggle")?.addEventListener("click", () => {
  document.body.classList.toggle("light");
  const btn = document.getElementById("theme-toggle");
  btn.textContent = document.body.classList.contains("light") ? "☀️" : "🌙";
});

// === Keyboard shortcuts ===
function switchView(viewName) {
  document.querySelectorAll(".sidebar-btn").forEach((b) => b.classList.remove("active"));
  document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
  const btn = document.querySelector(`.sidebar-btn[data-view="${viewName}"]`);
  if (btn) btn.classList.add("active");
  const view = document.getElementById(`view-${viewName}`);
  if (view) view.classList.add("active");
  if (viewName === "jobs") { refreshJobs(); startJobsPolling(); } else { stopJobsPolling(); }
}

const SHORTCUTS_KEY = "imfwizard-shortcuts";
const PREVIEW_SEEK_SECONDS = 5;

const PROJECT_BUTTON_SHORTCUTS = [
  { id: "new-project", label: "New IMP", binding: "Ctrl+N", buttonId: "btn-new-project" },
  { id: "open-project", label: "Open IMP", binding: "Ctrl+O", buttonId: "btn-open-project" },
  { id: "supplement", label: "Create supplement", binding: "Ctrl+Shift+S", buttonId: "btn-supplement" },
  { id: "build", label: "Create IMP", binding: "Ctrl+B", buttonId: "btn-build" },
  { id: "preview", label: "Preview", binding: "Ctrl+P", buttonId: "btn-preview" },
  { id: "import-video", label: "Import video", binding: "Ctrl+I", buttonId: "import-video" },
];
const THEME_BUTTON_SHORTCUT = { id: "toggle-theme", label: "Toggle light / dark theme", binding: "Ctrl+Shift+T", buttonId: "theme-toggle" };
const BUTTON_SHORTCUTS = [...PROJECT_BUTTON_SHORTCUTS, THEME_BUTTON_SHORTCUT];

function clickAction({ id, label, binding, buttonId }, category) {
  return { id, label, category, binding, handler: () => document.getElementById(buttonId)?.click() };
}

function viewAction(view, label, binding) {
  return { id: `view-${view}`, label, category: "Views", binding, handler: () => switchView(view) };
}

function previewAction(id, label, binding, handler) {
  return { id, label, category: "Preview", binding, when: isPreviewVisible, handler };
}

function refreshButtonTooltips() {
  for (const { id, label, buttonId } of BUTTON_SHORTCUTS) {
    const button = document.getElementById(buttonId);
    if (!button) continue;
    const binding = getBinding(id);
    button.title = binding ? `${label} (${binding})` : label;
  }
}

initShortcuts({
  storageKey: SHORTCUTS_KEY,
  onChange: refreshButtonTooltips,
  actions: [
    ...PROJECT_BUTTON_SHORTCUTS.map((shortcut) => clickAction(shortcut, "Project")),
    viewAction("project", "Project", "Ctrl+1"),
    viewAction("timeline", "Timeline", "Ctrl+2"),
    viewAction("validate", "Validate", "Ctrl+3"),
    viewAction("tools", "Tools", "Ctrl+4"),
    viewAction("deliver", "Deliver", "Ctrl+5"),
    viewAction("jobs", "Jobs", "Ctrl+6"),
    viewAction("settings", "Settings", "Ctrl+7"),
    previewAction("preview-play-pause", "Play / pause", "Space", previewPlayPause),
    previewAction("preview-back", `Back ${PREVIEW_SEEK_SECONDS} seconds`, "ArrowLeft", () => previewSeek(-PREVIEW_SEEK_SECONDS)),
    previewAction("preview-forward", `Forward ${PREVIEW_SEEK_SECONDS} seconds`, "ArrowRight", () => previewSeek(PREVIEW_SEEK_SECONDS)),
    previewAction("preview-start", "Go to start", "Home", () => previewSeekAbsolute(0)),
    clickAction(THEME_BUTTON_SHORTCUT, "Appearance"),
  ],
});
refreshButtonTooltips();

// === Preferences ===
const PREFS_KEY = "imfwizard-preferences";
const PREFS_VERSION = 2;
const PREF_DEFAULTS = {
  profile: "App2e", creator: "", language: "en",
  bandwidth: 250, colourspace: "Rec.709", hdr: "SDR",
  signingCert: "", signingKey: "", outputDir: "",
  showHintsBeforeBuild: true,
};

function getPrefs() {
  try {
    const stored = JSON.parse(localStorage.getItem(PREFS_KEY)) || {};
    if ((stored._version || 0) < PREFS_VERSION) {
      const migrated = { ...PREF_DEFAULTS, ...stored, _version: PREFS_VERSION };
      savePrefs(migrated); return migrated;
    }
    return { ...PREF_DEFAULTS, ...stored };
  } catch { return { ...PREF_DEFAULTS, _version: PREFS_VERSION }; }
}

function savePrefs(prefs) {
  prefs._version = PREFS_VERSION;
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
}

function loadSettings() {
  const prefs = getPrefs();
  const map = {
    "set-profile": prefs.profile, "set-creator": prefs.creator,
    "set-language": prefs.language, "set-bandwidth": prefs.bandwidth,
    "set-colourspace": prefs.colourspace, "set-hdr": prefs.hdr,
    "set-signing-cert": prefs.signingCert, "set-signing-key": prefs.signingKey,
    "set-output-dir": prefs.outputDir,
  };
  for (const [id, val] of Object.entries(map)) {
    const el = document.getElementById(id);
    if (el) el.value = val;
  }
  const showHints = document.getElementById("set-show-hints");
  if (showHints) showHints.checked = prefs.showHintsBeforeBuild;
}

document.getElementById("settings-form")?.addEventListener("submit", (e) => {
  e.preventDefault();
  savePrefs({
    profile: document.getElementById("set-profile")?.value,
    creator: document.getElementById("set-creator")?.value,
    language: document.getElementById("set-language")?.value,
    bandwidth: parseInt(document.getElementById("set-bandwidth")?.value) || 250,
    colourspace: document.getElementById("set-colourspace")?.value,
    hdr: document.getElementById("set-hdr")?.value,
    signingCert: document.getElementById("set-signing-cert")?.value,
    signingKey: document.getElementById("set-signing-key")?.value,
    outputDir: document.getElementById("set-output-dir")?.value,
    showHintsBeforeBuild: !!document.getElementById("set-show-hints")?.checked,
  });
  setStatus("Settings saved");
});

// Advisory findings the pre-build check made. Returns true to build anyway.
function showHintsDialog(hints) {
  const dialog = document.getElementById("hints-dialog");
  const list = document.getElementById("hints-list");
  const silence = document.getElementById("hints-silence");
  if (!dialog || !list) return Promise.resolve(true);

  list.innerHTML = "";
  for (const hint of hints) {
    const item = document.createElement("li");
    item.textContent = hint;
    list.appendChild(item);
  }
  silence.checked = false;
  dialog.hidden = false;

  return new Promise((resolve) => {
    const close = (build) => {
      dialog.hidden = true;
      if (silence.checked) savePrefs({ ...getPrefs(), showHintsBeforeBuild: false });
      loadSettings();
      document.getElementById("hints-build").removeEventListener("click", onBuild);
      document.getElementById("hints-back").removeEventListener("click", onBack);
      resolve(build);
    };
    const onBuild = () => close(true);
    const onBack = () => close(false);
    document.getElementById("hints-build").addEventListener("click", onBuild);
    document.getElementById("hints-back").addEventListener("click", onBack);
  });
}

document.getElementById("set-reset")?.addEventListener("click", () => {
  localStorage.removeItem(PREFS_KEY);
  location.reload();
});

loadSettings();

// === Project State ===
const project = {
  title: "",
  assets: [],
  compositions: [
    { id: 1, name: "Main", contentKind: "feature", segments: [{ id: 1, picture: null, sound: null, subtitle: null }] }
  ],
  activeComposition: 0,
};

// Convenience accessor for active composition segments
function getActiveSegments() {
  return project.compositions[project.activeComposition]?.segments || [];
}
function setActiveSegments(segs) {
  if (project.compositions[project.activeComposition]) {
    project.compositions[project.activeComposition].segments = segs;
  }
}

// Legacy alias for backward compat
Object.defineProperty(project, 'segments', {
  get() { return getActiveSegments(); },
  set(v) { setActiveSegments(v); },
  configurable: true,
});
let nextAssetId = 1;

// === Drop overlay ===
const dropOverlay = document.getElementById("drop-overlay");
document.addEventListener("dragover", (e) => { e.preventDefault(); if (dropOverlay) dropOverlay.hidden = false; });
document.addEventListener("dragleave", (e) => { if (e.relatedTarget === null && dropOverlay) dropOverlay.hidden = true; });
document.addEventListener("drop", (e) => {
  e.preventDefault();
  if (dropOverlay) dropOverlay.hidden = true;
  const files = e.dataTransfer?.files;
  if (files) for (const f of files) importAssetFromPath(f.path || f.name, guessType(f.name));
});

function guessType(name) {
  const ext = name.split('.').pop().toLowerCase();
  if (['mp4','mkv','mov','avi','mxf','webm','j2c','tiff','tif','dpx','exr'].includes(ext)) return 'video';
  if (['wav','aiff','flac','mp3','pcm'].includes(ext)) return 'audio';
  if (['xml','ttml','srt','vtt','imsc'].includes(ext)) return 'subtitle';
  return 'video';
}

// === Asset import ===
document.getElementById("import-video")?.addEventListener("click", async () => {
  const path = await open({ directory: false, multiple: false,
    filters: [{ name: 'Video', extensions: ['mp4','mkv','mov','avi','mxf','webm'] }, { name: 'All', extensions: ['*'] }]
  });
  if (path) importAssetFromPath(path, 'video');
});

document.getElementById("import-audio")?.addEventListener("click", async () => {
  const path = await open({ directory: false, multiple: false,
    filters: [{ name: 'Audio', extensions: ['wav','aiff','flac'] }, { name: 'All', extensions: ['*'] }]
  });
  if (path) importAssetFromPath(path, 'audio');
});

document.getElementById("import-subtitle")?.addEventListener("click", async () => {
  const path = await open({ directory: false, multiple: false,
    filters: [{ name: 'Subtitle', extensions: ['xml','ttml','imsc','srt','vtt'] }, { name: 'All', extensions: ['*'] }]
  });
  if (path) importAssetFromPath(path, 'subtitle');
});

function importAssetFromPath(path, type) {
  const name = path.split(/[/\\]/).pop();
  const asset = { id: nextAssetId++, type, path, name, meta: '' };
  project.assets.push(asset);

  const seg = project.segments[0];
  if (type === 'video' && !seg.picture) seg.picture = asset;
  else if (type === 'audio' && !seg.sound) seg.sound = asset;
  else if (type === 'subtitle' && !seg.subtitle) seg.subtitle = asset;

  renderAssets();
  renderSegments();
  updateStatusStats();
  setStatus(`Imported: ${name}`);

  // Auto-detect video properties
  if (type === 'video') {
    probeVideo(path).then(info => {
      if (!info) return;
      asset.meta = `${info.width}×${info.height} ${info.fps}`;
      if (project.assets.filter(a => a.type === 'video').length === 1) {
        const fpsMatch = info.fps?.match(/^(\d+)\/1$/);
        if (fpsMatch) {
          const fpsEl = document.getElementById("prop-framerate");
          if (fpsEl) {
            const fps = parseInt(fpsMatch[1]);
            for (const opt of fpsEl.options) {
              if (parseInt(opt.value) === fps) { fpsEl.value = opt.value; break; }
            }
          }
        }
      }
      renderAssets();
    });
  }
}

function renderAssets() {
  const list = document.getElementById("asset-list");
  if (!list) return;
  if (project.assets.length === 0) {
    list.innerHTML = '<div class="asset-empty"><p>Drag & drop video/audio files here<br>or use the buttons above</p></div>';
    return;
  }
  const icons = { video: '🎬', audio: '🔊', subtitle: '📝' };
  list.innerHTML = project.assets.map(a => `
    <div class="asset-item" data-asset-id="${a.id}" draggable="true">
      <span class="asset-icon">${icons[a.type]}</span>
      <span class="asset-name" title="${a.path}">${a.name}</span>
      <span class="asset-meta">${a.meta || a.type}</span>
      <button class="asset-remove" data-remove-id="${a.id}" title="Remove from project">✕</button>
    </div>
  `).join('');
  list.querySelectorAll('.asset-item').forEach(el => {
    el.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/plain', el.dataset.assetId); });
    el.addEventListener('contextmenu', (e) => { showContextMenu(e, parseInt(el.dataset.assetId)); });
  });
  list.querySelectorAll('.asset-remove').forEach(el => {
    el.addEventListener('click', (e) => { e.stopPropagation(); removeAsset(parseInt(el.dataset.removeId)); });
  });
  // Re-apply filter
  const q = document.getElementById("asset-filter")?.value?.toLowerCase() || "";
  if (q) {
    list.querySelectorAll('.asset-item').forEach(el => {
      const name = el.querySelector(".asset-name")?.textContent?.toLowerCase() || "";
      el.style.display = name.includes(q) ? "" : "none";
    });
  }
}

function renderSegments() {
  const list = document.getElementById("track-list");
  if (!list) return;
  list.innerHTML = project.segments.map((seg, i) => `
    <div class="reel" data-segment="${seg.id}">
      <div class="reel-header">
        <span class="reel-label">Segment ${i + 1}</span>
        <span class="reel-duration">${seg.picture ? '—' : '--:--:--'}</span>
      </div>
      <div class="reel-tracks">
        <div class="track track-picture" data-seg-id="${seg.id}" data-track="picture">
          <span class="track-label">Picture</span>
          <span class="track-info ${seg.picture ? 'has-content' : ''}">${seg.picture ? seg.picture.name : 'Drop video here'}</span>
        </div>
        <div class="track track-sound" data-seg-id="${seg.id}" data-track="sound">
          <span class="track-label">Sound</span>
          <span class="track-info ${seg.sound ? 'has-content' : ''}">${seg.sound ? seg.sound.name : 'Drop audio here'}</span>
        </div>
        <div class="track track-subtitle" data-seg-id="${seg.id}" data-track="subtitle">
          <span class="track-label">Timed Text</span>
          <span class="track-info ${seg.subtitle ? 'has-content' : ''}">${seg.subtitle ? seg.subtitle.name : 'Optional'}</span>
        </div>
      </div>
    </div>
  `).join('');

  list.querySelectorAll('.track').forEach(track => {
    track.addEventListener('dragover', (e) => { e.preventDefault(); track.style.background = 'var(--surface-hover)'; });
    track.addEventListener('dragleave', () => { track.style.background = ''; });
    track.addEventListener('drop', (e) => {
      e.preventDefault(); track.style.background = '';
      const assetId = parseInt(e.dataTransfer.getData('text/plain'));
      const asset = project.assets.find(a => a.id === assetId);
      if (!asset) return;
      const seg = project.segments.find(s => s.id === parseInt(track.dataset.segId));
      if (seg) { seg[track.dataset.track] = asset; renderSegments(); }
    });
  });

  renderAudioMap();
}

document.getElementById("add-segment")?.addEventListener("click", () => {
  const maxId = project.segments.reduce((m, s) => Math.max(m, s.id), 0);
  project.segments.push({ id: maxId + 1, picture: null, sound: null, subtitle: null });
  renderSegments();
});

// === Multi-CPL Management ===
let nextCplId = 2;

function renderCplTabs() {
  const container = document.getElementById("cpl-tabs");
  if (!container) return;
  container.innerHTML = "";
  project.compositions.forEach((cpl, idx) => {
    const tab = document.createElement("button");
    tab.className = "cpl-tab" + (idx === project.activeComposition ? " active" : "");
    tab.dataset.cpl = idx;
    tab.textContent = cpl.name;
    if (project.compositions.length > 1) {
      const rm = document.createElement("span");
      rm.className = "cpl-tab-remove";
      rm.textContent = "\u00d7";
      rm.addEventListener("click", (e) => {
        e.stopPropagation();
        removeCpl(idx);
      });
      tab.appendChild(rm);
    }
    tab.addEventListener("click", () => switchCpl(idx));
    container.appendChild(tab);
  });
}

function switchCpl(idx) {
  if (idx < 0 || idx >= project.compositions.length) return;
  project.activeComposition = idx;
  renderCplTabs();
  renderSegments();
  const cpl = project.compositions[idx];
  const kindEl = document.getElementById("prop-content-kind");
  if (kindEl && cpl.contentKind) kindEl.value = cpl.contentKind;
}

function removeCpl(idx) {
  if (project.compositions.length <= 1) return;
  project.compositions.splice(idx, 1);
  if (project.activeComposition >= project.compositions.length) {
    project.activeComposition = project.compositions.length - 1;
  }
  renderCplTabs();
  renderSegments();
}

document.getElementById("add-cpl")?.addEventListener("click", () => {
  const name = prompt("Composition name:", `CPL ${nextCplId}`);
  if (!name) return;
  project.compositions.push({
    id: nextCplId++,
    name: name,
    contentKind: "feature",
    segments: [{ id: 1, picture: null, sound: null, subtitle: null }],
  });
  switchCpl(project.compositions.length - 1);
});

document.getElementById("prop-content-kind")?.addEventListener("change", (e) => {
  const cpl = project.compositions[project.activeComposition];
  if (cpl) cpl.contentKind = e.target.value;
});

renderCplTabs();

// === Burn-in during the encode ===
document.getElementById("prop-browse-burn-subtitle")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Subtitles', extensions: ['srt','ass','ssa','scc','fcpxml','mks','mkv'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-burn-subtitle").value = path;
});

document.getElementById("prop-browse-burn-subtitle-font")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Fonts', extensions: ['ttf','otf','ttc'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-burn-subtitle-font").value = path;
});

// === Output directory ===
document.getElementById("browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    const outputEl = document.getElementById("prop-output");
    outputEl.value = dir;
    delete outputEl.dataset.autoFilled;
    refreshDiskSpace();
  }
});

document.getElementById("prop-output")?.addEventListener("input", (event) => {
  delete event.target.dataset.autoFilled;
});

// === Open existing IMP ===
async function openImp(dir) {
  const name = dir.split(/[/\\]/).pop();
  document.getElementById("project-name").textContent = name;
  project.title = name;
  document.getElementById("prop-title").value = name;
  // a name given in the recent list is the label for that row, so opening it keeps it
  addRecentProject(dir, getRecentProjects().find(r => r.path === dir)?.title || name);
  setStatus(`Opened: ${dir}`);

  // Load timeline from the first CPL found
  try {
    const cpls = await invoke('list_cpls', { impDir: dir });
    if (cpls && cpls.length > 0) {
      const cplPath = dir + '/' + cpls[0].file_path;
      loadTimelineFromCpl(cplPath);
    }
  } catch (e) {
    console.warn('[main] Could not load timeline:', e);
  }
}

document.getElementById("btn-open-project")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) openImp(dir);
});

// === Preview ===
const CROP_SIDES = ["left", "right", "top", "bottom"];

// mpv refuses a subtitle track until the clip it goes on has loaded, and
// preview_load only asks for the load, so a duration is the signal it landed.
const PREVIEW_LOAD_POLL_MILLISECONDS = 100;
const PREVIEW_LOAD_POLL_ATTEMPTS = 30;

// A preview opened while an earlier one's subtitle is still converting must not
// be given that track.
let previewGeneration = 0;
let previewShowsJobPicture = false;

document.getElementById("btn-preview")?.addEventListener("click", () => {
  const seg = project.segments[0];
  if (seg?.picture) { previewProjectFile(seg.picture.path); }
  else { tauriMessage("Import a video asset first"); }
});

// The preview shows what the build will do to the picture, so a file a
// composition takes as its picture carries the crop and that segment's timed
// text, and any other file plays plain.
function previewProjectFile(path) {
  previewGeneration += 1;
  const generation = previewGeneration;
  previewFile(path);
  const segment = project.segments.find(s => s.picture?.path === path);
  previewShowsJobPicture = Boolean(segment);
  setPreviewCrop(segment ? currentCrop() : null);
  if (segment?.subtitle?.path) showPreviewSubtitle(segment.subtitle.path, generation);
}

// The built IMP carries the crop in its pictures already, so only its packaged
// timed text is loaded on top.
function previewBuiltPackage(outputDir) {
  previewGeneration += 1;
  const generation = previewGeneration;
  previewShowsJobPicture = false;
  previewDcp(outputDir);
  setPreviewCrop(null);
  showPreviewSubtitle(outputDir, generation);
}

function currentCrop() {
  const crop = {};
  for (const side of CROP_SIDES) {
    crop[side] = parseInt(document.getElementById(`prop-crop-${side}`)?.value) || 0;
  }
  return CROP_SIDES.some(side => crop[side] > 0) ? crop : null;
}

for (const side of CROP_SIDES) {
  document.getElementById(`prop-crop-${side}`)?.addEventListener("input", () => {
    if (previewShowsJobPicture && isPreviewVisible()) setPreviewCrop(currentCrop());
  });
}

async function showPreviewSubtitle(subtitlePath, generation) {
  let playable;
  try {
    playable = await invoke("subtitle_file_for_preview", {
      subtitlePath,
      framerate: document.getElementById("prop-framerate")?.value || "24/1",
    });
  } catch (e) {
    console.error("[preview] subtitles not shown:", e);
    setStatus(`Preview subtitles: ${e}`);
    return;
  }
  if (!playable) return;
  const loaded = await previewClipLoaded();
  if (loaded && generation === previewGeneration) setPreviewSubtitleFile(playable);
}

async function previewClipLoaded() {
  for (let attempt = 0; attempt < PREVIEW_LOAD_POLL_ATTEMPTS; attempt++) {
    const duration = await invoke("preview_get_duration").catch(() => 0);
    if (duration > 0) return true;
    await new Promise(resolve => setTimeout(resolve, PREVIEW_LOAD_POLL_MILLISECONDS));
  }
  return false;
}

// === Supplement ===
document.getElementById("btn-supplement")?.addEventListener("click", () => {
  switchView("timeline");
  // Scroll the supplement panel into view
  setTimeout(() => {
    const supOv = document.getElementById("sup-ov");
    if (supOv) supOv.scrollIntoView({ behavior: "smooth", block: "start" });
  }, 100);
});

// === Source picture processing ===
document.getElementById("prop-auto-crop")?.addEventListener("click", async () => {
  const picture = project.segments[0]?.picture;
  const plan = document.getElementById("prop-picture-plan");
  const button = document.getElementById("prop-auto-crop");
  if (!picture) { tauriMessage("Import a video asset first"); return; }
  const threshold = parseFloat(document.getElementById("prop-auto-crop-threshold")?.value);
  button.disabled = true;
  plan.textContent = "Measuring the black borders...";
  try {
    const crop = await invoke("detect_source_crop", {
      videoPath: picture.path,
      threshold: Number.isFinite(threshold) ? threshold : null,
    });
    for (const side of CROP_SIDES) {
      const field = document.getElementById(`prop-crop-${side}`);
      if (field) field.value = crop[side];
    }
    if (previewShowsJobPicture && isPreviewVisible()) setPreviewCrop(currentCrop());
    plan.textContent = crop.description;
  } catch (e) {
    plan.textContent = "";
    tauriMessage(String(e), { title: "Auto-crop failed", kind: "error" });
  } finally {
    button.disabled = false;
  }
});

// === Audio channel map ===
// The matrix is the CLI's --audio-map: one cell per (input channel, output lane)
// holding that route's gain in dB, empty where the route is left out.
let audioMapShape = null;

async function renderAudioMap() {
  const container = document.getElementById("prop-audio-map");
  if (!container) return;
  const sound = project.segments[0]?.sound;
  if (!sound) {
    audioMapShape = null;
    container.innerHTML = '<div class="audio-map-empty">Drop a WAV on the Sound track to map its channels.</div>';
    return;
  }
  if (audioMapShape?.path === sound.path) return;
  try {
    const shape = await invoke("audio_map_shape", { audioPath: sound.path });
    audioMapShape = { path: sound.path, ...shape };
  } catch (e) {
    audioMapShape = null;
    container.innerHTML = `<div class="audio-map-empty">${e}</div>`;
    return;
  }
  const header = audioMapShape.destination_names.map((name) => `<th>${name}</th>`).join("");
  const rows = [];
  for (let channel = 1; channel <= audioMapShape.input_channels; channel++) {
    const cells = audioMapShape.destination_names
      .map((name) => `<td><input type="text" data-input="${channel}" data-output="${name}" title="Input ${channel} to ${name}, in dB"></td>`)
      .join("");
    rows.push(`<tr><th>${channel}</th>${cells}</tr>`);
  }
  container.innerHTML = `<table><thead><tr><th></th>${header}</tr></thead><tbody>${rows.join("")}</tbody></table>`;
  container.querySelectorAll("input[data-input]").forEach((cell) => {
    cell.addEventListener("click", () => { if (!cell.value) cell.value = "0"; });
  });
}

// Serialise the matrix into the CLI's spec. A bad gain throws so the build stops
// here rather than at the mixer.
function readAudioMap() {
  const container = document.getElementById("prop-audio-map");
  if (!container || !audioMapShape) return null;
  const entries = [];
  container.querySelectorAll("input[data-input]").forEach((cell) => {
    const value = cell.value.trim();
    if (!value) return;
    const gain = Number(value);
    if (!Number.isFinite(gain)) {
      throw new Error(`Channel map gain "${value}" is not a number of decibels`);
    }
    const route = `${cell.dataset.input}:${cell.dataset.output}`;
    entries.push(gain === 0 ? route : `${route}@${gain}`);
  });
  return entries.length ? entries.join(",") : null;
}

// === Build IMP ===
let currentJobId = null;

// durations are spelled as the CLI spells them: frames or seconds, never bare
const DURATION_SPEC = /^(\d+f|\d+(\.\d+)?s)$/i;

// Read the Properties panel's source treatment. A bad duration throws here so a
// build fails before the encode rather than partway through it.
function readSourceSettings() {
  const duration = (id, label) => {
    const value = document.getElementById(id)?.value?.trim();
    if (!value) return null;
    if (!DURATION_SPEC.test(value)) {
      throw new Error(`${label} "${value}" is not a duration: use frames like 48f or seconds like 2s`);
    }
    return value;
  };
  return {
    audioDelayMs: parseInt(document.getElementById("prop-audio-delay")?.value) || 0,
    sourceColourspace: document.getElementById("prop-source-colourspace")?.value || "rec709",
    trimStart: duration("prop-trim-start", "Trim from start"),
    trimEnd: duration("prop-trim-end", "Trim from end"),
    stillLength: duration("prop-still-length", "Still length"),
    burnSubtitle: document.getElementById("prop-burn-subtitle")?.value || null,
    burnSubtitleFont: document.getElementById("prop-burn-subtitle-font")?.value || null,
    burnFontSize: burnPercent("prop-burn-font-size", "Burn-in font size"),
    burnColour: document.getElementById("prop-burn-colour")?.value?.trim() || null,
    burnEffect: document.getElementById("prop-burn-effect")?.value || null,
    burnEffectColour: document.getElementById("prop-burn-effect-colour")?.value?.trim() || null,
    burnOutlineWidth: burnPercent("prop-burn-outline-width", "Burn-in outline width"),
    burnLineHeight: burnMultiple("prop-burn-line-height", "Burn-in line height"),
    burnMargin: burnPercent("prop-burn-margin", "Burn-in margin"),
    burnFadeUp: burnMilliseconds("prop-burn-fade-up", "Burn-in fade up"),
    burnFadeDown: burnMilliseconds("prop-burn-fade-down", "Burn-in fade down"),
    cropLeft: cropPixels("prop-crop-left", "Crop left"),
    cropRight: cropPixels("prop-crop-right", "Crop right"),
    cropTop: cropPixels("prop-crop-top", "Crop top"),
    cropBottom: cropPixels("prop-crop-bottom", "Crop bottom"),
    fillCrop: document.getElementById("prop-fill-crop")?.checked || false,
    deinterlace: document.getElementById("prop-deinterlace")?.checked || false,
    denoise: document.getElementById("prop-denoise")?.checked || false,
    rotate: document.getElementById("prop-rotate")?.value || null,
    flip: document.getElementById("prop-flip")?.value || null,
    raster: document.getElementById("prop-raster")?.value || null,
    audioMap: readAudioMap(),
  };
}

// Blank means the burn keeps the rasteriser's own default, so only a filled
// field is read. The range itself is checked in the build, not here.
function burnPercent(id, label) {
  return burnNonNegativeNumber(id, label, "a percent");
}

function burnMultiple(id, label) {
  return burnNonNegativeNumber(id, label, "a multiple");
}

function burnNonNegativeNumber(id, label, unit) {
  const value = document.getElementById(id)?.value?.trim();
  if (!value) return null;
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) {
    throw new Error(`${label} "${value}" is not ${unit}`);
  }
  return number;
}

function burnMilliseconds(id, label) {
  const value = document.getElementById(id)?.value?.trim();
  if (!value) return null;
  const milliseconds = Number(value);
  if (!Number.isInteger(milliseconds) || milliseconds < 0) {
    throw new Error(`${label} "${value}" is not a whole number of milliseconds`);
  }
  return milliseconds;
}

// A crop is whole pixels off one side, so anything else is a typo, not a crop.
function cropPixels(id, label) {
  const value = document.getElementById(id)?.value?.trim();
  if (!value) return 0;
  const pixels = Number(value);
  if (!Number.isInteger(pixels) || pixels < 0) {
    throw new Error(`${label} "${value}" is not a whole number of pixels`);
  }
  return pixels;
}

document.getElementById("btn-build")?.addEventListener("click", async () => {
  // a second build would queue behind the first and encode all over again
  if (buildInFlight) return;

  const title = document.getElementById("prop-title")?.value?.trim();
  if (!title) { tauriMessage("Enter a content title in Properties"); return; }

  // one composition per CPL tab; each becomes a separate CPL in the IMP
  const multi = project.compositions.length > 1;
  const comps = project.compositions
    .map((c) => {
      const s = c.segments[0];
      if (!s?.picture) return null;
      return {
        title: multi ? `${title} - ${c.name}` : title,
        contentKind: c.contentKind || "feature",
        videoPath: s.picture.path,
        audioPath: s.sound?.path || null,
        subtitles: s.subtitle?.path ? [s.subtitle.path] : [],
      };
    })
    .filter(Boolean);
  if (!comps.length) { tauriMessage("Import a video asset first"); return; }

  let sourceSettings;
  try {
    sourceSettings = readSourceSettings();
  } catch (e) {
    tauriMessage(e.message, { title: "Build failed", kind: "error" });
    return;
  }

  // re-derive an auto-filled output folder so it follows the current title
  const outputEl = document.getElementById("prop-output");
  let output = outputEl?.value;
  if (!output || outputEl?.dataset.autoFilled) {
    const docs = await documentDir();
    output = await join(docs, title);
    if (outputEl) {
      outputEl.value = output;
      outputEl.dataset.autoFilled = "1";
    }
  }

  const progressSection = document.getElementById("progress-section");
  const progressBar = document.getElementById("progress-bar");
  const stageEl = document.getElementById("progress-stage");
  const statsEl = document.getElementById("progress-stats");
  progressSection.style.display = "flex";
  progressBar.value = 0;
  stageEl.textContent = "Queued...";
  statsEl.textContent = "";
  setStatus("");

  const unlisten = await listen("pipeline-progress", (event) => {
    const p = event.payload;
    if (currentJobId && p.job_id !== currentJobId) return;
    progressBar.value = p.percent;
    stageEl.textContent = p.stage.charAt(0).toUpperCase() + p.stage.slice(1);
    setTitleProgress(p.percent, p.stage);
    const elapsed = formatTime(p.elapsed_secs);
    let eta = "";
    if (p.percent > 0 && p.percent < 100) {
      eta = ` ETA ${formatTime((p.elapsed_secs / p.percent) * (100 - p.percent))}`;
    }
    statsEl.textContent = `${elapsed}${p.fps > 0 ? ` ${p.fps.toFixed(1)}fps` : ''}${eta}`;
    if (p.stage === "done") {
      setStatus("Build complete");
      setTitleProgress(-1);
      notifyBuildComplete(true, title);
      addRecentProject(output, title);
      showPostBuildActions(output);
      endBuild();
      unlisten();
    } else if (p.stage === "cancelled") {
      setStatus("Cancelled");
      stageEl.textContent = "Cancelled";
      setTitleProgress(-1);
      endBuild();
      unlisten();
    } else if (p.stage === "error") {
      setStatus("Build failed: " + p.message);
      setTitleProgress(-1);
      notifyBuildComplete(false, title);
      tauriMessage(p.message, { title: "Build failed", kind: "error" });
      endBuild();
      unlisten();
    }
  });

  try {
    beginBuild();
    const submit = (hintsAccepted) => invoke("submit_job", {
      title, outputDir: output, compositions: comps, sourceSettings, hintsAccepted,
      framerate: document.getElementById("prop-framerate")?.value || "24/1",
      bandwidth: parseInt(document.getElementById("prop-bandwidth")?.value) || 250,
    });
    let result = await submit(!getPrefs().showHintsBeforeBuild);
    if (result.jobId === null) {
      if (!await showHintsDialog(result.hints)) {
        progressSection.style.display = "none";
        setStatus("Build cancelled");
        endBuild();
        unlisten();
        return;
      }
      result = await submit(true);
    }
    currentJobId = result.jobId;
    setStatus("Building IMP...");
  } catch (e) {
    stageEl.textContent = "Failed";
    setStatus("Error: " + e);
    tauriMessage(String(e), { title: "Build failed", kind: "error" });
    endBuild();
    unlisten();
  }
});

document.getElementById("progress-cancel")?.addEventListener("click", async () => {
  if (currentJobId) { await invoke("cancel_job", { jobId: currentJobId }); setStatus("Cancelled"); }
});

// === Post-build actions ===
function finishedOutputDir() {
  return document.getElementById("post-build-actions")?.dataset.output;
}

function showPostBuildActions(outputDir) {
  const row = document.getElementById("post-build-actions");
  if (!row) return;
  row.dataset.output = outputDir;
  row.hidden = false;
}

function hidePostBuildActions() {
  const row = document.getElementById("post-build-actions");
  if (row) row.hidden = true;
}

document.getElementById("post-build-play")?.addEventListener("click", () => {
  const output = finishedOutputDir();
  if (output) previewBuiltPackage(output);
});

document.getElementById("post-build-queue")?.addEventListener("click", () => {
  const output = finishedOutputDir();
  if (output) addToPlaylist(output);
});

document.getElementById("post-build-inspect")?.addEventListener("click", () => {
  const output = finishedOutputDir();
  if (!output) return;
  switchView("validate");
  document.getElementById("val-path").textContent = output;
  document.getElementById("val-run").disabled = false;
  runValidation();
});

document.getElementById("post-build-reveal")?.addEventListener("click", () => {
  const output = finishedOutputDir();
  if (output) revealItemInDir(output);
});

// === Validate ===
document.getElementById("val-browse")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("val-path").textContent = dir; document.getElementById("val-run").disabled = false; }
});

async function runValidation() {
  const dir = document.getElementById("val-path").textContent;
  if (!dir || dir.startsWith("No ")) return;
  const box = document.getElementById("val-results");
  box.classList.add("visible");
  box.textContent = "Validating...";
  const cmd = Command.sidecar("imfwizard", ["validate", dir]);
  const result = await cmd.execute();
  box.textContent = result.code === 0
    ? "✓ IMP validation PASSED\n\n" + result.stdout
    : "✗ Validation failed\n\n" + (result.stderr || result.stdout);
  setStatus(result.code === 0 ? "Validation passed" : "Validation failed");
}

document.getElementById("val-run")?.addEventListener("click", runValidation);

// === Tools: Transcode ===
document.getElementById("tc-browse-input")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("tc-input").value = f; checkToolsReady(); }
});
document.getElementById("tc-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("tc-output").value = d; checkToolsReady(); }
});

document.getElementById("tc-start")?.addEventListener("click", async () => {
  const input = document.getElementById("tc-input").value;
  const output = document.getElementById("tc-output").value;
  const format = document.getElementById("tc-format").value;
  const box = document.getElementById("tc-results");
  box.classList.add("visible"); box.textContent = "Transcoding...";
  const cmd = Command.sidecar("imfwizard", ["transcode", "-i", input, "-o", output]);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ Done\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Loudness ===
document.getElementById("loud-browse")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("loud-input").value = f; checkToolsReady(); }
});

document.getElementById("loud-measure")?.addEventListener("click", async () => {
  const input = document.getElementById("loud-input").value;
  const box = document.getElementById("loud-results");
  box.classList.add("visible");
  box.textContent = "Measuring loudness...";
  const cmd = Command.sidecar("imfwizard", ["loudness", input]);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ Loudness Results\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Burn-In ===
document.getElementById("bi-browse-video")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("bi-video").value = f; checkToolsReady(); }
});
document.getElementById("bi-browse-subs")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("bi-subs").value = f; checkToolsReady(); }
});
document.getElementById("bi-browse-output")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("bi-output").value = f; checkToolsReady(); }
});

document.getElementById("bi-start")?.addEventListener("click", async () => {
  const video = document.getElementById("bi-video").value;
  const subs = document.getElementById("bi-subs").value;
  const output = document.getElementById("bi-output").value;
  const box = document.getElementById("bi-results");
  box.classList.add("visible");
  box.textContent = "Burning subtitles...";
  const cmd = Command.sidecar("imfwizard", ["burn-in", "-i", video, "-s", subs, "-o", output]);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ Done\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Analytics ===
document.getElementById("an-browse")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("an-input").value = d; checkToolsReady(); }
});

document.getElementById("an-analyze")?.addEventListener("click", async () => {
  const input = document.getElementById("an-input").value;
  const box = document.getElementById("an-results");
  box.classList.add("visible"); box.textContent = "Analyzing...";
  const cmd = Command.sidecar("imfwizard", ["analytics", "--dir", input, "--json"]);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

function checkToolsReady() {
  const tcBtn = document.getElementById("tc-start");
  if (tcBtn) tcBtn.disabled = !(document.getElementById("tc-input")?.value && document.getElementById("tc-output")?.value);
  const loudBtn = document.getElementById("loud-measure");
  if (loudBtn) loudBtn.disabled = !document.getElementById("loud-input")?.value;
  const biBtn = document.getElementById("bi-start");
  if (biBtn) biBtn.disabled = !(document.getElementById("bi-video")?.value && document.getElementById("bi-subs")?.value && document.getElementById("bi-output")?.value);
  const anBtn = document.getElementById("an-analyze");
  if (anBtn) anBtn.disabled = !document.getElementById("an-input")?.value;
}

// === Deliver: Batch ===
document.getElementById("del-browse-video")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("del-video").value = d; checkDeliverReady(); }
});
document.getElementById("del-browse-audio")?.addEventListener("click", async () => {
  const f = await open({ directory: false }); if (f) { document.getElementById("del-audio").value = f; checkDeliverReady(); }
});
document.getElementById("del-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("del-output").value = d; checkDeliverReady(); }
});

function checkDeliverReady() {
  const btn = document.getElementById("del-start");
  if (btn) btn.disabled = !(document.getElementById("del-video")?.value && document.getElementById("del-output")?.value);
  const dcpBtn = document.getElementById("dcp-convert");
  if (dcpBtn) dcpBtn.disabled = !(document.getElementById("dcp-imp")?.value && document.getElementById("dcp-output")?.value);
}

// === Deliver: To DCP ===
document.getElementById("dcp-browse-imp")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("dcp-imp").value = d; checkDeliverReady(); }
});
document.getElementById("dcp-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("dcp-output").value = d; checkDeliverReady(); }
});

document.getElementById("dcp-convert")?.addEventListener("click", async () => {
  const imp = document.getElementById("dcp-imp").value;
  const output = document.getElementById("dcp-output").value;
  const title = document.getElementById("dcp-title").value;
  const kind = document.getElementById("dcp-kind").value;
  const box = document.getElementById("dcp-results");
  box.classList.add("visible");
  box.textContent = "Converting IMP to DCP...";
  const args = ["to-dcp", "-i", imp, "-o", output, "-k", kind];
  if (title) args.push("-t", title);
  const cmd = Command.sidecar("imfwizard", args);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ DCP created\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Supplement ===
document.getElementById("sup-browse-ov")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("sup-ov").value = d; checkSupReady(); }
});
document.getElementById("sup-browse-video")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("sup-video").value = d; checkSupReady(); }
});
document.getElementById("sup-browse-audio")?.addEventListener("click", async () => {
  const f = await open({ filters: [{ name: "WAV", extensions: ["wav"] }] }); if (f) { document.getElementById("sup-audio").value = f; checkSupReady(); }
});
document.getElementById("sup-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("sup-output").value = d; checkSupReady(); }
});

function checkSupReady() {
  const btn = document.getElementById("sup-create");
  const hasChange = document.getElementById("sup-video")?.value || document.getElementById("sup-audio")?.value;
  if (btn) btn.disabled = !(document.getElementById("sup-ov")?.value && document.getElementById("sup-title")?.value && document.getElementById("sup-output")?.value && hasChange);
}
document.getElementById("sup-title")?.addEventListener("input", checkSupReady);

document.getElementById("sup-create")?.addEventListener("click", async () => {
  const ov = document.getElementById("sup-ov").value;
  const title = document.getElementById("sup-title").value;
  const video = document.getElementById("sup-video").value;
  const audio = document.getElementById("sup-audio").value;
  const output = document.getElementById("sup-output").value;
  const box = document.getElementById("sup-results");
  box.classList.add("visible");
  box.textContent = "Creating supplemental IMP...";
  const args = ["supplement", "--ov", ov, "-t", title, "-o", output];
  if (video) args.push("--replace", video + "@video");
  if (audio) args.push("--replace", audio + "@audio");
  const cmd = Command.sidecar("imfwizard", args);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ Supplemental IMP created\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
  if (result.code === 0) addRecentProject(output, title);
});

// === Metadata ===
let metaImpDir = null;
document.getElementById("meta-browse")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) {
    metaImpDir = d;
    document.getElementById("meta-fields").style.display = "block";
    setStatus("Loading metadata...");
    const cmd = Command.sidecar("imfwizard", ["info", d]);
    const result = await cmd.execute();
    if (result.code === 0) {
      try {
        const info = JSON.parse(result.stdout);
        document.getElementById("meta-title").value = info.title || "";
        document.getElementById("meta-annotation").value = info.annotation || "";
        document.getElementById("meta-issuer").value = info.issuer || "";
        document.getElementById("meta-save").disabled = false;
        setStatus("Metadata loaded");
      } catch { setStatus("Could not parse metadata"); }
    }
  }
});

document.getElementById("meta-save")?.addEventListener("click", async () => {
  if (!metaImpDir) {
    setStatus("Browse to an IMP first");
    return;
  }
  const title = document.getElementById("meta-title").value;
  const annotation = document.getElementById("meta-annotation").value;
  const issuer = document.getElementById("meta-issuer").value;
  const args = ["metadata-edit", "-i", metaImpDir];
  if (title) args.push("-t", title);
  if (annotation) args.push("-a", annotation);
  if (issuer) args.push("--issuer", issuer);
  setStatus("Saving metadata...");
  const cmd = Command.sidecar("imfwizard", args);
  const result = await cmd.execute();
  setStatus(result.code === 0 ? "✓ Metadata saved" : "✗ Failed: " + (result.stderr || result.stdout));
});

// === Batch Delivery ===
document.getElementById("del-start")?.addEventListener("click", async () => {
  const video = document.getElementById("del-video").value;
  const audio = document.getElementById("del-audio").value;
  const title = document.getElementById("del-title").value || "Untitled";
  const output = document.getElementById("del-output").value;
  const box = document.getElementById("del-results");
  box.classList.add("visible");

  const targets = [...document.querySelectorAll("#del-start")
    .closest("section")
    .querySelectorAll(".checkbox-group input:checked")]
    .map(cb => cb.value);

  if (!targets.length) {
    box.textContent = "Please select at least one delivery target.";
    return;
  }

  box.textContent = `Submitting ${targets.length} delivery job(s)...`;
  const jobIds = [];
  for (const target of targets) {
    try {
      // no dialog here: the panel picks its own source and queues a run per target
      const result = await invoke("submit_job", {
        videoPath: video,
        title: `${title} [${target}]`,
        outputDir: `${output}/${target}`,
        audioPath: audio || null,
        hintsAccepted: true,
        framerate: document.getElementById("prop-framerate")?.value || "24/1",
        contentKind: document.getElementById("prop-content-kind")?.value || "feature",
        bandwidth: parseInt(document.getElementById("prop-bandwidth")?.value) || 250,
      });
      jobIds.push(result.jobId);
    } catch (e) {
      box.textContent += `\n✗ Failed to queue ${target}: ${e}`;
    }
  }
  box.textContent = `✓ Queued ${jobIds.length} delivery job(s): ${targets.join(", ")}`;
  setStatus(`${jobIds.length} delivery jobs queued`);
});

// === Jobs ===
let jobsPollInterval = null;

async function refreshJobs() {
  const badge = document.getElementById("jobs-status");
  try {
    const jobs = await invoke("list_jobs");
    badge.textContent = "Active";
    const tbody = document.getElementById("jobs-tbody");
    if (!jobs || jobs.length === 0) {
      tbody.innerHTML = '<tr><td colspan="5" style="text-align:center">No jobs</td></tr>';
      return;
    }
    tbody.innerHTML = jobs.map(j => {
      return `<tr><td>${j.id}</td><td>${j.title}</td><td>${j.status}</td><td>${j.percent.toFixed(0)}%</td>
        <td>${(j.status === "running" || j.status === "queued") ? `<button class="btn-sm btn-cancel" data-job-id="${j.id}">✕</button>` : ''}</td></tr>`;
    }).join('');
    tbody.querySelectorAll(".btn-cancel").forEach(btn => {
      btn.addEventListener("click", async () => {
        await invoke("cancel_job", { jobId: parseInt(btn.dataset.jobId) });
        refreshJobs();
      });
    });
  } catch { badge.textContent = "Error"; }
}

function startJobsPolling() { if (!jobsPollInterval) jobsPollInterval = setInterval(refreshJobs, 3000); }
function stopJobsPolling() { if (jobsPollInterval) { clearInterval(jobsPollInterval); jobsPollInterval = null; } }
document.getElementById("jobs-refresh")?.addEventListener("click", refreshJobs);

// === Utilities ===
function setStatus(text) {
  const el = document.getElementById("status-text");
  if (el) {
    el.textContent = text;
    el.title = text;
  }
}
function formatTime(secs) { const m = Math.floor(secs / 60); const s = Math.floor(secs % 60); return m > 0 ? `${m}m${s}s` : `${s}s`; }

// === Free disk ===
const DISK_REFRESH_MS = 30000;
const DISK_LOW_PERCENT = 10;

function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) { value /= 1000; unit++; }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

async function refreshDiskSpace() {
  const el = document.getElementById("status-disk");
  if (!el) return;
  const path = document.getElementById("prop-output")?.value || await documentDir();
  let space;
  try {
    space = await invoke("disk_space", { path });
  } catch {
    el.textContent = "";
    el.title = "";
    return;
  }
  const percent = Math.round(space.percent_free);
  el.textContent = `💾 ${percent}%`;
  el.title = `${formatBytes(space.free_bytes)} free of ${formatBytes(space.total_bytes)} on ${path}`;
  el.style.color = percent <= DISK_LOW_PERCENT ? "#ff6b6b" : "";
}

refreshDiskSpace();
setInterval(refreshDiskSpace, DISK_REFRESH_MS);

// === Title sync ===
document.getElementById("prop-title")?.addEventListener("input", (e) => {
  const title = e.target.value.trim();
  document.getElementById("project-name").textContent = title || "Untitled IMP";
  project.title = title;
});

// === Recent Projects ===
const RECENT_KEY = "imfwizard-recent-projects";
const RECENT_COLLAPSED_KEY = "imfwizard-recent-projects-collapsed";
const MAX_RECENT = 20;

function recentProjectsCollapsed() {
  return localStorage.getItem(RECENT_COLLAPSED_KEY) !== "false";
}

function applyRecentProjectsCollapsed() {
  const section = document.getElementById("recent-projects");
  const toggle = document.getElementById("recent-toggle");
  if (!section) return;
  const collapsed = recentProjectsCollapsed();
  section.classList.toggle("collapsed", collapsed);
  if (toggle) {
    toggle.textContent = collapsed ? "▶" : "▼";
    toggle.setAttribute("aria-expanded", String(!collapsed));
  }
}

document.getElementById("recent-header")?.addEventListener("click", () => {
  localStorage.setItem(RECENT_COLLAPSED_KEY, String(!recentProjectsCollapsed()));
  applyRecentProjectsCollapsed();
});

function getRecentProjects() {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY)) || []; }
  catch { return []; }
}

function addRecentProject(path, title) {
  let recent = getRecentProjects().filter(r => r.path !== path);
  recent.unshift({ path, title, time: Date.now() });
  if (recent.length > MAX_RECENT) recent = recent.slice(0, MAX_RECENT);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

function removeRecentProject(path) {
  const recent = getRecentProjects().filter(r => r.path !== path);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

function renderRecentProjects() {
  const section = document.getElementById("recent-projects");
  const list = document.getElementById("recent-list");
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
      addToPlaylist(el.dataset.path);
      setStatus(`Queued: ${el.dataset.path}`);
    });
  });
  list.querySelectorAll('.recent-retitle').forEach(el => {
    el.addEventListener('click', async (event) => {
      event.stopPropagation();
      const dir = el.dataset.path;
      const title = prompt("New content title:", dir.split(/[/\\]/).pop());
      if (!title?.trim()) return;
      const ok = await tauriConfirm(
        `Retitle to ${title}? The CPL gets a new composition id, so any KDM, supplemental IMP or delivery made from the old one no longer matches. A signed package loses its signature.`,
        { title: "Retitle IMP", kind: "warning" },
      );
      if (!ok) return;
      let newPath;
      try {
        newPath = await invoke("retitle_imp", { path: dir, title });
      } catch (e) {
        tauriMessage(String(e), { title: "Retitle failed", kind: "error" });
        return;
      }
      removeRecentProject(dir);
      addRecentProject(newPath, title.trim());
      setStatus(`Retitled to ${title.trim()}`);
    });
  });
  list.querySelectorAll('.recent-delete').forEach(el => {
    el.addEventListener('click', async (event) => {
      event.stopPropagation();
      const dir = el.dataset.path;
      const ok = await tauriConfirm(`Delete ${dir} and everything in it?`, {
        title: "Delete IMP",
        kind: "warning",
      });
      if (!ok) return;
      try {
        await invoke("delete_imp", { path: dir });
      } catch (e) {
        tauriMessage(String(e), { title: "Delete failed", kind: "error" });
        return;
      }
      removeRecentProject(dir);
      setStatus(`Deleted ${dir}`);
      refreshDiskSpace();
    });
  });
  list.querySelectorAll('.recent-item').forEach(el => {
    el.addEventListener('click', () => openImp(el.dataset.path));
  });
}

// === Desktop Notifications ===
function notifyBuildComplete(success, title) {
  if (Notification.permission === "granted") {
    new Notification(success ? "Build Complete" : "Build Failed", {
      body: success ? `"${title}" built successfully` : `"${title}" build failed`,
    });
  } else if (Notification.permission !== "denied") {
    Notification.requestPermission();
  }
}

if ("Notification" in window && Notification.permission === "default") {
  Notification.requestPermission();
}

// === Confirmation Dialogs ===
document.getElementById("btn-new-project")?.addEventListener("click", async () => {
  if (project.assets.length > 0) {
    if (!(await tauriConfirm("Clear current project and start new? Unsaved changes will be lost."))) return;
  }
  project.title = "";
  project.assets = [];
  project.compositions = [
    { id: 1, name: "Main", contentKind: "feature", segments: [{ id: 1, picture: null, sound: null, subtitle: null }] }
  ];
  project.activeComposition = 0;
  nextCplId = 2;
  nextAssetId = 1;
  const titleEl = document.getElementById("prop-title");
  if (titleEl) titleEl.value = "";
  document.getElementById("prop-output") && (document.getElementById("prop-output").value = "");
  document.getElementById("project-name").textContent = "Untitled IMP";
  switchView("project");
  renderAssets();
  renderCplTabs();
  renderSegments();
  updateStatusStats();
  setStatus("New project — enter a title to get started");
  if (titleEl) { titleEl.focus(); titleEl.select(); }
});

// === Status Bar Stats ===
function updateStatusStats() {
  const el = document.getElementById("status-stats");
  if (!el) return;
  const n = project.assets.length;
  const v = project.assets.filter(a => a.type === 'video').length;
  const a = project.assets.filter(a => a.type === 'audio').length;
  if (n === 0) { el.textContent = ""; } else {
    const parts = [];
    if (v) parts.push(`${v} video`);
    if (a) parts.push(`${a} audio`);
    const s = project.assets.filter(a => a.type === 'subtitle').length;
    if (s) parts.push(`${s} sub`);
    el.textContent = `${n} assets (${parts.join(', ')})`;
  }
  updateToolbarState();
}

// === Toolbar Button State ===
let buildInFlight = false;

function beginBuild() {
  buildInFlight = true;
  hidePostBuildActions();
  updateToolbarState();
}

function endBuild() {
  buildInFlight = false;
  updateToolbarState();
  refreshDiskSpace();
}

function updateToolbarState() {
  const hasVideo = project.segments.some(s => s.picture);
  const hasTitle = !!(document.getElementById("prop-title")?.value?.trim());
  const buildBtn = document.getElementById("btn-build");
  const previewBtn = document.getElementById("btn-preview");
  const supBtn = document.getElementById("btn-supplement");
  if (buildBtn) buildBtn.disabled = buildInFlight || !(hasVideo && hasTitle);
  if (previewBtn) previewBtn.disabled = !hasVideo;
  if (supBtn) supBtn.disabled = !hasTitle;
}

// Keep title in sync and update toolbar state
document.getElementById("prop-title")?.addEventListener("input", () => { updateToolbarState(); });

// === Context Menu ===
const ctxMenu = document.getElementById("context-menu");
let ctxAssetId = null;

function showContextMenu(e, assetId) {
  e.preventDefault();
  ctxAssetId = assetId;
  ctxMenu.style.left = e.clientX + "px";
  ctxMenu.style.top = e.clientY + "px";
  ctxMenu.hidden = false;
}

document.addEventListener("click", () => { if (ctxMenu) ctxMenu.hidden = true; });

async function removeAsset(assetId) {
  const asset = project.assets.find(a => a.id === assetId);
  if (!asset) return;
  if (!(await tauriConfirm(`Remove "${asset.name}" from project?`))) return;
  project.assets = project.assets.filter(a => a.id !== assetId);
  project.segments.forEach(s => {
    if (s.picture?.id === assetId) s.picture = null;
    if (s.sound?.id === assetId) s.sound = null;
    if (s.subtitle?.id === assetId) s.subtitle = null;
  });
  renderAssets();
  renderSegments();
  updateStatusStats();
}

ctxMenu?.querySelectorAll("button").forEach(btn => {
  btn.addEventListener("click", () => {
    const action = btn.dataset.action;
    const asset = project.assets.find(a => a.id === ctxAssetId);
    if (!asset) return;
    if (action === "preview") {
      previewProjectFile(asset.path);
    } else if (action === "remove") {
      removeAsset(ctxAssetId);
    } else if (action === "reveal") {
      // reveals the file in the OS file manager (shell open only accepts URLs)
      revealItemInDir(asset.path);
    }
    ctxMenu.hidden = true;
  });
});

// === Progress in Title Bar ===
function setTitleProgress(percent, stage) {
  if (percent >= 0 && percent < 100) {
    document.title = `IMF Wizard — ${stage} ${Math.round(percent)}%`;
  } else {
    document.title = "IMF Wizard";
  }
}

// === Asset Filter ===
document.getElementById("asset-filter")?.addEventListener("input", (e) => {
  const q = e.target.value.toLowerCase();
  document.querySelectorAll("#asset-list .asset-item").forEach(el => {
    const name = el.querySelector(".asset-name")?.textContent?.toLowerCase() || "";
    el.style.display = name.includes(q) ? "" : "none";
  });
});

// === Auto-detect Video Properties (ffprobe) ===
async function probeVideo(path) {
  try {
    const cmd = Command.create("ffprobe", [
      "-v", "quiet", "-print_format", "json",
      "-show_streams", "-show_format", path
    ]);
    const result = await cmd.execute();
    if (result.code !== 0) return null;
    const info = JSON.parse(result.stdout);
    const vs = info.streams?.find(s => s.codec_type === "video");
    if (!vs) return null;
    return {
      width: vs.width,
      height: vs.height,
      fps: vs.r_frame_rate,
      duration: parseFloat(info.format?.duration || vs.duration || "0"),
    };
  } catch { return null; }
}

// === Init ===
renderAssets();
renderSegments();
renderRecentProjects();
updateStatusStats();
initPreview();
initTimeline();
initPlaylist(document.getElementById("playlist"), { loadPackage: previewBuiltPackage });
setStatus("Ready");

// === Target Conversion (Scale/Crop/Letterbox) ===
document.getElementById("convert-browse-input")?.addEventListener("click", async () => {
  const path = await open({ filters: [
    { name: "Video", extensions: ["mp4", "mkv", "mov", "mxf"] },
    { name: "All", extensions: ["*"] }
  ]});
  if (path) {
    document.getElementById("convert-input").value = path;
    document.getElementById("convert-start").disabled = false;
  }
});
document.getElementById("convert-browse-output")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) document.getElementById("convert-output").value = path;
});
document.getElementById("convert-start")?.addEventListener("click", async () => {
  const input = document.getElementById("convert-input").value;
  const output = document.getElementById("convert-output").value;
  const target = document.getElementById("convert-target").value;
  if (!input) return;

  const resultsEl = document.getElementById("convert-results");
  resultsEl.textContent = "Converting...";
  resultsEl.classList.add("visible");
  const args = ["target-convert", "-i", input, "-t", target];
  if (output) args.push("-o", output);
  const cmd = Command.sidecar("imfwizard", args);
  const result = await cmd.execute();
  resultsEl.textContent = result.code === 0 ? "✓ Conversion complete\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});
