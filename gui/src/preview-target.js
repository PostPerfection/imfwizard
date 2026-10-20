export const PREVIEW_KIND_SOURCE = "source";
export const PREVIEW_KIND_PACKAGE = "package";
export const PREVIEW_KIND_TIMELINE = "timeline";

export function previewTarget({ selectedPreview, firstPicturePath, openedPackage, outputPath }) {
  if (selectedPreview?.kind === PREVIEW_KIND_SOURCE) return { kind: PREVIEW_KIND_SOURCE, path: selectedPreview.path };
  if (selectedPreview?.kind === PREVIEW_KIND_PACKAGE) return { kind: PREVIEW_KIND_PACKAGE, path: selectedPreview.path };
  if (firstPicturePath) return { kind: PREVIEW_KIND_SOURCE, path: firstPicturePath };
  if (openedPackage) return { kind: PREVIEW_KIND_PACKAGE, path: openedPackage };
  if (outputPath) return { kind: PREVIEW_KIND_PACKAGE, path: outputPath };
  return null;
}

// with Play whole timeline ticked, a picture a timeline clip shows plays as the whole timeline from that clip's start
export function sourcePreviewTarget(path, clip, playsWholeTimeline) {
  if (clip === null || !playsWholeTimeline) return { kind: PREVIEW_KIND_SOURCE, path };
  return { kind: PREVIEW_KIND_TIMELINE, startFrame: clip.start };
}

export function previewButtonEnabled(target, shownPath, playbackEnded) {
  return target !== null && (target.path !== shownPath || playbackEnded);
}
