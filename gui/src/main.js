import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Command } from "@tauri-apps/plugin-shell";
import { open as _open } from "@tauri-apps/plugin-dialog";
import { documentDir, join } from "@tauri-apps/api/path";
import { initPreview, previewFile, previewDcp } from "./preview.js";
import { initTimeline, loadTimelineFromCpl } from "./timeline.js";

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

document.addEventListener("keydown", (e) => {
  if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT" || e.target.tagName === "TEXTAREA") return;

  const ctrl = e.ctrlKey || e.metaKey;
  const shift = e.shiftKey;

  if (ctrl && e.key === "n") { e.preventDefault(); document.getElementById("btn-new-project")?.click(); }
  else if (ctrl && e.key === "o") { e.preventDefault(); document.getElementById("btn-open-project")?.click(); }
  else if (ctrl && e.key === "b") { e.preventDefault(); document.getElementById("btn-build")?.click(); }
  else if (ctrl && e.key === "p") { e.preventDefault(); document.getElementById("btn-preview")?.click(); }
  else if (ctrl && e.key === "i") { e.preventDefault(); document.getElementById("import-video")?.click(); }
  else if (ctrl && shift && e.key === "S") { e.preventDefault(); document.getElementById("btn-supplement")?.click(); }
  // View switching: Ctrl+1-7
  else if (ctrl && e.key === "1") { e.preventDefault(); switchView("project"); }
  else if (ctrl && e.key === "2") { e.preventDefault(); switchView("timeline"); }
  else if (ctrl && e.key === "3") { e.preventDefault(); switchView("validate"); }
  else if (ctrl && e.key === "4") { e.preventDefault(); switchView("tools"); }
  else if (ctrl && e.key === "5") { e.preventDefault(); switchView("deliver"); }
  else if (ctrl && e.key === "6") { e.preventDefault(); switchView("jobs"); }
  else if (ctrl && e.key === "7") { e.preventDefault(); switchView("settings"); }
  // Theme toggle
  else if (ctrl && shift && e.key === "T") { e.preventDefault(); document.getElementById("theme-toggle")?.click(); }
});

