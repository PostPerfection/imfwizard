import { assetPath, firstSoundAsset, timelineWire } from "../../extern/guikit/src/composition-timeline.js";

function frameSpec(frames) {
  return `${frames}f`;
}

function clipIsTrimmed(clip) {
  return clip.inFrame > 0 || clip.outFrame < clip.frames;
}

// the panel's Trim and Still Length apply while every timeline is one whole clip
export function timelinesSetClipLengths(compositions) {
  return compositions.some(({ timeline }) =>
    timeline.clips.length > 1 || timeline.reelStarts.length > 0 || timeline.clips.some(clipIsTrimmed));
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
    const trimFrames = [clip.inFrame, clip.frames - clip.outFrame];
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
