import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  FORM_CONTROLS,
  OUTPUT_FIELDS,
  TEXT_FIELDS,
  serializeForm,
  restoreFormState,
  audioMapCells,
} from '../src/project-form.js';

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

function editedPanel() {
  const panel = emptyPanel((id, property, index) => (property === 'checked' ? index % 2 === 0 : `${id} value`));
  const picture = { id: 3, type: 'video', path: '/media/film.mov', name: 'film.mov', meta: '1920×1080 24/1' };
  const sound = { id: 5, type: 'audio', path: '/media/mix.wav', name: 'mix.wav', meta: '' };
  const subtitle = { id: 6, type: 'subtitle', path: '/media/subs.ttml', name: 'subs.ttml', meta: '' };
  const trailerPicture = { id: 7, type: 'video', path: '/media/trailer.mov', name: 'trailer.mov', meta: '' };
  panel.project = {
    title: 'Film',
    assets: [picture, sound, subtitle, trailerPicture],
    compositions: [
      {
        id: 1,
        name: 'Main',
        contentKind: 'feature',
        segments: [
          { id: 1, picture, sound, subtitle },
          { id: 2, picture: null, sound: null, subtitle: null },
        ],
      },
      { id: 4, name: 'Teaser', contentKind: 'trailer', segments: [{ id: 1, picture: trailerPicture, sound: null, subtitle: null }] },
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

test('a restored segment holds the restored asset itself, so later edits to the asset reach it', () => {
  const reopened = emptyPanel(() => '');
  restoreFormState(JSON.parse(JSON.stringify(serialized(editedPanel()))), serialized(emptyPanel(() => ''), null), reopened);

  const [picture, sound] = reopened.project.assets;
  const firstSegment = reopened.project.compositions[0].segments[0];
  assert.equal(firstSegment.picture, picture);
  assert.equal(firstSegment.sound, sound);
  assert.equal(reopened.project.compositions[1].segments[0].picture, reopened.project.assets[3]);
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

test('a segment slot whose asset is gone from the file is named by composition and segment', () => {
  const saved = JSON.parse(JSON.stringify(serialized(editedPanel())));
  saved.project.assets = saved.project.assets.filter((asset) => asset.id !== 5);
  const reopened = emptyPanel(() => '');

  const { notRestored } = restoreFormState(saved, serialized(emptyPanel(() => ''), null), reopened);

  assert.deepEqual(notRestored, ['Main segment 1 sound']);
  assert.equal(reopened.project.compositions[0].segments[0].sound, null);
  assert.equal(reopened.project.compositions[0].segments[0].picture, reopened.project.assets[0]);
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
