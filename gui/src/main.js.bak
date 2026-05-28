import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as _open, save } from "@tauri-apps/plugin-dialog";
import { Command } from "@tauri-apps/plugin-shell";
import { sendNotification, isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { initTimeline } from "./timeline.js";
import { initPreview } from "./preview.js";

let lastBrowseDir = null;

// Wrapper that remembers last browse location
async function open(opts = {}) {
  const result = await _open({ ...opts, defaultPath: opts.defaultPath || lastBrowseDir || undefined });
  if (result) {
    lastBrowseDir = opts.directory ? result : result.replace(/[/\\][^/\\]*$/, '');
  }
  return result;
}

// === Tab navigation ===
document.querySelectorAll(".nav-tabs button[data-page]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".nav-tabs button[data-page]").forEach(b => b.classList.remove("active"));
    document.querySelectorAll(".page").forEach(p => p.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(btn.dataset.page).classList.add("active");
  });
});

// === Theme toggle ===
const themeBtn = document.getElementById("theme-toggle");
themeBtn?.addEventListener("click", () => {
  document.body.classList.toggle("light-theme");
  themeBtn.textContent = document.body.classList.contains("light-theme") ? "☀️" : "🌙";
  localStorage.setItem("theme", document.body.classList.contains("light-theme") ? "light" : "dark");
});
if (localStorage.getItem("theme") === "light") {
  document.body.classList.add("light-theme");
  if (themeBtn) themeBtn.textContent = "☀️";
}

// === Notifications ===
async function notify(title, body) {
  let granted = await isPermissionGranted();
  if (!granted) {
    const perm = await requestPermission();
    granted = perm === "granted";
  }
  if (granted) {
    sendNotification({ title, body });
  }
}

// === Recent Projects ===
function getRecentProjects() {
  try {
    return JSON.parse(localStorage.getItem("recent_projects") || "[]");
  } catch { return []; }
}

function addRecentProject(path, title) {
  let projects = getRecentProjects();
  projects = projects.filter(p => p.path !== path);
  projects.unshift({ path, title, date: new Date().toISOString() });
  if (projects.length > 8) projects = projects.slice(0, 8);
  localStorage.setItem("recent_projects", JSON.stringify(projects));
  renderRecentProjects();
}

function renderRecentProjects() {
  const projects = getRecentProjects();
  const container = document.getElementById("recent-projects");
  const list = document.getElementById("recent-list");
  if (!container || !list) return;

  if (projects.length === 0) {
    container.style.display = "none";
    return;
  }

  container.style.display = "flex";
  list.innerHTML = projects.map(p => {
    const shortPath = p.path.split("/").slice(-2).join("/");
    return `<button class="recent-item" data-path="${p.path}" title="${p.path}">${p.title || shortPath}</button>`;
  }).join("");

  list.querySelectorAll(".recent-item").forEach(btn => {
    btn.addEventListener("click", () => {
      const path = btn.dataset.path;
      document.querySelector('[data-page="validate-page"]').click();
      document.getElementById("val-path").textContent = path;
      valDir = path;
      checkFormReady();
    });
  });
}

renderRecentProjects();

// === Presets ===
const PRESETS = {
  custom: {},
  netflix: { fps: "24/1", colorSpace: "bt2020-pq", bitrate: 250 },
  disney: { fps: "24/1", colorSpace: "bt2020-pq", bitrate: 250 },
  amazon: { fps: "24/1", colorSpace: "bt2020-pq", bitrate: 200 },
  apple: { fps: "24/1", colorSpace: "p3-d65-pq", bitrate: 250 },
  cinema2k: { fps: "24/1", colorSpace: "bt709", bitrate: 250 },
  cinema4k: { fps: "24/1", colorSpace: "bt709", bitrate: 500 },
  broadcast: { fps: "25/1", colorSpace: "bt709", bitrate: 150 },
  archival: { fps: "24/1", colorSpace: "bt709", bitrate: 500 },
};

document.getElementById("preset")?.addEventListener("change", (e) => {
  const preset = PRESETS[e.target.value];
  if (!preset || !Object.keys(preset).length) return;
  if (preset.fps) document.getElementById("fps").value = preset.fps;
  if (preset.colorSpace) document.getElementById("color-space").value = preset.colorSpace;
  if (preset.bitrate) document.getElementById("encode-bitrate").value = preset.bitrate;
});

// === Initialize timeline & preview ===
initTimeline();
initPreview();

// === Form validation — disable buttons until required fields are filled ===
const formRules = {
  "create-btn": () => videoDir && outputDir && document.getElementById("title")?.value?.trim(),
  "sup-create": () => supOvDir && supVideoDir && supOutputDir,
  "loud-measure": () => loudFile,
  "meta-save": () => metaDir,
  "tc-start": () => tcInput && tcOutput,
  "val-start": () => valDir,
  "bi-start": () => biVideoPath && biSubsPath && biOutputPath,
  "an-analyze": () => anPath,
  "dcp-convert": () => dcpImpPath && dcpOutputPath,
  "del-start": () => delVideoPath && delOutputPath && document.getElementById("del-title")?.value?.trim(),
  "asset-scan": () => assetDir,
};

