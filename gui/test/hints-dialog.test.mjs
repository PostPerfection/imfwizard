import assert from 'node:assert/strict';
import test from 'node:test';

import { showHintsDialog } from '../src/hints-dialog.js';

globalThis.document = { createElement: () => ({ textContent: '' }) };

function fakeButton() {
  return {
    handlers: [],
    addEventListener(name, handler) { this.handlers.push(handler); },
    removeEventListener(name, handler) { this.handlers = this.handlers.filter((known) => known !== handler); },
    click() { for (const handler of [...this.handlers]) handler(); },
  };
}

function openDialog(hints) {
  const dialog = { hidden: true };
  const list = { innerHTML: 'stale', items: [], appendChild(item) { this.items.push(item); } };
  const silence = { checked: true };
  const buildButton = fakeButton();
  const backButton = fakeButton();
  const silenced = [];
  const closes = [];
  const answer = showHintsDialog(hints, {
    dialog,
    list,
    silence,
    buildButton,
    backButton,
    onSilence: () => silenced.push('silenced'),
    onClose: () => closes.push('closed'),
  });
  return { answer, dialog, list, silence, buildButton, backButton, silenced, closes };
}

test('the dialog opens with one item per finding', () => {
  const { dialog, list } = openDialog(['Audio is 48 kHz mono', 'No subtitle track']);

  assert.equal(dialog.hidden, false);
  assert.equal(list.innerHTML, '');
  assert.deepEqual(list.items.map((item) => item.textContent), ['Audio is 48 kHz mono', 'No subtitle track']);
});

test('build anyway answers true and closes the dialog', async () => {
  const { answer, dialog, buildButton, closes } = openDialog(['One finding']);

  buildButton.click();

  assert.equal(await answer, true);
  assert.equal(dialog.hidden, true);
  assert.deepEqual(closes, ['closed']);
});

test('go back answers false', async () => {
  const { answer, backButton } = openDialog(['One finding']);

  backButton.click();

  assert.equal(await answer, false);
});

test('the silence checkbox starts clear, so closing it untouched keeps the hints on', async () => {
  const { answer, silence, buildButton, silenced } = openDialog(['One finding']);

  assert.equal(silence.checked, false);
  buildButton.click();
  await answer;

  assert.deepEqual(silenced, []);
});

test('a ticked silence checkbox turns the hints off', async () => {
  const { answer, silence, buildButton, silenced } = openDialog(['One finding']);

  silence.checked = true;
  buildButton.click();
  await answer;

  assert.deepEqual(silenced, ['silenced']);
});

test('a click after the dialog closed does nothing', async () => {
  const { answer, buildButton, backButton, closes } = openDialog(['One finding']);

  backButton.click();
  await answer;
  assert.deepEqual(buildButton.handlers, []);
  assert.deepEqual(backButton.handlers, []);

  backButton.click();
  buildButton.click();

  assert.deepEqual(closes, ['closed']);
});
