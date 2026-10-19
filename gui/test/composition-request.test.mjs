import assert from 'node:assert/strict';
import test from 'node:test';

import { emptyTimeline, timelineWire } from '../../extern/guikit/src/composition-timeline.js';
import {
  clipUnprobed,
  compositionsRequest,
  durationFrames,
  everySingleClipProbed,
  sharedClipStillLength,
  sharedClipTrimSpecs,
  timelinesSetClipLengths,
  waitsForClipLengths,
  withClipStillLength,
  withClipTrims,
  withProbedSource,
} from '../src/composition-request.js';

const ASSETS = [
  { id: 1, type: 'video', path: '/media/feature.mov', name: 'feature.mov', fps: '24/1', frames: 240 },
  { id: 2, type: 'video', path: '/media/trailer.mov', name: 'trailer.mov', fps: '24/1', frames: 96 },
  { id: 3, type: 'audio', path: '/media/trailer.wav', name: 'trailer.wav' },
  { id: 4, type: 'subtitle', path: '/media/trailer.ttml', name: 'trailer.ttml' },
  { id: 5, type: 'audio', path: '/media/mix.wav', name: 'mix.wav' },
  { id: 6, type: 'subtitle', path: '/media/feature.ttml', name: 'feature.ttml' },
  { id: 7, type: 'video', path: '/media/card.png', name: 'card.png', still: true },
];

function assetById(assetId) {
  return ASSETS.find((asset) => asset.id === assetId);
}

function clip(picture, frames, inFrame, outFrame, sound = { kind: 'embedded', asset: null }, subtitle = null) {
  return { picture, frames, start: 0, layer: 0, opacity: 1, inFrame, outFrame, sound, subtitle };
}

function stillClip(picture, outFrame) {
  return { ...clip(picture, null, 0, outFrame, { kind: 'none', asset: null }), still: true };
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
  assert.equal(timelinesSetClipLengths([main]), false, 'a trimmed clip leaves the Trim fields on to show its trims');
});

test('a clip without sound goes silent rather than taking the picture file sound', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240, { kind: 'none', asset: null })] });

  const [request] = compositionsRequest([main], 'Film', assetById).compositions;

  assert.equal(request.silent, true);
  assert.equal(request.audioPath, null);
});