function checkFormReady() {
  for (const [btnId, check] of Object.entries(formRules)) {
    const btn = document.getElementById(btnId);
    if (btn) {
      btn.disabled = !check();
    }
  }
}

// Run validation on any input change
document.addEventListener("input", checkFormReady);
// Initial state — disable all buttons
setTimeout(checkFormReady, 0);

// === Drag and Drop ===
const dropOverlay = document.getElementById("drop-overlay");

document.addEventListener("dragover", (e) => {
  e.preventDefault();
  if (dropOverlay) dropOverlay.hidden = false;
});

document.addEventListener("dragleave", (e) => {
  if (e.relatedTarget === null && dropOverlay) dropOverlay.hidden = true;
});

document.addEventListener("drop", async (e) => {
  e.preventDefault();
  if (dropOverlay) dropOverlay.hidden = true;

  const files = Array.from(e.dataTransfer?.files || []);
  if (files.length === 0) return;

  const first = files[0];
  const ext = first.name.split(".").pop().toLowerCase();

  if (["wav"].includes(ext)) {
    audioFile = first.path || first.name;
    document.getElementById("audio-path").textContent = audioFile;
  } else if (["xml", "ttml"].includes(ext)) {
    subtitleFile = first.path || first.name;
    document.getElementById("subtitle-path").textContent = subtitleFile;
  } else {
    const path = first.path || first.name;
    videoDir = path;
    document.getElementById("video-path").textContent = path;
  }
  checkFormReady();
});

// === Create OV IMP ===
let videoDir = "";
let audioFile = "";
let subtitleFile = "";
let outputDir = "";

function pathSet(id) {
  const el = document.getElementById(id);
  if (!el) return false;
  const t = el.textContent;
  return t && !t.startsWith("No ");
}

document.getElementById("browse-video")?.addEventListener("click", async () => {
  const path = await open({
    directory: false,
    multiple: false,
    filters: [
      { name: 'Video', extensions: ['mp4', 'mkv', 'mov', 'avi', 'mxf', 'webm'] },
      { name: 'All Files', extensions: ['*'] }
    ]
  });
  if (path) {
    videoDir = path;
    document.getElementById("video-path").textContent = path;
    checkFormReady();
  }
});

document.getElementById("browse-video-folder")?.addEventListener("click", async () => {
  const dir = await open({ directory: true, title: "Select Image Sequence / J2K Directory" });
  if (dir) {
    videoDir = dir;
    document.getElementById("video-path").textContent = dir;
    checkFormReady();
  }
});

document.getElementById("browse-audio")?.addEventListener("click", async () => {
  const selected = await open({
    filters: [{ name: "WAV Audio", extensions: ["wav"] }],
    title: "Select Audio File",
  });
  if (selected) {
    audioFile = selected;
    document.getElementById("audio-path").textContent = selected;
    checkFormReady();
  }
});

document.getElementById("select-subtitle")?.addEventListener("click", async () => {
  const selected = await open({
    filters: [{ name: "TTML/IMSC", extensions: ["xml", "ttml"] }],
    title: "Select Subtitle File",
  });
  if (selected) {
    subtitleFile = selected;
    document.getElementById("subtitle-path").textContent = selected;
    checkFormReady();
  }
});

document.getElementById("browse-output")?.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select Output Directory" });
  if (selected) {
    outputDir = selected;
    document.getElementById("output-path").textContent = selected;
    checkFormReady();
  }
});

