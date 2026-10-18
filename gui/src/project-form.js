import {
  DEFAULT_TIMELINE_FPS,
  embeddedSound,
  emptyTimeline,
  parseFrameRate,
  timelineWithKnownAssets,
} from "../../extern/guikit/src/composition-timeline.js";

const VALUE = "value";
const CHECKED = "checked";

export const PROJECT_FILE_VERSION = 3;

// frames stay 0 until the restore probes the source
function migratedClip(picture, sound, subtitle) {
  const clipSound = sound === null ? embeddedSound() : { kind: "file", asset: sound };
  return { picture, frames: 0, inFrame: 0, outFrame: 0, sound: clipSound, subtitle };
}

// a version 1 asset holds no rate, so the timeline takes the Frame Rate menu's until the first clip is probed
function timelineFromSegments(composition, form, upgradeNotes, reelStartingClipIndices) {
  const timeline = emptyTimeline(parseFrameRate(form.framerate) ?? DEFAULT_TIMELINE_FPS);
  composition.segments.forEach((segment, index) => {
    const segmentName = `${composition.name} segment ${index + 1}`;
    const picture = segment.picture ?? null;
    const sound = segment.sound ?? null;
    const subtitle = segment.subtitle ?? null;
    if (picture === null) {
      if (index === 0) {
        timeline.sound = sound;
        timeline.subtitle = subtitle;
        return;
      }
      if (sound !== null) upgradeNotes.push(`${segmentName} sound stays off the timeline: the segment has no picture`);
      if (subtitle !== null) upgradeNotes.push(`${segmentName} timed text stays off the timeline: the segment has no picture`);
      return;
    }
    timeline.clips.push(migratedClip(picture, sound, subtitle));
    if (index === 0) return;
    reelStartingClipIndices.push(timeline.clips.length - 1);
    upgradeNotes.push(`${segmentName} is clip ${timeline.clips.length} of the timeline`);
  });
  return timeline;
}

function segmentsIntoTimeline(form, upgradeNotes) {
  if (!form.project) return form;
  const compositions = form.project.compositions.map(({ segments, ...composition }) => {
    const reelStartingClipIndices = [];
    const timeline = timelineFromSegments({ ...composition, segments }, form, upgradeNotes, reelStartingClipIndices);
    return { ...composition, timeline, reelStartingClipIndices };
  });
  return { ...form, project: { ...form.project, compositions } };
}

// a version 2 timeline plays its clips end to end on one picture track
function clipsPlacedEndToEnd(form) {
  if (!form.project) return form;
  const compositions = form.project.compositions.map((composition) => {
    let start = 0;
    const clips = composition.timeline.clips.map((clip) => {
      const placed = { ...clip, start, layer: 0, opacity: 1 };
      start += clip.outFrame - clip.inFrame;
      return placed;
    });
    return { ...composition, timeline: { ...composition.timeline, clips } };
  });
  return { ...form, project: { ...form.project, compositions } };
}

// a rename or a meaning change gets a step and a version bump, a new field with a default does not
export const PROJECT_FILE_MIGRATIONS = { 2: segmentsIntoTimeline, 3: clipsPlacedEndToEnd };

