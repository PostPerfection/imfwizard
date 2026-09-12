import assert from 'node:assert/strict';
import test from 'node:test';

import { previewButtonEnabled, previewTarget, PREVIEW_KIND_PACKAGE, PREVIEW_KIND_SOURCE } from '../src/preview-target.js';

const EVERY_TIER = {
  selectedPreview: { kind: PREVIEW_KIND_SOURCE, path: '/assets/picked.mov' },
  firstPicturePath: '/assets/first.mov',
  openedPackage: '/packages/Opened',
  outputPath: '/packages/Output',
};

test('a picked asset wins over everything below it', () => {
  assert.deepEqual(previewTarget(EVERY_TIER), { kind: PREVIEW_KIND_SOURCE, path: '/assets/picked.mov' });
});

test('a picked package wins over the first picture', () => {
  const target = previewTarget({ ...EVERY_TIER, selectedPreview: { kind: PREVIEW_KIND_PACKAGE, path: '/packages/Picked' } });

  assert.deepEqual(target, { kind: PREVIEW_KIND_PACKAGE, path: '/packages/Picked' });
});

test('with nothing picked the first picture wins over both packages', () => {
  const target = previewTarget({ ...EVERY_TIER, selectedPreview: null });

  assert.deepEqual(target, { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' });
});

test('the opened package wins over the output path', () => {
  const target = previewTarget({ ...EVERY_TIER, selectedPreview: null, firstPicturePath: undefined });

  assert.deepEqual(target, { kind: PREVIEW_KIND_PACKAGE, path: '/packages/Opened' });
});

test('the output path is the last resort', () => {
  const target = previewTarget({ ...EVERY_TIER, selectedPreview: null, firstPicturePath: undefined, openedPackage: null });

  assert.deepEqual(target, { kind: PREVIEW_KIND_PACKAGE, path: '/packages/Output' });
});

test('an empty project has nothing to open', () => {
  assert.equal(previewTarget({ selectedPreview: null, firstPicturePath: undefined, openedPackage: null, outputPath: '' }), null);
});

test('the button is on when the target differs from what the panel shows', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, '/assets/other.mov'), true);
});

test('the button is off when the target is already shown', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, '/assets/first.mov'), false);
});

test('the button comes back on once the panel shows nothing', () => {
  const target = { kind: PREVIEW_KIND_SOURCE, path: '/assets/first.mov' };

  assert.equal(previewButtonEnabled(target, null), true);
});

test('the button is off when there is nothing to open', () => {
  assert.equal(previewButtonEnabled(null, null), false);
});
