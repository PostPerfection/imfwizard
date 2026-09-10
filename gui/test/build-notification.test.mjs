import assert from 'node:assert/strict';
import test from 'node:test';

import { notifyBuildComplete } from '../src/build-notification.js';

function fakeNotificationApi(permission) {
  class FakeNotification {
    static permission = permission;
    static requests = 0;
    static shown = [];

    static requestPermission() {
      FakeNotification.requests += 1;
    }

    constructor(title, options) {
      FakeNotification.shown.push({ title, ...options });
    }
  }
  return FakeNotification;
}

test('a finished build shows the success notification', () => {
  const api = fakeNotificationApi('granted');

  notifyBuildComplete(true, 'Sol Levante', api);

  assert.deepEqual(api.shown, [{ title: 'Build Complete', body: '"Sol Levante" built successfully' }]);
  assert.equal(api.requests, 0);
});

test('a failed build shows the failure notification', () => {
  const api = fakeNotificationApi('granted');

  notifyBuildComplete(false, 'Sol Levante', api);

  assert.deepEqual(api.shown, [{ title: 'Build Failed', body: '"Sol Levante" build failed' }]);
});

test('an undecided permission is asked for and nothing is shown', () => {
  const api = fakeNotificationApi('default');

  notifyBuildComplete(true, 'Sol Levante', api);

  assert.equal(api.requests, 1);
  assert.deepEqual(api.shown, []);
});

test('a refused permission is never asked again', () => {
  const api = fakeNotificationApi('denied');

  notifyBuildComplete(true, 'Sol Levante', api);

  assert.equal(api.requests, 0);
  assert.deepEqual(api.shown, []);
});
