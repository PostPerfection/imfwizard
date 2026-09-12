import assert from 'node:assert/strict';
import test from 'node:test';

import { addRecentProject, getRecentProjects, initRecentProjects, removeRecentProject, renderRecentProjects } from '../src/recent-projects.js';

const COLLAPSED_KEY = 'imfwizard-recent-projects-collapsed';
const MAX_RECENT = 20;

const ROW_ELEMENT = /<(?:div|button) class="([\w-]+)" data-path="([^"]*)"/g;

function installStorage() {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
  return store;
}

function parseRows(html) {
  return [...html.matchAll(ROW_ELEMENT)].map(([, className, path]) => ({
    className,
    dataset: { path },
    handlers: [],
    addEventListener(name, handler) { this.handlers.push(handler); },
    click() { for (const handler of this.handlers) handler({ stopPropagation() {} }); },
  }));
}

function fakeList() {
  let html = '';
  let rows = [];
  return {
    get innerHTML() { return html; },
    set innerHTML(value) { html = value; rows = parseRows(value); },
    querySelectorAll(selector) { return rows.filter((row) => row.className === selector.slice(1)); },
  };
}

function fakeSection() {
  const classes = new Set();
  return {
    hidden: false,
    classes,
    classList: { toggle: (name, on) => (on ? classes.add(name) : classes.delete(name)) },
  };
}

function fakeToggle() {
  return {
    textContent: '',
    attributes: {},
    setAttribute(name, value) { this.attributes[name] = value; },
  };
}

function fakeHeader() {
  return {
    handlers: [],
    addEventListener(name, handler) { this.handlers.push(handler); },
    click() { for (const handler of this.handlers) handler(); },
  };
}

function setup() {
  const store = installStorage();
  const section = fakeSection();
  const list = fakeList();
  const toggle = fakeToggle();
  const header = fakeHeader();
  const opened = [];
  const queued = [];
  const retitled = [];
  const deleted = [];
  const statuses = [];
  initRecentProjects({
    section,
    list,
    header,
    toggle,
    onOpen: (path) => opened.push(path),
    onQueue: (path) => queued.push(path),
    onRetitle: (path) => retitled.push(path),
    onDelete: (path) => deleted.push(path),
    afterRender: () => {},
    setStatus: (text) => statuses.push(text),
  });
  return { store, section, list, toggle, header, opened, queued, retitled, deleted, statuses };
}

function paths() {
  return getRecentProjects().map((entry) => entry.path);
}

function rowPaths(list) {
  return list.querySelectorAll('.recent-item').map((row) => row.dataset.path);
}

test('the newest project is first', () => {
  setup();

  addRecentProject('/imps/First', 'First');
  addRecentProject('/imps/Second', 'Second');

  assert.deepEqual(paths(), ['/imps/Second', '/imps/First']);
});

test('building the same project again moves it up instead of listing it twice', () => {
  setup();

  addRecentProject('/imps/A', 'A');
  addRecentProject('/imps/B', 'B');
  addRecentProject('/imps/C', 'C');
  addRecentProject('/imps/A', 'A again');

  assert.deepEqual(paths(), ['/imps/A', '/imps/C', '/imps/B']);
  assert.equal(getRecentProjects()[0].title, 'A again');
});

test('opening a project again keeps its row where it was', () => {
  const { list } = setup();

  addRecentProject('/imps/A', 'A');
  addRecentProject('/imps/B', 'B');
  addRecentProject('/imps/C', 'C');
  addRecentProject('/imps/A', 'A again');

  assert.deepEqual(rowPaths(list), ['/imps/C', '/imps/B', '/imps/A']);
  assert.equal(paths()[0], '/imps/A');

  addRecentProject('/imps/D', 'D');

  assert.deepEqual(rowPaths(list), ['/imps/D', '/imps/C', '/imps/B', '/imps/A']);
});

test('the list stops at twenty and drops the oldest', () => {
  setup();

  for (let index = 1; index <= MAX_RECENT + 5; index++) addRecentProject(`/imps/${index}`, `Build ${index}`);

  assert.equal(getRecentProjects().length, MAX_RECENT);
  assert.equal(paths()[0], `/imps/${MAX_RECENT + 5}`);
  assert.ok(!paths().includes('/imps/1'));
});

test('removing one project leaves the rest alone', () => {
  setup();

  addRecentProject('/imps/A', 'A');
  addRecentProject('/imps/B', 'B');
  removeRecentProject('/imps/A');

  assert.deepEqual(paths(), ['/imps/B']);
});

test('an empty history hides the section', () => {
  const { section, list } = setup();

  renderRecentProjects();

  assert.equal(section.hidden, true);
  assert.equal(list.innerHTML, '');
});

test('each project is one row, named by the last path segment when it has no title', () => {
  const { section, list } = setup();

  addRecentProject('/imps/Sol Levante', '');
  addRecentProject('/imps/Meridian', 'Meridian OV');

  assert.equal(section.hidden, false);
  assert.equal(list.querySelectorAll('.recent-item').length, 2);
  assert.ok(list.innerHTML.includes('<span class="recent-title">Sol Levante</span>'));
  assert.ok(list.innerHTML.includes('<span class="recent-title">Meridian OV</span>'));
});

test('a row click opens that project and the queue button queues it', () => {
  const { list, opened, queued, statuses } = setup();

  addRecentProject('/imps/Sol Levante', 'Sol Levante');
  list.querySelectorAll('.recent-item')[0].click();
  list.querySelectorAll('.recent-queue')[0].click();

  assert.deepEqual(opened, ['/imps/Sol Levante']);
  assert.deepEqual(queued, ['/imps/Sol Levante']);
  assert.deepEqual(statuses, ['Queued: /imps/Sol Levante']);
});

test('the section starts collapsed', () => {
  const { section, toggle } = setup();

  addRecentProject('/imps/A', 'A');

  assert.ok(section.classes.has('collapsed'));
  assert.equal(toggle.textContent, '▶');
  assert.equal(toggle.attributes['aria-expanded'], 'false');
});

test('a header click flips the collapsed state and stores it', () => {
  const { store, section, toggle, header } = setup();

  header.click();

  assert.equal(store.get(COLLAPSED_KEY), 'false');
  assert.ok(!section.classes.has('collapsed'));
  assert.equal(toggle.textContent, '▼');
  assert.equal(toggle.attributes['aria-expanded'], 'true');

  header.click();

  assert.equal(store.get(COLLAPSED_KEY), 'true');
  assert.ok(section.classes.has('collapsed'));
});

test('a quote or an angle bracket in a path stays text', () => {
  const { list } = setup();

  addRecentProject('/imps/Say "hi" <now>', '');

  assert.ok(list.innerHTML.includes('data-path="/imps/Say &quot;hi&quot; &lt;now&gt;"'));
  assert.ok(!list.innerHTML.includes('<now>'));
});