document.getElementById("create-form").addEventListener("submit", async (e) => {
  e.preventDefault();

  const title = document.getElementById("title").value.trim();
  if (!videoDir || !outputDir) {
    alert("Please select video/image input and output directory.");
    return;
  }

  const progressDiv = document.getElementById("pipeline-progress");
  const progressBar = document.getElementById("pipeline-bar");
  const stageEl = document.getElementById("pipeline-stage");
  const statsEl = document.getElementById("pipeline-stats");
  const msgEl = document.getElementById("pipeline-message");

  progressDiv.style.display = "block";
  progressBar.value = 0;
  stageEl.textContent = "Queued...";
  statsEl.textContent = "";
  msgEl.textContent = "";

  const btn = document.getElementById("create-btn");
  btn.disabled = true;
  btn.textContent = "Creating...";

  const unlisten = await listen("pipeline-progress", (event) => {
    const p = event.payload;
    if (currentJobId && p.job_id !== currentJobId) return;

    progressBar.value = p.percent;
    stageEl.textContent = p.stage.charAt(0).toUpperCase() + p.stage.slice(1);
    msgEl.textContent = p.message;

    const elapsed = formatTime(p.elapsed_secs);
    let remaining = "";
    if (p.percent > 0 && p.percent < 100) {
      const eta = (p.elapsed_secs / p.percent) * (100 - p.percent);
      remaining = ` | ETA: ${formatTime(eta)}`;
    }
    const fpsStr = p.fps > 0 ? ` | ${p.fps.toFixed(1)} fps` : "";
    statsEl.textContent = `${elapsed}${fpsStr}${remaining}`;

    if (p.stage === "done") {
      btn.disabled = false;
      btn.textContent = "Create IMP";
      notify("IMF Wizard", `IMP "${title}" created successfully`);
      unlisten();
    } else if (p.stage === "error") {
      btn.disabled = false;
      btn.textContent = "Create IMP";
      notify("IMF Wizard", "IMP creation failed");
      unlisten();
    }
  });

  try {
    currentJobId = await invoke("submit_job", {
      videoPath: videoDir,
      title: title,
      outputDir: outputDir,
      audioPath: audioFile || null,
    });
  } catch (err) {
    msgEl.textContent = "Error: " + err;
    stageEl.textContent = "Failed";
    btn.disabled = false;
    btn.textContent = "Create IMP";
    unlisten();
  }
});

let currentJobId = null;
let paused = false;

// Pause / Resume
document.getElementById("pipeline-pause")?.addEventListener("click", async () => {
  if (paused) {
    await invoke("resume_job");
    paused = false;
    document.getElementById("pipeline-pause").textContent = "⏸";
  } else {
    await invoke("pause_job");
    paused = true;
    document.getElementById("pipeline-pause").textContent = "▶";
    document.getElementById("pipeline-stage").textContent += " (paused)";
  }
});

// Cancel
document.getElementById("pipeline-cancel")?.addEventListener("click", async () => {
  if (currentJobId) {
    await invoke("cancel_job", { jobId: currentJobId });
  }
});

function formatTime(secs) {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

// === Supplemental IMP ===
let supOvDir = "", supVideoDir = "", supAudioFile = "", supOutputDir = "";

document.getElementById("sup-select-ov")?.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select OV IMP Directory" });
  if (selected) { supOvDir = selected; document.getElementById("sup-ov-path").textContent = selected; checkFormReady(); }
});

document.getElementById("sup-select-video")?.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select Replacement Video Directory" });
  if (selected) { supVideoDir = selected; document.getElementById("sup-video-path").textContent = selected; checkFormReady(); }
});

document.getElementById("sup-select-audio")?.addEventListener("click", async () => {
  const selected = await open({ filters: [{ name: "WAV", extensions: ["wav"] }] });
  if (selected) { supAudioFile = selected; document.getElementById("sup-audio-path").textContent = selected; checkFormReady(); }
});

document.getElementById("sup-select-output")?.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select Output Directory" });
  if (selected) { supOutputDir = selected; document.getElementById("sup-output-path").textContent = selected; checkFormReady(); }
});

document.getElementById("sup-create")?.addEventListener("click", async () => {
  if (!supOvDir || !supVideoDir || !supOutputDir) {
    alert("Please select OV directory, replacement video, and output directory.");
    return;
  }

  const log = document.getElementById("sup-log");
  log.style.display = "block";
  log.textContent = "Creating supplemental IMP...\n";

  const title = document.getElementById("sup-title").value || "Supplemental";
  const entryPoint = document.getElementById("sup-entry-point").value;
  const duration = document.getElementById("sup-duration").value;

  const args = [
    "supplement",
    "--title", title,
    "--ov", supOvDir,
    "--video", supVideoDir,
    "--output", supOutputDir,
    "--entry-point", entryPoint,
    "--duration", duration,
  ];
  if (supAudioFile) args.push("--audio", supAudioFile);

  const result = await Command.sidecar("imfwizard", args).execute();
  if (result.code === 0) {
    log.textContent += result.stdout + "\nDone!";
    addRecentProject(supOutputDir, title);
    notify("IMF Wizard", `Supplemental IMP "${title}" created`);
  } else {
    log.textContent += "ERROR:\n" + result.stderr;
  }
});

// === Transcode page ===
let tcInput = "", tcOutput = "";

document.getElementById("tc-select-input").addEventListener("click", async () => {
  const selected = await open({ title: "Select Video File" });
  if (selected) { tcInput = selected; document.getElementById("tc-input-path").textContent = selected; checkFormReady(); }
});

document.getElementById("tc-select-output").addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select Output Directory" });
  if (selected) { tcOutput = selected; document.getElementById("tc-output-path").textContent = selected; checkFormReady(); }
});

