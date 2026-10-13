import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { register } from 'node:module';
import test from 'node:test';

register('../../extern/guikit/test/tauri-plugins-hooks.mjs', import.meta.url);

const { PROJECT_BUTTON_SHORTCUTS } = await import('../src/shortcut-bindings.js');
const { POST_BUILD_SHORTCUTS, THEME_SHORTCUT } = await import('../../extern/guikit/src/shared-shortcuts.js');

const page = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const BUTTON_SHORTCUTS = [...PROJECT_BUTTON_SHORTCUTS, ...POST_BUILD_SHORTCUTS, THEME_SHORTCUT];

function bindingFor(id) {
  return BUTTON_SHORTCUTS.find((shortcut) => shortcut.id === id)?.binding;
}

test('the project buttons carry the bindings the README advertises', () => {
  assert.equal(bindingFor('new-project'), 'Ctrl+N');
  assert.equal(bindingFor('project-open'), 'Ctrl+O');
  assert.equal(bindingFor('project-save'), 'Ctrl+S');
  assert.equal(bindingFor('project-save-as'), 'Ctrl+Shift+S');
  assert.equal(bindingFor('open-imp'), 'Ctrl+Shift+O');
  assert.equal(bindingFor('build'), 'Ctrl+B');
  assert.equal(bindingFor('preview'), 'Ctrl+P');
  assert.equal(bindingFor('import-video'), 'Ctrl+I');
  assert.equal(bindingFor('supplement'), 'Ctrl+Shift+N');
});

test('no two shortcuts share a binding', () => {
  const bindings = BUTTON_SHORTCUTS.map((shortcut) => shortcut.binding);
  assert.deepEqual(bindings.filter((binding, index) => bindings.indexOf(binding) !== index), []);
});

test('every bound button is a real id in the page', () => {
  for (const { buttonId } of BUTTON_SHORTCUTS) {
    assert.ok(page.includes(`id="${buttonId}"`), `no element with id ${buttonId} in index.html`);
  }
});
