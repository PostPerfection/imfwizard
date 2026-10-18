import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { register } from 'node:module';
import test from 'node:test';

import {
  FORM_CONTROLS,
  OUTPUT_FIELDS,
  TEXT_FIELDS,
  PROJECT_FILE_VERSION,
  PROJECT_FILE_MIGRATIONS,
  serializeForm,
  restoreFormState,
  audioMapCells,
} from '../src/project-form.js';

import { withProbedFrames, withReelStartingClipsPlaced } from '../../extern/guikit/src/composition-timeline.js';

register('../../extern/guikit/test/tauri-plugins-hooks.mjs', import.meta.url);

const { readProjectFile } = await import('../../extern/guikit/src/project.js');

// the threshold only drives Auto-crop
const CONTROLS_NOT_SAVED = ['prop-auto-crop-threshold'];

const CONTROL_TAG = /<(?:input|select|textarea)\b[^>]*\bid="(prop-[^"]+)"/g;

function panelControls(fill) {
  const controls = new Map();
  FORM_CONTROLS.forEach(([, id, property], index) => {
    controls.set(id, { [property]: fill(id, property, index) });
  });
  return controls;
}

function emptyPanel(fill) {
  const controls = panelControls(fill);
  return {
    controls,
    elementById: (id) => controls.get(id),
    project: { title: '', assets: [], compositions: [], activeComposition: 0 },
  };
}

function clip(picture, frames, inFrame, outFrame, sound = { kind: 'embedded', asset: null }, subtitle = null) {
  return { picture, frames, inFrame, outFrame, sound, subtitle };
}

function timeline(fields) {
  return { fps: [24, 1], clips: [], sound: null, subtitle: null, reelStarts: [], ...fields };
}

function editedPanel() {
  const panel = emptyPanel((id, property, index) => (property === 'checked' ? index % 2 === 0 : `${id} value`));
  const picture = { id: 3, type: 'video', path: '/media/film.mov', name: 'film.mov', meta: '1920×1080 24/1', fps: '24/1', frames: 240 };
  const sound = { id: 5, type: 'audio', path: '/media/mix.wav', name: 'mix.wav', meta: '' };
  const subtitle = { id: 6, type: 'subtitle', path: '/media/subs.ttml', name: 'subs.ttml', meta: '' };
  const trailerPicture = { id: 7, type: 'video', path: '/media/trailer.mov', name: 'trailer.mov', meta: '', fps: '24/1', frames: 96 };
  panel.project = {
    title: 'Film',
    assets: [picture, sound, subtitle, trailerPicture],
    compositions: [
      {
        id: 1,
        name: 'Main',
        contentKind: 'feature',
        timeline: timeline({ clips: [clip(3, 240, 0, 240), clip(7, 96, 12, 84, { kind: 'file', asset: 5 })], subtitle: 6, reelStarts: [240] }),
      },
      { id: 4, name: 'Teaser', contentKind: 'trailer', timeline: timeline({ clips: [clip(7, 96, 0, 96)] }) },
    ],
    activeComposition: 1,
  };
  return panel;
}

function serialized(panel, audioMap = '1:L,2:R@-3') {
  return serializeForm({ ...panel, audioMap });
}

test('serialize then restore gives back every field of the properties panel', () => {
  const edited = editedPanel();
  const saved = JSON.parse(JSON.stringify(serialized(edited)));
  const defaults = serialized(emptyPanel(() => ''), null);

  const reopened = emptyPanel(() => '');
  const restored = restoreFormState(saved, defaults, reopened);

  assert.deepEqual(serialized(reopened, restored.form.audioMap), serialized(edited));
  for (const [, id, property] of FORM_CONTROLS) {
    assert.deepEqual(reopened.controls.get(id)[property], edited.controls.get(id)[property], id);
  }
  assert.equal(restored.form.audioMap, '1:L,2:R@-3');
  assert.deepEqual(restored.notRestored, []);
});