test('two clips go as a timeline of paths and nothing in the single picture fields', () => {
  const main = composition('Main', {
    clips: [clip(1, 240, 0, 240), { ...clip(2, 96, 12, 84, { kind: 'file', asset: 3 }), start: 240 }],
    reelStarts: [240],
  });

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
  const afterAGap = composition('Main', { clips: [{ ...clip(1, 240, 0, 240), start: 24 }] });
  const overTheBase = composition('Main', { clips: [{ ...clip(1, 240, 0, 240), layer: 1, sound: { kind: 'none', asset: null } }] });
  const seeThrough = composition('Main', { clips: [{ ...clip(1, 240, 0, 240), opacity: 0.5 }] });

  for (const timeline of [segmented, timedAgainstTimeline, afterAGap, overTheBase, seeThrough]) {
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

test('the Trim fields set every composition\'s single clip, and the build and the preview read the same frames', () => {
  const main = composition('Main', { clips: [clip(1, 240, 0, 240)] });
  const textless = composition('Textless', { clips: [clip(1, 240, 0, 240)] });
  const trimStartFrames = durationFrames('2s', [24, 1]);
  const trimEndFrames = durationFrames('12f', [24, 1]);

  const trimmed = [main, textless].map((each) => withClipTrims(each, trimStartFrames, trimEndFrames));
  const { compositions, clipTrims } = compositionsRequest(trimmed, 'Film', assetById);
  const played = timelineWire(trimmed[0].timeline, assetById).clips[0];

  assert.deepEqual(trimmed.map((each) => each.timeline.clips[0]), [clip(1, 240, 48, 228), clip(1, 240, 48, 228)]);
  assert.deepEqual(clipTrims, { trimStart: '48f', trimEnd: '12f' });
  assert.deepEqual(compositions.map((each) => each.videoPath), ['/media/feature.mov', '/media/feature.mov']);
  assert.deepEqual([played.in_frame, played.out_frame], [48, 228]);
  assert.deepEqual(sharedClipTrimSpecs(trimmed), { trimStart: '48f', trimEnd: '12f' });
  assert.deepEqual(sharedClipTrimSpecs([main]), { trimStart: '', trimEnd: '' });
  assert.equal(sharedClipTrimSpecs([trimmed[0], textless]), null, 'clips keeping different frames leave the fields as typed');
  assert.equal(durationFrames('2s', [24000, 1001]), 48);
  assert.throws(() => withClipTrims(main, 200, 40), RangeError);
});

test('an unprobed clip waits for its frames before it takes the Trim fields, and the build reads the fields until then', () => {
  const unprobed = composition('Main', { clips: [clip(1, 0, 0, 0)] });
  const segmented = composition('Main', { clips: [clip(1, 240, 0, 240)], reelStarts: [120] });

  assert.equal(withClipTrims(unprobed, 48, 12), unprobed);
  assert.equal(everySingleClipProbed([unprobed]), false);
  assert.equal(sharedClipTrimSpecs([unprobed]), null);
  assert.equal(everySingleClipProbed([composition('Main', { clips: [clip(1, 240, 0, 240)] })]), true);
  assert.equal(sharedClipTrimSpecs([segmented]), null, 'the fields are off beside a segment start');
});

test('a one still timeline goes as the single picture request with its hold as the still length and no trims', () => {
  const main = composition('Main', { clips: [stillClip(7, 120)] });

  const { compositions, clipTrims, stillLength } = compositionsRequest([main], 'Card', assetById);

  assert.equal(compositions[0].videoPath, '/media/card.png');
  assert.equal(compositions[0].silent, true);
  assert.equal('timeline' in compositions[0], false);
  assert.equal(stillLength, '120f');
  assert.deepEqual(clipTrims, { trimStart: null, trimEnd: null });
  assert.equal(compositionsRequest([composition('Main', { clips: [clip(1, 240, 0, 240)] })], 'Film', assetById).stillLength, null);
});

test('the Still Length field sets the single still\'s out point and shows its hold, and a still takes no trims', () => {
  const main = composition('Main', { clips: [stillClip(7, 120)] });
  const textless = composition('Textless', { clips: [stillClip(7, 120)] });

  const held = [main, textless].map((each) => withClipStillLength(each, durationFrames('10s', [24, 1])));

  assert.deepEqual(held.map((each) => each.timeline.clips[0]), [stillClip(7, 240), stillClip(7, 240)]);
  assert.equal(compositionsRequest(held, 'Card', assetById).stillLength, '240f');
  assert.equal(sharedClipStillLength(held), '240f');
  assert.deepEqual(sharedClipTrimSpecs(held), { trimStart: '', trimEnd: '' });
  assert.equal(everySingleClipProbed(held), true);
  assert.equal(withClipStillLength(main, null), main, 'an empty field leaves the hold as it is');
  assert.equal(withClipTrims(main, 0, 0), main);
  assert.throws(() => withClipTrims(main, 12, 0), { name: 'RangeError', message: "Main's clip is a still, and a still has no trims" });
  assert.throws(
    () => withClipStillLength(composition('Main', { clips: [clip(1, 240, 0, 240)] }), 48),
    { name: 'RangeError', message: "Main's clip is not a single image, so a still length has nothing to hold" },
  );
  assert.equal(sharedClipStillLength([composition('Main', { clips: [clip(1, 240, 0, 240)] })]), '');
});

test('two compositions holding their stills for different lengths are refused by name, and the field shows neither', () => {
  const main = composition('Main', { clips: [stillClip(7, 120)] });
  const textless = composition('Textless', { clips: [stillClip(7, 48)] });

  assert.equal(sharedClipStillLength([main, textless]), null);
  assert.throws(
    () => compositionsRequest([main, textless], 'Card', assetById),
    { message: 'Main and Textless hold their stills for different lengths, and a still length applies to every composition: build them separately' },
  );
});

test('the Still Length field is off on a timeline of more than one clip', () => {
  const main = composition('Main', { clips: [stillClip(7, 120), { ...clip(1, 240, 0, 240), start: 120 }] });

  assert.equal(timelinesSetClipLengths([main]), true);
  assert.equal(sharedClipStillLength([main]), null);
  assert.equal(compositionsRequest([main], 'Film', assetById).stillLength, null);
});

test("an old project's still clip of 0 frames is re-probed on open, marked a still and takes the Still Length field", () => {
  const opened = composition('Main', { clips: [clip(7, 0, 0, 0)] });

  assert.equal(clipUnprobed(opened.timeline.clips[0]), true);
  assert.equal(waitsForClipLengths(opened, 7), true);
  assert.equal(sharedClipStillLength([opened]), null, 'the field keeps the saved length until the clip is probed');
  const probed = { ...opened, timeline: withProbedSource(opened.timeline, 7, { kind: 'still' }) };
  assert.deepEqual(probed.timeline.clips, [stillClip(7, 120)]);
  assert.equal(clipUnprobed(probed.timeline.clips[0]), false);
  assert.equal(waitsForClipLengths(probed, 7), false);

  const held = withClipStillLength(probed, 240);
  assert.equal(held.timeline.clips[0].outFrame, 240);
  assert.equal(compositionsRequest([held], 'Card', assetById).stillLength, '240f');
});

test('an old project\'s video clip of 0 frames is re-probed on open and counted', () => {
  const opened = composition('Main', { clips: [clip(1, 0, 0, 0)] });

  const probed = withProbedSource(opened.timeline, 1, { kind: 'counted', frames: 240 });

  assert.deepEqual(probed.clips, [clip(1, 240, 0, 240)]);
});
