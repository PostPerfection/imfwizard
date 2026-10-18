import { assetPath, firstSoundAsset, timelineWire } from "../../extern/guikit/src/composition-timeline.js";
import { setClipRange } from "../../extern/guikit/src/timeline-editor.js";

const SECONDS_UNIT = "s";

// durations are spelled as the CLI spells them: frames or seconds, never bare
export const DURATION_SPEC = /^(\d+f|\d+(\.\d+)?s)$/i;

function frameSpec(frames) {
  return `${frames}f`;
}

// the panel's Trim and Still Length apply while every timeline is at most one clip with no segment start
export function timelinesSetClipLengths(compositions) {
  return compositions.some(({ timeline }) => timeline.clips.length > 1 || timeline.reelStarts.length > 0);
}

// a spec DURATION_SPEC accepts, read at the job's rate as the build reads it
export function durationFrames(spec, [numerator, denominator]) {
  const value = Number(spec.slice(0, -1));
  if (spec.slice(-1).toLowerCase() !== SECONDS_UNIT) return value;
  return Math.round((value * numerator) / denominator);
}

// the Trim fields are the single clip's in and out points, which an unprobed clip takes once it is probed
export function withClipTrims(composition, trimStartFrames, trimEndFrames) {
  const { timeline } = composition;
  const [clip] = timeline.clips;
  if (!clip || clip.frames === 0) return composition;
  const outFrame = clip.frames - trimEndFrames;
  if (trimStartFrames >= outFrame) {
    throw new RangeError(`The trims leave none of the ${clip.frames} frames of ${composition.name}'s clip`);
  }
  return { ...composition, timeline: setClipRange(timeline, 0, trimStartFrames, outFrame) };
}

function singleClips(compositions) {
  return compositions.filter(({ timeline }) => timeline.clips.length === 1).map(({ timeline }) => timeline.clips[0]);
}

export function everySingleClipProbed(compositions) {
  return singleClips(compositions).every((clip) => clip.frames > 0);
}

function clipTrimFrames(clip) {
  return [clip.inFrame, clip.frames - clip.outFrame];
}

// what the Trim fields show, null while they are off, a clip is unprobed or the clips keep different frames
export function sharedClipTrimSpecs(compositions) {
  const clips = singleClips(compositions);
  if (timelinesSetClipLengths(compositions) || clips.length === 0 || !everySingleClipProbed(compositions)) return null;
  const [trimStart, trimEnd] = clipTrimFrames(clips[0]);
  if (!clips.every((clip) => sameTrim(clipTrimFrames(clip), [trimStart, trimEnd]))) return null;
  return { trimStart: trimStart > 0 ? frameSpec(trimStart) : "", trimEnd: trimEnd > 0 ? frameSpec(trimEnd) : "" };
}

// mirrors takes_the_single_path, where a sound or subtitle on the timeline is timed against the timeline
function takesSingleRequest(timeline, clip) {
  if (timeline.reelStarts.length > 0) return false;
  if (timeline.subtitle !== null && clip.subtitle !== null) return false;
  return clip.inFrame === 0 || (timeline.sound === null && timeline.subtitle === null);
}

function singleRequest(timeline, clip, assetById) {
  const subtitle = clip.subtitle ?? timeline.subtitle;
  return {
    videoPath: assetPath(assetById, clip.picture),
    audioPath: firstSoundAsset({ ...timeline, clips: [clip] }, assetById)?.path ?? null,
    silent: timeline.sound === null && clip.sound.kind === "none",
    subtitles: subtitle === null ? [] : [assetPath(assetById, subtitle)],
  };
}

function compositionRequest(composition, title, assetById) {
  const { timeline } = composition;
  const [clip, ...laterClips] = timeline.clips;
  const named = { title, contentKind: composition.contentKind || "feature" };
  if (laterClips.length === 0 && takesSingleRequest(timeline, clip)) {
    const trimFrames = clipTrimFrames(clip);
    return { request: { ...named, ...singleRequest(timeline, clip, assetById) }, trimFrames };
  }
  const timelineRequest = { videoPath: "", audioPath: null, subtitles: [], timeline: timelineWire(timeline, assetById) };
  return { request: { ...named, ...timelineRequest }, trimFrames: null };
}

function sameTrim(first, second) {
  return first[0] === second[0] && first[1] === second[1];
}

// submit_job's compositions, and the trims a single clip's in and out points set, which every composition shares
export function compositionsRequest(compositions, title, assetById) {
  const titled = compositions.length > 1;
  const built = compositions
    .filter((composition) => composition.timeline.clips.length > 0)
    .map((composition) => ({
      name: composition.name,
      ...compositionRequest(composition, titled ? `${title} - ${composition.name}` : title, assetById),
    }));
  const singles = built.filter(({ trimFrames }) => trimFrames !== null);
  const trimmed = singles.find(({ trimFrames }) => trimFrames.some((frames) => frames > 0));
  const disagreeing = trimmed && singles.find(({ trimFrames }) => !sameTrim(trimFrames, trimmed.trimFrames));
  if (disagreeing) {
    throw new Error(
      `${trimmed.name} and ${disagreeing.name} keep different frames of their clips, and a trim applies to every composition: build them separately`,
    );
  }
  const [trimStart, trimEnd] = trimmed ? trimmed.trimFrames : [0, 0];
  return {
    compositions: built.map(({ request }) => request),
    clipTrims: { trimStart: trimStart > 0 ? frameSpec(trimStart) : null, trimEnd: trimEnd > 0 ? frameSpec(trimEnd) : null },
  };
}
