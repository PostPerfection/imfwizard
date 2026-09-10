export const PROJECT_BUTTON_SHORTCUTS = [
  { id: "new-project", label: "New IMP", binding: "Ctrl+N", buttonId: "btn-new-project" },
  { id: "open-project", label: "Open IMP", binding: "Ctrl+O", buttonId: "btn-open-project" },
  { id: "supplement", label: "Create supplement", binding: "Ctrl+Shift+S", buttonId: "btn-supplement" },
  { id: "build", label: "Create IMP", binding: "Ctrl+B", buttonId: "btn-build" },
  { id: "preview", label: "Preview", binding: "Ctrl+P", buttonId: "btn-preview" },
  { id: "import-video", label: "Import video", binding: "Ctrl+I", buttonId: "import-video" },
];

export const THEME_BUTTON_SHORTCUT = { id: "toggle-theme", label: "Toggle light / dark theme", binding: "Ctrl+Shift+T", buttonId: "theme-toggle" };

export const BUTTON_SHORTCUTS = [...PROJECT_BUTTON_SHORTCUTS, THEME_BUTTON_SHORTCUT];

export const VIEW_SHORTCUTS = [
  { view: "project", label: "Project", binding: "Ctrl+1" },
  { view: "timeline", label: "Timeline", binding: "Ctrl+2" },
  { view: "validate", label: "Validate", binding: "Ctrl+3" },
  { view: "tools", label: "Tools", binding: "Ctrl+4" },
  { view: "deliver", label: "Deliver", binding: "Ctrl+5" },
  { view: "jobs", label: "Jobs", binding: "Ctrl+6" },
  { view: "settings", label: "Settings", binding: "Ctrl+7" },
];
