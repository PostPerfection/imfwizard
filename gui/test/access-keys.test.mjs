import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { accessKeyProblems, pageButtons } from '../../extern/guikit/test/access-key-check.mjs';

const page = readFileSync(new URL('../index.html', import.meta.url), 'utf8');

test('every view button has its own letter, taken from its label', () => {
  assert.deepEqual(accessKeyProblems(page), []);
});

test('the sidebar has no more views than Ctrl+1 to Ctrl+9 reach', () => {
  assert.ok([...page.matchAll(/class="sidebar-btn[^"]*" data-view=/g)].length <= 9);
});

test('the Project menu items take N, O, S and A', () => {
  const menu = pageButtons(page).filter((button) => button.scope === 'menu project-menu');
  assert.deepEqual(menu.map((button) => button.attributes['data-key']), ['n', 'o', 's', 'a']);
});
