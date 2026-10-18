import assert from 'node:assert/strict';
import test from 'node:test';

import { emptyTimeline } from '../../extern/guikit/src/composition-timeline.js';
import { compositionsRequest, timelinesSetClipLengths } from '../src/composition-request.js';

const ASSETS = [
  { id: 1, type: 'video', path: '/media/feature.mov', name: 'feature.mov', fps: '24/1', frames: 240 },
  { id: 2, type: 'video', path: '/media/trailer.mov', name: 'trailer.mov', fps: '24/1', frames: 96 },
  { id: 3, type: 'audio', path: '/media/trailer.wav', name: 'trailer.wav' },
  { id: 4, type: 'subtitle', path: '/media/trailer.ttml', name: 'trailer.ttml' },
  { id: 5, type: 'audio', path: '/media/mix.wav', name: 'mix.wav' },
  { id: 6, type: 'subtitle', path: '/media/feature.ttml', name: 'feature.ttml' },
];

function assetById(assetId) {
  return ASSETS.find((asset) => asset.id === assetId);
}

function clip(picture, frames, inFrame, outFrame, sound = { kind: 'embedded', asset: null }, subtitle = null) {
  return { picture, frames, inFrame, outFrame, sound, subtitle };
}

function composition(name, timeline) {
  return { id: name.length, name, contentKind: 'feature', timeline: { ...emptyTimeline([24, 1]), ...timeline } };
}

test('one whole clip goes as the single picture request it always was', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240)], sound: 5, subtitle: 6 });

  const { compositions, clipTrims } = compositionsRequest([main], 'Film', assetById);

  assert.deepEqual(compositions, [
    {
      title: 'Film',
      contentKind: 'feature',
      videoPath: '/media/feature.mov',
      audioPath: '/media/mix.wav',
      silent: false,
      subtitles: ['/media/feature.ttml'],
    },
  ]);
  assert.deepEqual(clipTrims, { trimStart: null, trimEnd: null });
  assert.equal(timelinesSetClipLengths([main]), false);
});

test("a trimmed clip sends its in and out points as the trims, with its own sound and timed text", () => {
  const main = composition('Main', { clips: [clip(2, 96, 12, 84, { kind: 'file', asset: 3 }, 4)] });

  const { compositions, clipTrims } = compositionsRequest([main], 'Film', assetById);

  assert.equal(compositions[0].videoPath, '/media/trailer.mov');
  assert.equal(compositions[0].audioPath, '/media/trailer.wav');
  assert.deepEqual(compositions[0].subtitles, ['/media/trailer.ttml']);
  assert.equal('timeline' in compositions[0], false);
  assert.deepEqual(clipTrims, { trimStart: '12f', trimEnd: '12f' });
  assert.equal(timelinesSetClipLengths([main]), true);
});

test('a clip without sound goes silent rather than taking the picture file sound', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240, { kind: 'none', asset: null })] });

  const [request] = compositionsRequest([main], 'Film', assetById).compositions;

  assert.equal(request.silent, true);
  assert.equal(request.audioPath, null);
});

test('two clips go as a timeline of paths and nothing in the single picture fields', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240), clip(2, 96, 12, 84, { kind: 'file', asset: 3 })], reelStarts: [240] });

  const { compositions, clipTrims } = compositionsRequest([main], 'Film', assetById);

  assert.equal(compositions[0].videoPath, '');
  assert.equal(compositions[0].audioPath, null);
  assert.deepEqual(compositions[0].subtitles, []);
  assert.deepEqual(compositions[0].timeline.clips.map((each) => [each.picture, each.in_frame, each.out_frame, each.sound]), [
    ['/media/feature.mov', 0, 240, 'Embedded'],
    ['/media/trailer.mov', 12, 84, { File: '/media/trailer.wav' }],
  ]);
  assert.deepEqual(compositions[0].timeline.reel_starts, [240]);
  assert.deepEqual(clipTrims, { trimStart: null, trimEnd: null });
  assert.equal(timelinesSetClipLengths([main]), true);
});

test('what the single picture request times against the source goes as a timeline', () => {
  const segmented = composition('Main', { clips: [clip(1, 240, 0, 240)], reelStarts: [120] });
  const timedAgainstTimeline = composition('Main', { clips: [clip(1, 240, 24, 240)], sound: 5 });

  for (const timeline of [segmented, timedAgainstTimeline]) {
    const [request] = compositionsRequest([timeline], 'Film', assetById).compositions;
    assert.equal(request.videoPath, '');
    assert.ok(request.timeline);
  }
});

test('two compositions with the same trim share it, and with different trims are refused by name', () => {
  const main = composition('Main', { clips: [clip(1, 240, 12, 228)] });
  const textless = composition('Textless', { clips: [clip(1, 240, 12, 228)] });
  const trailer = composition('Trailer', { clips: [clip(2, 96, 0, 96)] });

  const shared = compositionsRequest([main, textless], 'Film', assetById);
  assert.deepEqual(shared.compositions.map((each) => each.title), ['Film - Main', 'Film - Textless']);
  assert.deepEqual(shared.clipTrims, { trimStart: '12f', trimEnd: '12f' });
  assert.throws(
    () => compositionsRequest([main, trailer], 'Film', assetById),
    { message: 'Main and Trailer keep different frames of their clips, and a trim applies to every composition: build them separately' },
  );
});

test('a composition with nothing on its timeline is left out of the request', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240)] });

  const { compositions } = compositionsRequest([main, composition('Empty', {})], 'Film', assetById);

  assert.deepEqual(compositions.map((each) => each.title), ['Film - Main']);
});
