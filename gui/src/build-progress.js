export function formatTime(seconds) {
  const wholeMinutes = Math.floor(seconds / 60);
  const wholeSeconds = Math.floor(seconds % 60);
  return wholeMinutes > 0 ? `${wholeMinutes}m${wholeSeconds}s` : `${wholeSeconds}s`;
}

export function stageLabel(stage) {
  return stage.charAt(0).toUpperCase() + stage.slice(1);
}

export function progressStatsText(payload) {
  const elapsed = formatTime(payload.elapsed_secs);
  let eta = "";
  if (payload.percent > 0 && payload.percent < 100) {
    eta = ` ETA ${formatTime((payload.elapsed_secs / payload.percent) * (100 - payload.percent))}`;
  }
  return `${elapsed}${payload.fps > 0 ? ` ${payload.fps.toFixed(1)}fps` : ''}${eta}`;
}

export function titleForProgress(percent, stage) {
  if (percent >= 0 && percent < 100) return `IMF Wizard — ${stage} ${Math.round(percent)}%`;
  return "IMF Wizard";
}