document.getElementById("tc-start").addEventListener("click", async () => {
  if (!tcInput || !tcOutput) { alert("Select input and output."); return; }
  const log = document.getElementById("tc-log");
  log.style.display = "block";
  log.textContent = "Transcoding...\n";

  const format = document.getElementById("tc-format").value;
  const bitdepth = document.getElementById("tc-bitdepth").value;

  const command = Command.sidecar("imfwizard", [
    "transcode", "-i", tcInput, "-o", tcOutput, "-f", format, "--bit-depth", bitdepth
  ]);
  const output = await command.execute();
  if (output.code === 0) {
    log.textContent += output.stdout + "\nDone!";
    notify("IMF Wizard", "Transcode complete");
  } else {
    log.textContent += "ERROR:\n" + output.stderr;
  }
});

document.getElementById("tc-queue")?.addEventListener("click", async () => {
  if (!tcInput || !tcOutput) { alert("Select input and output."); return; }
  const format = document.getElementById("tc-format").value;
  const bitdepth = document.getElementById("tc-bitdepth").value;
  const desc = `Transcode ${tcInput.split("/").pop()} \u2192 ${format}`;
  const args = ["batch", "add", "-T", "transcode", "-d", desc, "--",
    "transcode", "-i", tcInput, "-o", tcOutput, "-f", format, "--bit-depth", bitdepth];
  const result = await Command.sidecar("imfwizard", args).execute();
  if (result.code === 0) {
    alert("Job submitted to queue!");
  } else {
    alert("Failed: " + (result.stderr || "Daemon not running"));
  }
});

// === Loudness page ===
let loudFile = "";

document.getElementById("loud-select")?.addEventListener("click", async () => {
  const selected = await open({ filters: [{ name: "Audio", extensions: ["wav", "mp3", "aac", "flac", "mxf"] }] });
  if (selected) { loudFile = selected; document.getElementById("loud-path").textContent = selected; checkFormReady(); }
});

document.getElementById("loud-measure")?.addEventListener("click", async () => {
  if (!loudFile) { alert("Select an audio file."); return; }

  const btn = document.getElementById("loud-measure");
  btn.disabled = true;
  btn.textContent = "Measuring...";

  const result = await Command.sidecar("imfwizard", ["loudness", loudFile]).execute();
  btn.disabled = false;
  btn.textContent = "Measure Loudness";

  const panel = document.getElementById("loud-results");
  panel.style.display = "block";

  if (result.code === 0) {
    const output = result.stdout;
    const intMatch = output.match(/Integrated:\s*([-\d.]+)/);
    const rangeMatch = output.match(/Range:\s*([-\d.]+)/);
    const peakMatch = output.match(/Peak:\s*([-\d.]+)/);
    const r128Match = output.match(/R128:\s*(\w+)/);
    const atscMatch = output.match(/ATSC:\s*(\w+)/);

    document.getElementById("loud-integrated").textContent = intMatch ? `${intMatch[1]} LUFS` : "\u2014";
    document.getElementById("loud-range").textContent = rangeMatch ? `${rangeMatch[1]} LU` : "\u2014";
    document.getElementById("loud-peak").textContent = peakMatch ? `${peakMatch[1]} dBTP` : "\u2014";

    const r128El = document.getElementById("loud-r128");
    r128El.textContent = r128Match ? r128Match[1] : "\u2014";
    r128El.className = "compliance-badge " + (r128Match?.[1] === "PASS" ? "pass" : "fail");

    const atscEl = document.getElementById("loud-atsc");
    atscEl.textContent = atscMatch ? atscMatch[1] : "\u2014";
    atscEl.className = "compliance-badge " + (atscMatch?.[1] === "PASS" ? "pass" : "fail");
  } else {
    document.getElementById("loud-integrated").textContent = "Error: " + result.stderr;
  }
});

// === Metadata Editor ===
let metaDir = "";

document.getElementById("meta-select")?.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select IMP Directory" });
  if (selected) { metaDir = selected; document.getElementById("meta-path").textContent = selected; checkFormReady(); }
});

document.getElementById("meta-load")?.addEventListener("click", async () => {
  if (!metaDir) { alert("Select an IMP directory."); return; }

  const result = await Command.sidecar("imfwizard", ["info", metaDir]).execute();
  if (result.code === 0) {
    document.getElementById("meta-fields").style.display = "block";
    const output = result.stdout;
    const titleMatch = output.match(/Title:\s*(.+)/);
    const issuerMatch = output.match(/Issuer:\s*(.+)/);
    const annotMatch = output.match(/Annotation:\s*(.+)/);

    if (titleMatch) document.getElementById("meta-title").value = titleMatch[1].trim();
    if (issuerMatch) document.getElementById("meta-issuer").value = issuerMatch[1].trim();
    if (annotMatch) document.getElementById("meta-annotation").value = annotMatch[1].trim();
    document.getElementById("meta-tracks").textContent = output;
  } else {
    alert("Failed to load IMP: " + result.stderr);
  }
});

