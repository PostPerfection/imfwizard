import assert from 'node:assert/strict';
import test from 'node:test';

import { formatTime, progressStatsText, stageLabel, titleForProgress } from '../src/build-progress.js';

test('a duration under a minute drops the minutes', () => {
  assert.equal(formatTime(9.7), '9s');
  assert.equal(formatTime(125), '2m5s');
});

test('the eta is the elapsed time spread over the percent left', () => {
  assert.equal(progressStatsText({ elapsed_secs: 30, percent: 25, fps: 0 }), '30s ETA 1m30s');
});

test('a measured frame rate shows with one decimal, a zero one not at all', () => {
  assert.equal(progressStatsText({ elapsed_secs: 30, percent: 25, fps: 12.34 }), '30s 12.3fps ETA 1m30s');
  assert.ok(!progressStatsText({ elapsed_secs: 30, percent: 25, fps: 0 }).includes('fps'));
});

test('no eta before the first percent or after the last', () => {
  assert.equal(progressStatsText({ elapsed_secs: 4, percent: 0, fps: 0 }), '4s');
  assert.equal(progressStatsText({ elapsed_secs: 90, percent: 100, fps: 24 }), '1m30s 24.0fps');
});

test('the stage reads as a capitalised word', () => {
  assert.equal(stageLabel('encode'), 'Encode');
});

test('the window title carries the stage and the rounded percent while a build runs', () => {
  assert.equal(titleForProgress(41.4, 'encode'), 'IMF Wizard — encode 41%');
  assert.equal(titleForProgress(0, 'queued'), 'IMF Wizard — queued 0%');
});

test('the window title goes back to the app name when the build ends', () => {
  assert.equal(titleForProgress(100, 'done'), 'IMF Wizard');
  assert.equal(titleForProgress(-1), 'IMF Wizard');
});
