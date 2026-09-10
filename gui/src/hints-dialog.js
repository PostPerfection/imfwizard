export function showHintsDialog(hints, { dialog, list, silence, buildButton, backButton, onSilence, onClose }) {
  if (!dialog || !list) return Promise.resolve(true);

  list.innerHTML = "";
  for (const hint of hints) {
    const item = document.createElement("li");
    item.textContent = hint;
    list.appendChild(item);
  }
  silence.checked = false;
  dialog.hidden = false;

  return new Promise((resolve) => {
    const close = (build) => {
      dialog.hidden = true;
      if (silence.checked) onSilence();
      onClose();
      buildButton.removeEventListener("click", onBuild);
      backButton.removeEventListener("click", onBack);
      resolve(build);
    };
    const onBuild = () => close(true);
    const onBack = () => close(false);
    buildButton.addEventListener("click", onBuild);
    backButton.addEventListener("click", onBack);
  });
}