// [form key, control id, control property], keyed like the submit_job payload
export const FORM_CONTROLS = [
  ["title", "prop-title", VALUE],
  ["issuer", "prop-issuer", VALUE],
  ["deliveryPreset", "prop-preset", VALUE],
  ["contentKind", "prop-content-kind", VALUE],
  ["framerate", "prop-framerate", VALUE],
  ["sourceColourspace", "prop-source-colourspace", VALUE],
  ["hdr", "prop-hdr", VALUE],
  ["bandwidth", "prop-bandwidth", VALUE],
  ["qualityPsnr", "prop-quality-psnr", VALUE],
  ["stillLength", "prop-still-length", VALUE],
  ["burnSubtitle", "prop-burn-subtitle", VALUE],
  ["burnSubtitleFont", "prop-burn-subtitle-font", VALUE],
  ["burnFontSize", "prop-burn-font-size", VALUE],
  ["burnOutlineWidth", "prop-burn-outline-width", VALUE],
  ["burnLineHeight", "prop-burn-line-height", VALUE],
  ["burnMargin", "prop-burn-margin", VALUE],
  ["burnColour", "prop-burn-colour", VALUE],
  ["burnEffectColour", "prop-burn-effect-colour", VALUE],
  ["burnFadeUp", "prop-burn-fade-up", VALUE],
  ["burnFadeDown", "prop-burn-fade-down", VALUE],
  ["burnEffect", "prop-burn-effect", VALUE],
  ["cropLeft", "prop-crop-left", VALUE],
  ["cropRight", "prop-crop-right", VALUE],
  ["cropTop", "prop-crop-top", VALUE],
  ["cropBottom", "prop-crop-bottom", VALUE],
  ["fillCrop", "prop-fill-crop", CHECKED],
  ["deinterlace", "prop-deinterlace", CHECKED],
  ["denoise", "prop-denoise", CHECKED],
  ["rotate", "prop-rotate", VALUE],
  ["flip", "prop-flip", VALUE],
  ["raster", "prop-raster", VALUE],
  ["channels", "prop-channels", VALUE],
  ["audioDelayMs", "prop-audio-delay", VALUE],
  ["trimStart", "prop-trim-start", VALUE],
  ["trimEnd", "prop-trim-end", VALUE],
  ["outputDir", "prop-output", VALUE],
];

// the output folder is the package folder the build writes, so it need not exist yet
export const OUTPUT_FIELDS = ["outputDir"];

// free text that can start with a slash, matched at any depth
export const TEXT_FIELDS = [
  "title",
  "name",
  "meta",
  "issuer",
  "audioMap",
  "stillLength",
  "trimStart",
  "trimEnd",
  "burnColour",
  "burnEffectColour",
];

export function serializeForm({ elementById, project, audioMap }) {
  const form = {};
  for (const [key, id, property] of FORM_CONTROLS) form[key] = elementById(id)[property];
  return {
    ...form,
    audioMap,
    project: {
      title: project.title,
      assets: project.assets.map((asset) => ({ ...asset })),
      compositions: project.compositions.map(({ reelStartingClipIndices, ...composition }) => ({
        ...composition,
        timeline: structuredClone(composition.timeline),
      })),
      activeComposition: project.activeComposition,
    },
  };
}

function offersOption(select, value) {
  return [...select.options].some((option) => option.value === value);
}

export function restoreFormState(savedForm, defaults, { elementById, project }) {
  const form = { ...defaults, ...savedForm, project: { ...defaults.project, ...savedForm.project } };
  const notRestored = [];
  for (const [key, id, property] of FORM_CONTROLS) {
    const element = elementById(id);
    const offered = !element.options || offersOption(element, form[key]);
    if (!offered) notRestored.push(`${element.labels[0].textContent.trim()} option ${form[key]}`);
    element[property] = offered ? form[key] : defaults[key];
  }

  const assets = form.project.assets.map((asset) => ({ ...asset }));
  const assetsById = new Map(assets.map((asset) => [asset.id, asset]));
  project.title = form.project.title;
  project.assets = assets;
  project.compositions = form.project.compositions.map((composition) => ({
    ...composition,
    timeline: timelineWithKnownAssets(composition, assetsById, notRestored),
  }));
  project.activeComposition = form.project.activeComposition;
  return { form, notRestored };
}

// a bare IN:OUTPUT pair is routed at 0 dB
export function audioMapCells(spec) {
  if (!spec) return [];
  return spec.split(",").map((entry) => {
    const [pair, gain = "0"] = entry.split("@");
    const [input, output] = pair.split(":");
    return { input, output, gain };
  });
}