document.getElementById("meta-save")?.addEventListener("click", async () => {
  if (!metaDir) return;

  const title = document.getElementById("meta-title").value;
  const annotation = document.getElementById("meta-annotation").value;
  const issuer = document.getElementById("meta-issuer").value;
  const contentKind = document.getElementById("meta-content-kind").value;
  const versionLabel = document.getElementById("meta-version-label").value;
  const locale = document.getElementById("meta-locale").value;

  const args = ["metadata", "--imp", metaDir];
  if (title) args.push("--title", title);
  if (annotation) args.push("--annotation", annotation);
  if (issuer) args.push("--issuer", issuer);
  if (contentKind) args.push("--content-kind", contentKind);
  if (versionLabel) args.push("--version-label", versionLabel);
  if (locale) args.push("--locale", locale);

  const result = await Command.sidecar("imfwizard", args).execute();
  if (result.code === 0) {
    alert("Metadata saved successfully!");
    notify("IMF Wizard", "Metadata updated");
  } else {
    alert("Failed: " + result.stderr);
  }
});

// === Validate page ===
let valDir = "";

document.getElementById("val-select").addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select IMP Directory" });
  if (selected) { valDir = selected; document.getElementById("val-path").textContent = selected; checkFormReady(); }
});

document.getElementById("val-start").addEventListener("click", async () => {
  if (!valDir) { alert("Select an IMP directory."); return; }
  const log = document.getElementById("val-log");
  log.style.display = "block";
  log.textContent = "Validating...\n";

  const command = Command.sidecar("imfwizard", ["validate", valDir]);
  const output = await command.execute();
  log.textContent += output.stdout + (output.stderr || "");
  notify("IMF Wizard", output.code === 0 ? "Validation passed" : "Validation found issues");
});

// === Jobs page ===
let jobsPollInterval = null;

async function refreshJobs() {
  const statusBadge = document.getElementById("jobs-daemon-status");
  try {
    const ping = Command.sidecar("imfwizard", ["batch", "list"]);
    const result = await ping.execute();
    if (result.code !== 0) {
      statusBadge.textContent = "Daemon offline";
      statusBadge.className = "status-badge offline";
      document.getElementById("jobs-tbody").innerHTML =
        '<tr><td colspan="6" style="text-align:center">Daemon not running</td></tr>';
      return;
    }
    statusBadge.textContent = "Daemon online";
    statusBadge.className = "status-badge online";

    const lines = result.stdout.trim().split("\n");
    const tbody = document.getElementById("jobs-tbody");

    if (lines.length <= 1 || lines[0].startsWith("No jobs")) {
      tbody.innerHTML = '<tr><td colspan="6" style="text-align:center">No jobs in queue</td></tr>';
      return;
    }

    const jobLines = lines.slice(2).filter(l => l.trim());
    tbody.innerHTML = jobLines.map(line => {
      const parts = line.trim().split(/\s+/);
      const id = parts[0];
      const state = parts[1];
      const progress = parts[2];
      const type = parts[3];
      const desc = parts.slice(4).join(" ");
      const cancelBtn = (state === "queued" || state === "running")
        ? `<button class="btn-cancel" data-job-id="${id}">Cancel</button>`
        : "";
      const stateClass = `state-${state}`;
      return `<tr>
        <td>${id}</td>
        <td>${type}</td>
        <td>${desc}</td>
        <td><span class="${stateClass}">${state}</span></td>
        <td>${progress}</td>
        <td>${cancelBtn}</td>
      </tr>`;
    }).join("");

    tbody.querySelectorAll(".btn-cancel").forEach(btn => {
      btn.addEventListener("click", async () => {
        const jobId = btn.dataset.jobId;
        await Command.sidecar("imfwizard", ["batch", "cancel", jobId]).execute();
        refreshJobs();
      });
    });
  } catch (err) {
    statusBadge.textContent = "Error";
    statusBadge.className = "status-badge offline";
    document.getElementById("jobs-tbody").innerHTML =
      `<tr><td colspan="6" style="text-align:center;color:var(--accent)">${err}</td></tr>`;
  }
}

document.getElementById("jobs-refresh")?.addEventListener("click", refreshJobs);

document.getElementById("jobs-start-daemon")?.addEventListener("click", async () => {
  Command.sidecar("imfwizard", ["daemon"]).spawn();
  await new Promise(r => setTimeout(r, 1500));
  refreshJobs();
});

document.getElementById("job-submit")?.addEventListener("click", async () => {
  const type = document.getElementById("job-type").value;
  const desc = document.getElementById("job-desc").value;
  const argsStr = document.getElementById("job-args").value;
  if (!desc) { alert("Enter a description"); return; }
  if (!argsStr) { alert("Enter arguments"); return; }

  const args = ["batch", "add", "-T", type, "-d", desc, "--"].concat(argsStr.split(/\s+/));
  const result = await Command.sidecar("imfwizard", args).execute();
  if (result.code === 0) {
    document.getElementById("job-desc").value = "";
    document.getElementById("job-args").value = "";
    refreshJobs();
  } else {
    alert("Failed to submit: " + result.stderr);
  }
});