test('the saved project holds each timeline, and never the clips still waiting for a segment start', () => {
  const edited = editedPanel();
  edited.project.compositions[1].reelStartingClipIndices = [1];

  const saved = serialized(edited);

  assert.deepEqual(saved.project.compositions[0].timeline, editedPanel().project.compositions[0].timeline);
  assert.equal('reelStartingClipIndices' in saved.project.compositions[1], false);
});

test('a field missing from the file takes the panel default', () => {
  const defaultsPanel = emptyPanel((id, property) => (property === 'checked' ? true : `${id} default`));
  const defaults = serialized(defaultsPanel, null);
  const reopened = editedPanel();

  restoreFormState({ title: 'Only a title' }, defaults, reopened);

  assert.equal(reopened.controls.get('prop-title').value, 'Only a title');
  assert.equal(reopened.controls.get('prop-framerate').value, 'prop-framerate default');
  assert.equal(reopened.controls.get('prop-denoise').checked, true);
  assert.deepEqual(reopened.project.assets, []);
  assert.deepEqual(reopened.project.compositions, []);
});

function openedFixture(name) {
  const text = readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8');
  return readProjectFile(text, 'imfwizard', PROJECT_FILE_VERSION, PROJECT_FILE_MIGRATIONS);
}

test('a version 1 project file opens and restores its saved fields', () => {
  const { form, version, upgradeNotes } = openedFixture('Film-version-1.imfwizard');
  const reopened = emptyPanel(() => '');

  const { notRestored } = restoreFormState(form, serialized(emptyPanel(() => ''), null), reopened);

  assert.equal(version, 1);
  assert.deepEqual(upgradeNotes, []);
  assert.deepEqual(notRestored, []);
  assert.equal(reopened.controls.get('prop-title').value, 'Film');
  assert.equal(reopened.controls.get('prop-preset').value, 'netflix');
  assert.equal(reopened.controls.get('prop-framerate').value, '24000/1001');
  assert.equal(reopened.controls.get('prop-raster').value, '3840x2160');
  assert.equal(reopened.controls.get('prop-deinterlace').checked, true);
  assert.equal(reopened.controls.get('prop-output').value, '/home/user/IMF/Film');
  assert.equal(form.audioMap, '1:L,2:R@-3');
  assert.equal(reopened.project.assets[0].path, '/media/film.mov');
  assert.deepEqual(reopened.project.compositions[0].timeline, timeline({
    fps: [24000, 1001],
    clips: [clip(1, 0, 0, 0, { kind: 'file', asset: 2 }, 3)],
  }));
});

test('a version 1 project puts each later segment on the timeline as a clip that starts a segment, and says so', () => {
  const { form, upgradeNotes } = openedFixture('Film-two-segments-version-1.imfwizard');
  const reopened = emptyPanel(() => '');
  restoreFormState(form, serialized(emptyPanel(() => ''), null), reopened);
  const [composition] = reopened.project.compositions;

  assert.deepEqual(upgradeNotes, [
    'Main segment 2 is clip 2 of the timeline',
    'Main segment 3 sound stays off the timeline: the segment has no picture',
  ]);
  assert.deepEqual(composition.timeline.clips, [
    clip(1, 0, 0, 0, { kind: 'file', asset: 2 }, 3),
    clip(4, 0, 0, 0, { kind: 'file', asset: 5 }, 6),
  ]);
  assert.deepEqual(composition.reelStartingClipIndices, [1]);

  const probed = withReelStartingClipsPlaced({
    ...composition,
    timeline: withProbedFrames(withProbedFrames(composition.timeline, 1, 2400), 4, 480),
  });
  assert.deepEqual(probed.timeline.reelStarts, [2400]);
  const unreadableSecondSegmentFrames = 0;
  const secondProbeFailed = withReelStartingClipsPlaced({
    ...composition,
    timeline: withProbedFrames(withProbedFrames(composition.timeline, 1, 2400), 4, unreadableSecondSegmentFrames),
  });
  assert.deepEqual(secondProbeFailed.timeline.reelStarts, [2400], 'the segment start outlives the failed probe');
  reopened.project.compositions = [probed];
  const saved = JSON.parse(JSON.stringify(serialized(reopened, form.audioMap)));
  const savedAgain = emptyPanel(() => '');
  restoreFormState(saved, serialized(emptyPanel(() => ''), null), savedAgain);
  assert.deepEqual(savedAgain.project.compositions, [probed]);
});

