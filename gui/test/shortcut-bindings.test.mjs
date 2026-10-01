import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { register } from 'node:module';
import test from 'node:test';

register('../../extern/guikit/test/tauri-plugins-hooks.mjs', import.meta.url);

const { BUTTON_SHORTCUTS, PROJECT_BUTTON_SHORTCUTS, THEME_BUTTON_SHORTCUT, VIEW_SHORTCUTS } = await import('../src/shortcut-bindings.js');

const page = readFileSync(new URL('../index.html', import.meta.url), 'utf8');

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
  const bindings = [...BUTTON_SHORTCUTS, ...VIEW_SHORTCUTS].map((shortcut) => shortcut.binding);
  assert.deepEqual(bindings.filter((binding, index) => bindings.indexOf(binding) !== index), []);
});

test('the seven views take Ctrl+1 to Ctrl+7 in sidebar order', () => {
  assert.deepEqual(
    VIEW_SHORTCUTS.map(({ view, binding }) => [view, binding]),
    [
      ['project', 'Ctrl+1'],
      ['timeline', 'Ctrl+2'],
      ['validate', 'Ctrl+3'],
      ['tools', 'Ctrl+4'],
      ['deliver', 'Ctrl+5'],
      ['jobs', 'Ctrl+6'],
      ['settings', 'Ctrl+7'],
    ],
  );
});

test('the theme shortcut is the last button binding', () => {
  assert.equal(BUTTON_SHORTCUTS.at(-1), THEME_BUTTON_SHORTCUT);
  assert.equal(BUTTON_SHORTCUTS.length, PROJECT_BUTTON_SHORTCUTS.length + 1);
});

test('every bound button is a real id in the page', () => {
  for (const { buttonId } of BUTTON_SHORTCUTS) {
    assert.ok(page.includes(`id="${buttonId}"`), `no element with id ${buttonId} in index.html`);
  }
});

test('every view name is a real view id in the page', () => {
  for (const { view } of VIEW_SHORTCUTS) {
    assert.ok(page.includes(`id="view-${view}"`), `no element with id view-${view} in index.html`);
  }
});