// Auto-refresh jobs when tab is active
document.querySelectorAll(".nav-tabs button[data-page]").forEach(btn => {
  btn.addEventListener("click", () => {
    if (btn.dataset.page === "jobs-page") {
      refreshJobs();
      if (!jobsPollInterval) {
        jobsPollInterval = setInterval(refreshJobs, 3000);
      }
    } else {
      if (jobsPollInterval) { clearInterval(jobsPollInterval); jobsPollInterval = null; }
    }
  });
});

// ========== BURN-IN ==========
let biVideoPath = "", biSubsPath = "", biOutputPath = "";

document.getElementById("bi-select-video")?.addEventListener("click", async () => {
  const f = await open({ filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "mxf"] }] });
  if (f) { biVideoPath = f; document.getElementById("bi-video-path").textContent = f; checkFormReady(); }
});
document.getElementById("bi-select-subs")?.addEventListener("click", async () => {
  const f = await open({ filters: [{ name: "Subtitles", extensions: ["srt", "ttml", "xml", "scc"] }] });
  if (f) { biSubsPath = f; document.getElementById("bi-subs-path").textContent = f; checkFormReady(); }
});
document.getElementById("bi-select-output")?.addEventListener("click", async () => {
  const f = await save({ filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv"] }] });
  if (f) { biOutputPath = f; document.getElementById("bi-output-path").textContent = f; checkFormReady(); }
});
document.getElementById("bi-start")?.addEventListener("click", async () => {
  if (!biVideoPath || !biSubsPath) { alert("Select video and subtitle files"); return; }
  const log = document.getElementById("bi-log");
  log.style.display = "block";
  log.textContent = "Burning in subtitles...";
  const args = ["burn-in", "-i", biVideoPath, "-s", biSubsPath];
  if (biOutputPath) args.push("-o", biOutputPath);
  const font = document.getElementById("bi-font").value;
  const fontSize = document.getElementById("bi-fontsize").value;
  if (font) args.push("--font", font);
  if (fontSize) args.push("--font-size", fontSize);
  const r = await Command.sidecar("imfwizard", args).execute();
  log.textContent = r.code === 0 ? r.stdout : "Error: " + r.stderr;
  if (r.code === 0) notify("Burn-in complete");
});

// ========== ANALYTICS ==========
let anPath = "";

document.getElementById("an-select")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { anPath = d; document.getElementById("an-path").textContent = d; checkFormReady(); }
});

document.getElementById("an-analyze")?.addEventListener("click", async () => {
  if (!anPath) { alert("Select a J2K directory"); return; }
  const fpsVal = document.getElementById("an-fps").value;
  const [fpsNum, fpsDen] = fpsVal.split("/");
  const args = ["analytics", "-d", anPath, "--fps-num", fpsNum, "--fps-den", fpsDen, "--json"];
  const r = await Command.sidecar("imfwizard", args).execute();
  if (r.code !== 0) { alert("Analytics failed: " + r.stderr); return; }

  const data = JSON.parse(r.stdout);
  document.getElementById("an-results").style.display = "block";
  document.getElementById("an-frames").textContent = data.total_frames;
  document.getElementById("an-duration").textContent = data.duration_seconds.toFixed(1) + "s";
  document.getElementById("an-size").textContent = (data.total_bytes / (1024 * 1024)).toFixed(1) + " MB";
  document.getElementById("an-avg").textContent = data.avg_bitrate_mbps.toFixed(2) + " Mbps";
  document.getElementById("an-peak").textContent = data.peak_bitrate_mbps.toFixed(2) + " Mbps";
  document.getElementById("an-min").textContent = data.min_bitrate_mbps.toFixed(2) + " Mbps";
  document.getElementById("an-stddev").textContent = data.stddev_bitrate_mbps.toFixed(2) + " Mbps";

  // Draw per-second bitrate chart
  drawChart("an-chart", data.per_second_bitrate, data.avg_bitrate_mbps);
  // Draw histogram
  drawHistogram("an-histogram", data.histogram, data.histogram_min, data.histogram_max);
});

function drawChart(canvasId, values, avg) {
  const canvas = document.getElementById(canvasId);
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  if (!values || values.length === 0) return;

  const max = Math.max(...values) * 1.1;
  const barW = w / values.length;

  // Draw average line
  const avgY = h - (avg / max) * h;
  ctx.strokeStyle = "#fbbf24";
  ctx.lineWidth = 1;
  ctx.setLineDash([4, 4]);
  ctx.beginPath(); ctx.moveTo(0, avgY); ctx.lineTo(w, avgY); ctx.stroke();
  ctx.setLineDash([]);

  // Draw bars
  ctx.fillStyle = "#a78bfa";
  for (let i = 0; i < values.length; i++) {
    const barH = (values[i] / max) * h;
    ctx.fillRect(i * barW, h - barH, Math.max(barW - 1, 1), barH);
  }
}