function selectOffering(label, values, value) {
  return { value, labels: [{ textContent: ` ${label} ` }], options: values.map((optionValue) => ({ value: optionValue })) };
}

test('a saved select value the panel does not offer is named and the select keeps its default', () => {
  const defaults = serialized(emptyPanel(() => ''), null);
  defaults.framerate = '24/1';
  const reopened = emptyPanel(() => '');
  reopened.controls.set('prop-framerate', selectOffering('Frame Rate', ['24/1', '25/1'], '24/1'));
  reopened.controls.set('prop-hdr', selectOffering('HDR (ST 2067-21)', ['', 'pq-bt2020'], ''));

  const { notRestored } = restoreFormState({ framerate: '23/1', hdr: 'pq-bt2020' }, defaults, reopened);

  assert.deepEqual(notRestored, ['Frame Rate option 23/1']);
  assert.equal(reopened.controls.get('prop-framerate').value, '24/1');
  assert.equal(reopened.controls.get('prop-hdr').value, 'pq-bt2020');
});

test('a clip whose asset is gone from the file is named by composition and clip', () => {
  const saved = JSON.parse(JSON.stringify(serialized(editedPanel())));
  saved.project.assets = saved.project.assets.filter((asset) => asset.id !== 5);
  const reopened = emptyPanel(() => '');

  const { notRestored } = restoreFormState(saved, serialized(emptyPanel(() => ''), null), reopened);

  assert.deepEqual(notRestored, ['Main clip 2 sound']);
  assert.deepEqual(reopened.project.compositions[0].timeline.clips[1].sound, { kind: 'embedded', asset: null });
});

test('every properties panel control is saved, and every saved control is on the panel', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  const onPanel = [...html.matchAll(CONTROL_TAG)].map((match) => match[1]);
  const saved = FORM_CONTROLS.map(([, id]) => id);

  assert.deepEqual(onPanel.filter((id) => !saved.includes(id) && !CONTROLS_NOT_SAVED.includes(id)), []);
  assert.deepEqual(saved.filter((id) => !onPanel.includes(id)), []);
});

function keysAtAnyDepth(value, keys = new Set()) {
  if (Array.isArray(value)) value.forEach((item) => keysAtAnyDepth(item, keys));
  else if (value && typeof value === 'object') {
    for (const [key, field] of Object.entries(value)) {
      keys.add(key);
      keysAtAnyDepth(field, keys);
    }
  }
  return keys;
}

test('the text and output field lists only name keys a saved form holds', () => {
  const saved = keysAtAnyDepth(serialized(editedPanel()));
  const topLevel = Object.keys(serialized(editedPanel()));

  assert.deepEqual(TEXT_FIELDS.filter((key) => !saved.has(key)), []);
  assert.deepEqual(OUTPUT_FIELDS.filter((key) => !topLevel.includes(key)), []);
});

test('the audio map spec comes back as the cells it was read from', () => {
  assert.deepEqual(audioMapCells('1:L,2:R@-3,3:C@0.5'), [
    { input: '1', output: 'L', gain: '0' },
    { input: '2', output: 'R', gain: '-3' },
    { input: '3', output: 'C', gain: '0.5' },
  ]);
  assert.deepEqual(audioMapCells(null), []);
});
