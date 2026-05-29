import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

// Timeline state
let timelineData = null; // { segments: [], totalFrames, editRate }
let currentSegmentIdx = -1;
let playheadFrame = 0;
let timelinePollingId = null;
let zoomLevel = 1; // 1 = fit-to-view, >1 = zoomed in

export function initTimeline() {
  renderEmpty();

  // Wire up zoom buttons
  const zoomInBtn = document.getElementById('tl-zoom-in');
  const zoomOutBtn = document.getElementById('tl-zoom-out');
  if (zoomInBtn) zoomInBtn.addEventListener('click', () => setZoom(zoomLevel * 1.5));
  if (zoomOutBtn) zoomOutBtn.addEventListener('click', () => setZoom(zoomLevel / 1.5));
}

function setZoom(level) {
  zoomLevel = Math.max(1, Math.min(20, level));
  const view = document.getElementById('timeline-view');
  if (!view) return;
  const tracks = view.querySelector('.timeline-tracks');
  const ruler = document.getElementById('timeline-ruler');
  const width = zoomLevel === 1 ? '100%' : `${zoomLevel * 100}%`;
  if (tracks) tracks.style.width = width;
  if (ruler) ruler.style.width = width;
  // Scroll playhead into view
  if (zoomLevel > 1 && timelineData && timelineData.totalFrames > 0) {
    const pct = playheadFrame / timelineData.totalFrames;
    view.scrollLeft = pct * view.scrollWidth - view.clientWidth / 2;
  }
}

// Load timeline from an opened IMP's CPL
export async function loadTimelineFromCpl(cplPath) {
  try {
    const segs = await invoke('get_timeline', { cplPath });
    if (!segs || segs.length === 0) {
      renderEmpty();
      return;
    }
    buildTimelineData(segs);
    render();
    startTimelinePolling();
  } catch (e) {
    console.error('[timeline] Failed to load CPL:', e);
    renderEmpty();
  }
}

// Load timeline from the project model (for IMPs being built)
export function loadTimelineFromProject(segments, editRate) {
  if (!segments || segments.length === 0) {
    renderEmpty();
    return;
  }
  const entries = segments.map((seg, i) => ({
    segment_id: String(seg.id || i),
    segment_number: i + 1,
    duration_frames: seg.durationFrames || seg.duration || 0,
    entry_point: 0,
    edit_rate: editRate || '24 1',
    video_track_file_id: '',
    audio_track_file_id: '',
    video_file: seg.videoPath || '',
    audio_file: seg.audioPath || '',
  }));
  buildTimelineData(entries);
  render();
}

function buildTimelineData(segs) {
  let totalFrames = 0;
  const parsed = segs.map(s => {
    const fps = parseEditRate(s.edit_rate);
    const startFrame = totalFrames;
    totalFrames += s.duration_frames;
    return { ...s, startFrame, fps };
  });
  timelineData = { segments: parsed, totalFrames, editRate: parsed[0]?.fps || 24 };
}

function parseEditRate(er) {
  if (!er) return 24;
  const parts = er.trim().split(/\s+/);
  if (parts.length === 2) return Math.round(parseInt(parts[0]) / parseInt(parts[1]));
  return parseInt(parts[0]) || 24;
}

function renderEmpty() {
  const ruler = document.getElementById('timeline-ruler');
  const picture = document.getElementById('timeline-picture');
  const sound = document.getElementById('timeline-sound');
  const subtitle = document.getElementById('timeline-subtitle');
  if (ruler) ruler.innerHTML = '<span class="timeline-empty-msg">Open an IMP or build a project to see the timeline</span>';
  if (picture) picture.innerHTML = '';
  if (sound) sound.innerHTML = '';
  if (subtitle) subtitle.innerHTML = '';
  timelineData = null;
}

function render() {
  if (!timelineData || timelineData.segments.length === 0) {
    renderEmpty();
    return;
  }
  renderRuler();
  renderTracks();
  updatePlayheadPosition();
}