function drawHistogram(canvasId, buckets, min, max) {
  const canvas = document.getElementById(canvasId);
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  if (!buckets || buckets.length === 0) return;

  const peak = Math.max(...buckets);
  const barW = w / buckets.length;
  ctx.fillStyle = "#38bdf8";
  for (let i = 0; i < buckets.length; i++) {
    const barH = (buckets[i] / peak) * (h - 20);
    ctx.fillRect(i * barW + 2, h - barH - 20, barW - 4, barH);
  }
  // Labels
  ctx.fillStyle = "#a1a1aa";
  ctx.font = "11px Inter, sans-serif";
  ctx.textAlign = "center";
  for (let i = 0; i < buckets.length; i++) {
    const val = min + (max - min) * (i + 0.5) / buckets.length;
    ctx.fillText(val.toFixed(0), i * barW + barW / 2, h - 4);
  }
}

// ========== DELIVER ==========
let delVideoPath = "", delAudioPath = "", delOutputPath = "";

document.getElementById("del-select-video")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { delVideoPath = d; document.getElementById("del-video-path").textContent = d; checkFormReady(); }
});
document.getElementById("del-select-audio")?.addEventListener("click", async () => {
  const f = await open({ filters: [{ name: "Audio", extensions: ["wav"] }] });
  if (f) { delAudioPath = f; document.getElementById("del-audio-path").textContent = f; checkFormReady(); }
});
document.getElementById("del-select-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { delOutputPath = d; document.getElementById("del-output-path").textContent = d; checkFormReady(); }
});
document.getElementById("del-start")?.addEventListener("click", async () => {
  const title = document.getElementById("del-title").value;
  if (!delVideoPath || !title || !delOutputPath) { alert("Fill video, title, and output"); return; }
  const checkboxes = document.querySelectorAll("#deliver-page .checkbox-group input:checked, #todcp-page ~ * .checkbox-group input:checked, .page#deliver-page .checkbox-group input:checked");
  const targets = [];
  document.querySelectorAll("#deliver-page .checkbox-group input:checked").forEach(cb => targets.push(cb.value));
  if (targets.length === 0) { alert("Select at least one delivery target"); return; }

  const log = document.getElementById("del-log");
  log.style.display = "block";
  log.textContent = "Starting batch delivery to: " + targets.join(", ") + "...\n";

  const args = ["deliver", "-v", delVideoPath, "-t", title, "-o", delOutputPath, "--targets"];
  args.push(...targets);
  if (delAudioPath) args.push("-a", delAudioPath);

  const r = await Command.sidecar("imfwizard", args).execute();
  log.textContent += r.code === 0 ? r.stdout : "Error: " + r.stderr;
  if (r.code === 0) notify("Batch delivery complete");
});

// ========== TO-DCP ==========
let dcpImpPath = "", dcpOutputPath = "";

document.getElementById("dcp-select-imp")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { dcpImpPath = d; document.getElementById("dcp-imp-path").textContent = d; checkFormReady(); }
});
document.getElementById("dcp-select-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { dcpOutputPath = d; document.getElementById("dcp-output-path").textContent = d; checkFormReady(); }
});
document.getElementById("dcp-convert")?.addEventListener("click", async () => {
  if (!dcpImpPath || !dcpOutputPath) { alert("Select IMP and output directories"); return; }
  const log = document.getElementById("dcp-log");
  log.style.display = "block";
  log.textContent = "Converting IMF to DCP...";
  const args = ["to-dcp", "-i", dcpImpPath, "-o", dcpOutputPath];
  const title = document.getElementById("dcp-title").value;
  const kind = document.getElementById("dcp-kind").value;
  if (title) args.push("-t", title);
  if (kind) args.push("-k", kind);
  const r = await Command.sidecar("imfwizard", args).execute();
  log.textContent = r.code === 0 ? r.stdout : "Error: " + r.stderr;
  if (r.code === 0) notify("DCP conversion complete");
});

// ========== KEYBOARD SHORTCUTS ==========
const shortcutsModal = document.getElementById("shortcuts-modal");
document.getElementById("shortcuts-btn")?.addEventListener("click", () => {
  shortcutsModal.hidden = !shortcutsModal.hidden;
});
document.getElementById("shortcuts-close")?.addEventListener("click", () => {
  shortcutsModal.hidden = true;
});

