// Preview player - uses mpv via IPC for high-performance video playback
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

let scrubberInterval = null;
let isSeeking = false;

export function initPreview() {
  // Keyboard shortcuts for preview (space=play/pause, arrows=seek)
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT' || e.target.tagName === 'TEXTAREA') return;
    if (e.key === ' ') {
      e.preventDefault();
      invoke('preview_play_pause').catch(() => {});
    }
    if (e.key === 'ArrowLeft') invoke('preview_seek', { seconds: -5.0 }).catch(() => {});
    if (e.key === 'ArrowRight') invoke('preview_seek', { seconds: 5.0 }).catch(() => {});
    if (e.key === 'Home') invoke('preview_seek_absolute', { seconds: 0.0 }).catch(() => {});
  });

  // Initialize scrubber
  initScrubber();
}

function initScrubber() {
  const scrubber = document.getElementById('timeline-scrubber');
  const playBtn = document.getElementById('timeline-play-btn');
  const durLabel = document.getElementById('timeline-duration');

  if (!scrubber) return;

  // Click to seek
  scrubber.addEventListener('mousedown', (e) => {
    isSeeking = true;
    seekToMouse(e);
  });
  document.addEventListener('mousemove', (e) => {
    if (isSeeking) seekToMouse(e);
  });
  document.addEventListener('mouseup', () => {
    isSeeking = false;
  });

  function seekToMouse(e) {
    const rect = scrubber.getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const dur = parseFloat(durLabel?.dataset.raw || '0');
    if (dur > 0) {
      invoke('preview_seek_absolute', { seconds: pct * dur }).catch(() => {});
      updatePlayhead(pct);
    }
  }

  // Play/pause button
  playBtn?.addEventListener('click', () => {
    invoke('preview_play_pause').catch(() => {});
  });

  // Start position polling
  startScrubberPolling();
}

function startScrubberPolling() {
  if (scrubberInterval) return;
  scrubberInterval = setInterval(async () => {
    if (isSeeking) return;
    try {
      const resp = await invoke('preview_get_metadata');
      const meta = JSON.parse(resp);
      if (meta.position != null && meta.duration != null && meta.duration > 0) {
        const pct = meta.position / meta.duration;
        updatePlayhead(pct);
        updateTimecode(meta.position, meta.duration);
        updatePlayBtn(meta.paused);
      }
    } catch {
      // mpv not running — that's fine
    }
  }, 250);
}

export function stopScrubberPolling() {
  if (scrubberInterval) {
    clearInterval(scrubberInterval);
    scrubberInterval = null;
  }
}

function updatePlayhead(pct) {
  const playhead = document.getElementById('timeline-playhead');
  if (playhead) {
    playhead.style.left = `${(pct * 100).toFixed(2)}%`;
  }
}

function updateTimecode(pos, dur) {
  const posLabel = document.getElementById('timeline-position');
  const durLabel = document.getElementById('timeline-duration');
  if (posLabel) posLabel.textContent = formatTimecode(pos);
  if (durLabel) {
    durLabel.textContent = formatTimecode(dur);
    durLabel.dataset.raw = String(dur);
  }
}

function updatePlayBtn(paused) {
  const playBtn = document.getElementById('timeline-play-btn');
  if (playBtn) {
    playBtn.textContent = paused ? '▶' : '⏸';
    playBtn.title = paused ? 'Play' : 'Pause';
  }
}

function formatTimecode(seconds) {
  if (!seconds || seconds < 0) return '00:00:00:00';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  const f = Math.floor((seconds % 1) * 24); // Assume 24fps for frame display
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}:${String(f).padStart(2, '0')}`;
}

/// Load a file into the preview player
export function previewFile(filePath) {
  invoke('preview_load', { filePath }).catch((e) => {
    console.error('[preview] Failed to load:', e);
  });
  startScrubberPolling();
}

/// Load a DCP directory into the preview player
export function previewDcp(dirPath) {
  invoke('preview_load_dcp', { dirPath }).catch((e) => {
    console.error('[preview] Failed to load DCP:', e);
  });
  startScrubberPolling();
}