// === Preferences ===
const PREFS_KEY = "imfwizard-preferences";
const PREFS_VERSION = 2;
const PREF_DEFAULTS = {
  profile: "App2e", creator: "", language: "en",
  bandwidth: 250, colourspace: "Rec.709", hdr: "SDR",
  signingCert: "", signingKey: "", outputDir: "",
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
  });
  setStatus("Settings saved");
});

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
    </div>
  `).join('');
  list.querySelectorAll('.asset-item').forEach(el => {
    el.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/plain', el.dataset.assetId); });
    el.addEventListener('contextmenu', (e) => { showContextMenu(e, parseInt(el.dataset.assetId)); });
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

// === Output directory ===
document.getElementById("browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) document.getElementById("prop-output").value = dir;
});

// === Open existing IMP ===
document.getElementById("btn-open-project")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    const name = dir.split(/[/\\]/).pop();
    document.getElementById("project-name").textContent = name;
    project.title = name;
    document.getElementById("prop-title").value = name;
    addRecentProject(dir, name);
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
});

// === Preview ===
document.getElementById("btn-preview")?.addEventListener("click", () => {
  const seg = project.segments[0];
  if (seg?.picture) { previewFile(seg.picture.path); }
  else { alert("Import a video asset first"); }
});

// === Supplement ===
document.getElementById("btn-supplement")?.addEventListener("click", () => {
  switchView("timeline");
  // Scroll the supplement panel into view
  setTimeout(() => {
    const supOv = document.getElementById("sup-ov");
    if (supOv) supOv.scrollIntoView({ behavior: "smooth", block: "start" });
  }, 100);
});

// === Build IMP ===
let currentJobId = null;

document.getElementById("btn-build")?.addEventListener("click", async () => {
  const title = document.getElementById("prop-title")?.value?.trim();
  if (!title) { alert("Enter a content title in Properties"); return; }

  const seg = project.segments[0];
  if (!seg?.picture) { alert("Import a video asset first"); return; }

  const video = seg.picture.path;
  const audio = seg.sound?.path || null;
  let output = document.getElementById("prop-output")?.value;
  if (!output) {
    const docs = await documentDir();
    output = await join(docs, title);
    document.getElementById("prop-output").value = output;
  }

  const progressSection = document.getElementById("progress-section");
  const progressBar = document.getElementById("progress-bar");
  const stageEl = document.getElementById("progress-stage");
  const statsEl = document.getElementById("progress-stats");
  progressSection.style.display = "flex";
  progressBar.value = 0;
  stageEl.textContent = "Queued...";
  statsEl.textContent = "";

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
      unlisten();
    } else if (p.stage === "error") {
      setStatus("Build failed");
      setTitleProgress(-1);
      notifyBuildComplete(false, title);
      unlisten();
    }
  });

  try {
    currentJobId = await invoke("submit_job", {
      videoPath: video, title, outputDir: output, audioPath: audio,
      framerate: document.getElementById("prop-framerate")?.value || "24/1",
      contentKind: document.getElementById("prop-content-kind")?.value || "feature",
      bandwidth: parseInt(document.getElementById("prop-bandwidth")?.value) || 250,
    });
    setStatus("Building IMP...");
  } catch (e) {
    stageEl.textContent = "Failed";
    setStatus("Error: " + e);
    unlisten();
  }
});

document.getElementById("progress-cancel")?.addEventListener("click", async () => {
  if (currentJobId) { await invoke("cancel_job", { jobId: currentJobId }); setStatus("Cancelled"); }
});

// === Validate ===
document.getElementById("val-browse")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("val-path").textContent = dir; document.getElementById("val-run").disabled = false; }
});

document.getElementById("val-run")?.addEventListener("click", async () => {
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
});

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
document.getElementById("sup-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true }); if (d) { document.getElementById("sup-output").value = d; checkSupReady(); }
});

function checkSupReady() {
  const btn = document.getElementById("sup-create");
  if (btn) btn.disabled = !(document.getElementById("sup-ov")?.value && document.getElementById("sup-title")?.value && document.getElementById("sup-output")?.value);
}
document.getElementById("sup-title")?.addEventListener("input", checkSupReady);

document.getElementById("sup-create")?.addEventListener("click", async () => {
  const ov = document.getElementById("sup-ov").value;
  const title = document.getElementById("sup-title").value;
  const video = document.getElementById("sup-video").value;
  const output = document.getElementById("sup-output").value;
  const entryPoint = document.getElementById("sup-entry-point").value;
  const duration = document.getElementById("sup-duration").value;
  const box = document.getElementById("sup-results");
  box.classList.add("visible");
  box.textContent = "Creating supplemental IMP...";
  const args = ["supplement", "--ov", ov, "-t", title, "-o", output];
  if (video) args.push("-v", video);
  if (entryPoint && parseInt(entryPoint) > 0) args.push("--entry-point", entryPoint);
  if (duration) args.push("--duration", duration);
  const cmd = Command.sidecar("imfwizard", args);
  const result = await cmd.execute();
  box.textContent = result.code === 0 ? "✓ Supplemental IMP created\n\n" + result.stdout : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Metadata ===
document.getElementById("meta-browse")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) {
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
  const box = document.getElementById("meta-results") || document.createElement("div");
  box.classList?.add("visible");
  setStatus("Metadata editing is not yet supported by the CLI. This feature is under development.");
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
      const jobId = await invoke("submit_job", {
        videoPath: video,
        title: `${title} [${target}]`,
        outputDir: `${output}/${target}`,
        audioPath: audio || null,
        framerate: document.getElementById("prop-framerate")?.value || "24/1",
        contentKind: document.getElementById("prop-content-kind")?.value || "feature",
        bandwidth: parseInt(document.getElementById("prop-bandwidth")?.value) || 250,
      });
      jobIds.push(jobId);
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
function setStatus(text) { const el = document.getElementById("status-text"); if (el) el.textContent = text; }
function formatTime(secs) { const m = Math.floor(secs / 60); const s = Math.floor(secs % 60); return m > 0 ? `${m}m${s}s` : `${s}s`; }

// === Title sync ===
document.getElementById("prop-title")?.addEventListener("input", (e) => {
  const title = e.target.value.trim();
  document.getElementById("project-name").textContent = title || "Untitled IMP";
  project.title = title;
});

// === Recent Projects ===
const RECENT_KEY = "imfwizard-recent-projects";
const MAX_RECENT = 8;

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

function renderRecentProjects() {
  const section = document.getElementById("recent-projects");
  const list = document.getElementById("recent-list");
  if (!section || !list) return;
  const recent = getRecentProjects();
  if (recent.length === 0) { section.hidden = true; return; }
  section.hidden = false;
  list.innerHTML = recent.map(r => `
    <div class="recent-item" data-path="${r.path}" title="${r.path}">
      <span class="recent-title">${r.title || r.path.split(/[/\\]/).pop()}</span>
      <span class="recent-path">${r.path}</span>
    </div>
  `).join('');
  list.querySelectorAll('.recent-item').forEach(el => {
    el.addEventListener('click', () => {
      const dir = el.dataset.path;
      const name = dir.split(/[/\\]/).pop();
      document.getElementById("project-name").textContent = name;
      project.title = name;
      document.getElementById("prop-title").value = name;
      setStatus(`Opened: ${dir}`);
    });
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
document.getElementById("btn-new-project")?.addEventListener("click", () => {
  if (project.assets.length > 0) {
    if (!confirm("Clear current project and start new? Unsaved changes will be lost.")) return;
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
function updateToolbarState() {
  const hasVideo = project.segments.some(s => s.picture);
  const hasTitle = !!(document.getElementById("prop-title")?.value?.trim());
  const buildBtn = document.getElementById("btn-build");
  const previewBtn = document.getElementById("btn-preview");
  const supBtn = document.getElementById("btn-supplement");
  if (buildBtn) buildBtn.disabled = !(hasVideo && hasTitle);
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

ctxMenu?.querySelectorAll("button").forEach(btn => {
  btn.addEventListener("click", () => {
    const action = btn.dataset.action;
    const asset = project.assets.find(a => a.id === ctxAssetId);
    if (!asset) return;
    if (action === "preview") {
      previewFile(asset.path);
    } else if (action === "remove") {
      if (!confirm(`Remove "${asset.name}" from project?`)) return;
      project.assets = project.assets.filter(a => a.id !== ctxAssetId);
      project.segments.forEach(s => {
        if (s.picture?.id === ctxAssetId) s.picture = null;
        if (s.sound?.id === ctxAssetId) s.sound = null;
        if (s.subtitle?.id === ctxAssetId) s.subtitle = null;
      });
      renderAssets();
      renderSegments();
      updateStatusStats();
    } else if (action === "reveal") {
      invoke("plugin:shell|open", { path: asset.path.replace(/[/\\][^/\\]*$/, '') });
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