function renderRuler() {
  const ruler = document.getElementById('timeline-ruler');
  if (!ruler) return;
  ruler.innerHTML = '';
  ruler.style.position = 'relative';

  const { totalFrames, editRate } = timelineData;
  if (totalFrames === 0) return;

  const totalSeconds = totalFrames / editRate;
  const interval = getTickInterval(totalSeconds);

  for (let t = 0; t <= totalSeconds; t += interval) {
    const pct = (t / totalSeconds) * 100;
    const tick = document.createElement('div');
    tick.className = 'ruler-tick';
    tick.style.left = `${pct}%`;

    const label = document.createElement('span');
    label.className = 'ruler-label';
    label.textContent = formatTimecodeShort(t);
    tick.appendChild(label);
    ruler.appendChild(tick);
  }

  // Segment boundary markers
  for (const seg of timelineData.segments) {
    if (seg.startFrame > 0) {
      const pct = (seg.startFrame / totalFrames) * 100;
      const marker = document.createElement('div');
      marker.className = 'ruler-reel-marker';
      marker.style.left = `${pct}%`;
      marker.title = `Segment ${seg.segment_number}`;
      ruler.appendChild(marker);
    }
  }

  // Playhead
  const playhead = document.createElement('div');
  playhead.className = 'ruler-playhead';
  playhead.id = 'ruler-playhead';
  ruler.appendChild(playhead);

  // Click on ruler to seek
  ruler.addEventListener('mousedown', handleRulerSeek);
}

function renderTracks() {
  const pictureEl = document.getElementById('timeline-picture');
  const soundEl = document.getElementById('timeline-sound');
  const subtitleEl = document.getElementById('timeline-subtitle');
  if (!pictureEl || !soundEl) return;

  pictureEl.innerHTML = '';
  soundEl.innerHTML = '';
  if (subtitleEl) subtitleEl.innerHTML = '';

  const { segments, totalFrames } = timelineData;
  const colors = ['#7c3aed', '#6d28d9', '#5b21b6', '#4c1d95', '#8b5cf6'];
  const soundColors = ['#3b82f6', '#2563eb', '#1d4ed8', '#1e40af', '#60a5fa'];

  for (const seg of segments) {
    const widthPct = (seg.duration_frames / totalFrames) * 100;
    const leftPct = (seg.startFrame / totalFrames) * 100;

    if (seg.video_file || seg.video_track_file_id) {
      const el = createSegment(seg, leftPct, widthPct, colors[(seg.segment_number - 1) % colors.length]);
      pictureEl.appendChild(el);
    }

    if (seg.audio_file || seg.audio_track_file_id) {
      const el = createSegment(seg, leftPct, widthPct, soundColors[(seg.segment_number - 1) % soundColors.length]);
      soundEl.appendChild(el);
    }
  }

  pictureEl.addEventListener('mousedown', handleTrackSeek);
  soundEl.addEventListener('mousedown', handleTrackSeek);
}

function createSegment(seg, leftPct, widthPct, color) {
  const el = document.createElement('div');
  el.className = 'timeline-segment' + (seg.segment_number - 1 === currentSegmentIdx ? ' active' : '');
  el.style.left = `${leftPct}%`;
  el.style.width = `${widthPct}%`;
  el.style.backgroundColor = color;
  el.dataset.segIndex = seg.segment_number - 1;

  const label = document.createElement('span');
  label.className = 'segment-label';
  label.textContent = `S${seg.segment_number}`;
  el.appendChild(label);

  const dur = document.createElement('span');
  dur.className = 'segment-duration';
  dur.textContent = formatTimecodeShort(seg.duration_frames / (seg.fps || 24));
  el.appendChild(dur);

  return el;
}

function handleRulerSeek(e) {
  const ruler = document.getElementById('timeline-ruler');
  if (!ruler || !timelineData) return;
  const rect = ruler.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  seekToPercent(pct);
}

function handleTrackSeek(e) {
  const track = e.currentTarget;
  if (!track || !timelineData) return;
  const rect = track.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  seekToPercent(pct);
}

