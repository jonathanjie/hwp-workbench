import {test} from 'node:test';
import assert from 'node:assert/strict';
import {PageView} from '../page-view.js';

test('thumbnail rendering advances one page per timer turn without idle callbacks', async () => {
  const view = Object.create(PageView.prototype);
  view.entries = [{url: null}, {url: null}];
  const rendered = [];
  const failures = [];
  view.onFailure = error => failures.push(error);
  let complete;
  const finished = new Promise(resolve => { complete = resolve; });
  view.render = index => {
    rendered.push(index);
    view.entries[index].url = `page-${index}`;
    if (rendered.length === 2) {
      complete();
    }
  };
  try {
    view.scheduleThumbnails();
    assert.deepEqual(rendered, []);
    await finished;
    assert.deepEqual(rendered, [0, 1]);
    assert.deepEqual(failures, []);
  } finally {
    view.resize = {disconnect() {}};
    view.suspend();
  }
});

test('suspending a page view cancels its pending thumbnail timer', async () => {
  const view = Object.create(PageView.prototype);
  view.entries = [{url: null}];
  view.resize = {disconnect() {}};
  let rendered = false;
  view.render = () => { rendered = true; };
  view.onFailure = error => { throw error; };
  view.scheduleThumbnails();
  view.suspend();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(rendered, false);
  assert.equal(view.idleRender, null);
});
