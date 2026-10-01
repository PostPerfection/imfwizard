const VALUE = "value";
const CHECKED = "checked";

export const PROJECT_FILE_VERSION = 1;

// a rename or a meaning change gets a step and a version bump, a new field with a default does not
export const PROJECT_FILE_MIGRATIONS = {};

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

const SEGMENT_SLOTS = ["picture", "sound", "subtitle"];

function assetId(asset) {
  return asset ? asset.id : null;
}

export function serializeForm({ elementById, project, audioMap }) {
  const form = {};
  for (const [key, id, property] of FORM_CONTROLS) form[key] = elementById(id)[property];
  return {
    ...form,
    audioMap,
    project: {
      title: project.title,
      assets: project.assets.map((asset) => ({ ...asset })),
      compositions: project.compositions.map((composition) => ({
        ...composition,
        segments: composition.segments.map((segment) => ({
          id: segment.id,
          picture: assetId(segment.picture),
          sound: assetId(segment.sound),
          subtitle: assetId(segment.subtitle),
        })),
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
    segments: composition.segments.map((segment, index) => {
      const restoredSegment = { id: segment.id };
      for (const slot of SEGMENT_SLOTS) {
        const savedId = segment[slot] ?? null;
        restoredSegment[slot] = assetsById.get(savedId) || null;
        if (savedId !== null && !restoredSegment[slot]) notRestored.push(`${composition.name} segment ${index + 1} ${slot}`);
      }
      return restoredSegment;
    }),
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
