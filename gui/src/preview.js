// Preview player - uses mpv via IPC for high-performance video playback
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

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
  });
}

/// Load a file into the preview player
export function previewFile(filePath) {
  invoke('preview_load', { filePath }).catch((e) => {
    console.error('[preview] Failed to load:', e);
  });
}

/// Load a DCP directory into the preview player
export function previewDcp(dirPath) {
  invoke('preview_load_dcp', { dirPath }).catch((e) => {
    console.error('[preview] Failed to load DCP:', e);
  });
}