async function seekToPercent(pct) {
  if (!timelineData) return;
  const targetFrame = Math.floor(pct * timelineData.totalFrames);

  let targetSeg = null;
  for (const seg of timelineData.segments) {
    if (targetFrame >= seg.startFrame && targetFrame < seg.startFrame + seg.duration_frames) {
      targetSeg = seg;
      break;
    }
  }
  if (!targetSeg) targetSeg = timelineData.segments[timelineData.segments.length - 1];

  const segIdx = targetSeg.segment_number - 1;
  const frameInSeg = targetFrame - targetSeg.startFrame + targetSeg.entry_point;
  const secondsInSeg = frameInSeg / (targetSeg.fps || 24);

  if (segIdx !== currentSegmentIdx && targetSeg.video_file) {
    currentSegmentIdx = segIdx;
    try {
      await invoke('preview_load', { filePath: targetSeg.video_file });
    } catch (e) {
      console.error('[timeline] Failed to load segment:', e);
      return;
    }
  }

  try {
    await invoke('preview_seek_absolute', { seconds: secondsInSeg });
  } catch (e) {
    console.error('[timeline] Failed to seek:', e);
  }

  playheadFrame = targetFrame;
  updatePlayheadPosition();
}

function updatePlayheadPosition() {
  if (!timelineData || timelineData.totalFrames === 0) return;
  const pct = (playheadFrame / timelineData.totalFrames) * 100;
  const rulerPlayhead = document.getElementById('ruler-playhead');
  if (rulerPlayhead) rulerPlayhead.style.left = `${pct}%`;

  document.querySelectorAll('.timeline-segment').forEach(el => {
    const idx = parseInt(el.dataset.segIndex);
    el.classList.toggle('active', idx === currentSegmentIdx);
  });
}

export function startTimelinePolling() {
  if (timelinePollingId) return;
  timelinePollingId = setInterval(async () => {
    if (!timelineData) return;
    try {
      const resp = await invoke('preview_get_metadata');
      const meta = JSON.parse(resp);
      if (meta.position != null && meta.duration != null) {
        const seg = timelineData.segments[currentSegmentIdx] || timelineData.segments[0];
        if (seg) {
          const fps = seg.fps || 24;
          const frameInSeg = Math.floor(meta.position * fps) - seg.entry_point;
          playheadFrame = seg.startFrame + Math.max(0, frameInSeg);

          // Auto-advance to next segment
          if (meta.position >= meta.duration - 0.1 && currentSegmentIdx < timelineData.segments.length - 1) {
            const nextSeg = timelineData.segments[currentSegmentIdx + 1];
            if (nextSeg && nextSeg.video_file) {
              currentSegmentIdx++;
              invoke('preview_load', { filePath: nextSeg.video_file }).catch(() => {});
            }
          }

          updatePlayheadPosition();
        }
      }
    } catch {
      // mpv not running
    }
  }, 250);
}

export function stopTimelinePolling() {
  if (timelinePollingId) {
    clearInterval(timelinePollingId);
    timelinePollingId = null;
  }
}

function getTickInterval(totalSeconds) {
  if (totalSeconds <= 10) return 1;
  if (totalSeconds <= 30) return 5;
  if (totalSeconds <= 60) return 10;
  if (totalSeconds <= 300) return 30;
  if (totalSeconds <= 600) return 60;
  if (totalSeconds <= 1800) return 300;
  return 600;
}

function formatTimecodeShort(seconds) {
  if (!seconds || seconds <= 0) return '0:00';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  return `${m}:${String(s).padStart(2, '0')}`;
}

export function getTimelineSegments() {
  return timelineData?.segments || [];
}

export function getTimelineAsImpArgs() {
  if (!timelineData) return [];
  const segs = timelineData.segments;
  const args = [];
  if (segs.length > 0 && segs[0].video_file) args.push("--video", segs[0].video_file);
  const audioSeg = segs.find(s => s.audio_file);
  if (audioSeg) args.push("--audio", audioSeg.audio_file);
  return args;
}
