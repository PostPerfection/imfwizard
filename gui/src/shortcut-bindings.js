import { PROJECT_FILE_SHORTCUTS } from "../../extern/guikit/src/project.js";
import { SHARED_BINDINGS } from "../../extern/guikit/src/shared-shortcuts.js";

export const PROJECT_BUTTON_SHORTCUTS = [
  ...PROJECT_FILE_SHORTCUTS,
  { id: "open-imp", label: "Open IMP", binding: SHARED_BINDINGS.openPackage, buttonId: "btn-open-imp" },
  { id: "supplement", label: "Create supplement", binding: "Ctrl+Shift+N", buttonId: "btn-supplement" },
  { id: "build", label: "Create IMP", binding: SHARED_BINDINGS.build, buttonId: "btn-build" },
  { id: "preview", label: "Preview", binding: SHARED_BINDINGS.preview, buttonId: "btn-preview" },
  { id: "import-video", label: "Import video", binding: SHARED_BINDINGS.importVideo, buttonId: "import-video" },
].map((shortcut) => ({ ...shortcut, category: "Project" }));