document.addEventListener("keydown", (e) => {
  // Close modal on Escape
  if (e.key === "Escape" && !shortcutsModal.hidden) {
    shortcutsModal.hidden = true;
    return;
  }

  // Don't trigger shortcuts when typing in inputs
  if (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA" || e.target.tagName === "SELECT") return;

  // ? = show shortcuts
  if (e.key === "?") {
    shortcutsModal.hidden = false;
    return;
  }

  // Number keys = switch tabs
  const tabs = document.querySelectorAll(".nav-tabs button[data-page]");
  const num = parseInt(e.key);
  if (num >= 1 && num <= tabs.length) {
    tabs[num - 1].click();
    return;
  }

  // Ctrl+N = New IMP (switch to create tab)
  if (e.ctrlKey && e.key === "n") {
    e.preventDefault();
    document.querySelector('[data-page="create-page"]').click();
  }

  // Ctrl+O = Open IMP (switch to validate tab + browse)
  if (e.ctrlKey && e.key === "o") {
    e.preventDefault();
    document.querySelector('[data-page="validate-page"]').click();
    document.getElementById("val-select")?.click();
  }

  // Ctrl+J = View jobs
  if (e.ctrlKey && e.key === "j") {
    e.preventDefault();
    document.querySelector('[data-page="jobs-page"]').click();
  }
});

// ========== ASSET BROWSER ==========
let assetDir = "";

document.getElementById("asset-select-dir")?.addEventListener("click", async () => {
  const d = await open({ directory: true, title: "Select IMP Directory" });
  if (d) { assetDir = d; document.getElementById("asset-dir-path").textContent = d; checkFormReady(); }
});

document.getElementById("asset-scan")?.addEventListener("click", async () => {
  if (!assetDir) { alert("Select an IMP directory"); return; }

  const grid = document.getElementById("asset-grid");
  grid.style.display = "grid";
  grid.innerHTML = '<div class="asset-card-meta">Scanning...</div>';

  // Use imfwizard info to get asset listing
  const r = await Command.sidecar("imfwizard", ["info", assetDir]).execute();
  if (r.code !== 0) {
    grid.innerHTML = '<div class="asset-card-meta">Error: ' + r.stderr + '</div>';
    return;
  }

  // Parse track lines from info output
  const lines = r.stdout.split("\n");
  const assets = [];
  for (const line of lines) {
    const match = line.match(/Track:\s+(\S+)\s+\((\w+)\)\s+(.+)/);
    if (match) {
      assets.push({ id: match[1], type: match[2], detail: match[3] });
    }
    // Also match MXF file lines
    const mxfMatch = line.match(/^\s*(\S+\.mxf)/i);
    if (mxfMatch && !assets.find(a => a.id === mxfMatch[1])) {
      assets.push({ id: mxfMatch[1], type: "mxf", detail: mxfMatch[1] });
    }
  }

  if (assets.length === 0) {
    grid.innerHTML = '<div class="asset-card-meta">No assets found. Make sure directory contains ASSETMAP.xml</div>';
    return;
  }

  grid.innerHTML = assets.map((asset, i) => {
    const icon = asset.type === "video" ? "🎬" : asset.type === "audio" ? "🔊" : "📄";
    return `<div class="asset-card" data-index="${i}">
      <div class="asset-thumb">${icon}</div>
      <div class="asset-card-name">${asset.id}</div>
      <div class="asset-card-meta">${asset.type} — ${asset.detail}</div>
    </div>`;
  }).join("");

  // Click to show detail
  grid.querySelectorAll(".asset-card").forEach(card => {
    card.addEventListener("click", () => {
      grid.querySelectorAll(".asset-card").forEach(c => c.classList.remove("selected"));
      card.classList.add("selected");
      const idx = parseInt(card.dataset.index);
      const asset = assets[idx];
      const detail = document.getElementById("asset-detail");
      detail.style.display = "block";
      document.getElementById("asset-detail-name").textContent = asset.id;
      document.getElementById("asset-detail-info").textContent = `Type: ${asset.type}\n${asset.detail}`;
    });
  });
});

// ========== PROGRESS BAR HELPERS ==========
function createProgressBar(containerId) {
  const container = document.getElementById(containerId);
  if (!container) return null;

  const bar = document.createElement("div");
  bar.className = "progress-bar";
  bar.innerHTML = '<div class="progress-bar-fill" style="width:0%"></div>';
  const text = document.createElement("div");
  text.className = "progress-text";
  text.textContent = "0%";
  container.appendChild(bar);
  container.appendChild(text);

  return {
    update(percent, message) {
      bar.querySelector(".progress-bar-fill").style.width = `${Math.min(100, percent)}%`;
      text.textContent = message || `${Math.round(percent)}%`;
    },
    done(message) {
      bar.querySelector(".progress-bar-fill").style.width = "100%";
      text.textContent = message || "Complete";
    },
    remove() {
      bar.remove();
      text.remove();
    }
  };
}
