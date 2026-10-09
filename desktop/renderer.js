import {applyLanguage, toggleLanguage, t, warningLabel, errorReason} from './i18n.js';
import {PageView} from './page-view.js';
import {ProofSlip} from './slip.js';
import {EditModel} from './edit-model.js';
import {changeCards} from './changes.js';
import {renderStructure} from './structure-view.js';

const $ = id => document.getElementById(id);
let documentData = null;
let model = null;
let busy = false;
let structure = false;
let substitutions = [];
let lastStatus = null;

function status(key, values = {}, error = false) {
  lastStatus = {key, values, error};
  $('status').textContent = t(key, values);
  $('status').classList.toggle('error', error);
}

async function call(method, ...args) {
  if (!window.hwp) {
    throw Error(t('bridge'));
  }
  return window.hwp[method](...args);
}

function hasUnsavedText() {
  return slip.dirty || Object.keys(model?.edits || {}).length > 0;
}

const slip = new ProofSlip($('slipHost'), (node, text) => {
  if (busy || !model.stage(node, text)) {
    return false;
  }
  refreshEdits();
  return true;
}, id => revert(id));

function selectNode(node, location) {
  if (busy || (slip.dirty && !confirm(t('discard')))) {
    return;
  }
  slip.open(node, model.current(node), documentData.format, location);
}

const pageView = new PageView({
  pages: $('pages'),
  thumbnails: $('thumbnails'),
  canvas: $('canvas'),
  onSelect: selectNode,
  onCounter: (page, total) => {
    $('pageCounter').textContent = `${page} / ${total}`;
  },
  onZoom: percent => {
    $('zoomValue').textContent = `${percent}%`;
    slip.position();
  },
  onFonts: fonts => {
    substitutions = fonts;
    renderFonts();
  },
  onFailure: () => {
    structure = true;
    status('previewFailed');
    $('structureChip').hidden = false;
    if (model) {
      renderFallback();
    }
  },
  translate: t
});

function renderFonts() {
  $('fontSubstitutions').replaceChildren(...substitutions.map(({from, to}) => {
    const row = document.createElement('div');
    row.className = 'font-substitution';
    row.textContent = `${from} → ${to}`;
    return row;
  }));
}

function warningCards() {
  return documentData.warnings.map(warning => {
    const card = document.createElement('div');
    card.className = `warning ${warning.severity}`;
    const code = document.createElement('span');
    code.className = 'code';
    code.textContent = warning.code;
    card.append(code, document.createTextNode(warningLabel(warning)));
    return card;
  });
}

function renderWarnings() {
  if (documentData) {
    $('warnings').replaceChildren(...warningCards());
  }
  renderFonts();
}

function updateControls() {
  const count = Object.keys(model?.edits || {}).length;
  $('dirty').hidden = !count;
  $('dirty').textContent = t(count === 1 ? 'edit' : 'edits', {n: count});
  $('changeCount').textContent = count;
  $('stagedCount').textContent = t('staged', {n: count});
  for (const id of ['save', 'review']) {
    $(id).disabled = !count || busy;
  }
  for (const id of ['open', 'emptyOpen', 'demo']) {
    $(id).disabled = busy;
  }
  $('confirmSave').disabled = !$('acknowledge').checked || busy;
}

function renderOutline() {
  $('outlineItems').replaceChildren(...documentData.nodes.map(node => {
    const button = document.createElement('button');
    button.className = 'outline-item';
    button.classList.toggle('edited', Object.hasOwn(model.edits, node.id));
    button.textContent = model.current(node) || t('empty');
    button.title = node.id;
    button.onclick = () => {
      const hit = [...$('pages').querySelectorAll('[data-node-id]')].find(element => element.dataset.nodeId === node.id);
      if (hit) {
        hit.scrollIntoView({block: 'center'});
        hit.click();
      } else if (!structure) {
        const entry = pageView.entries.find(entry => entry.mapped.some(item => item.node?.id === node.id));
        if (entry) {
          pageView.render(Number(entry.page.dataset.page));
          const target = [...entry.page.querySelectorAll('[data-node-id]')].find(element => element.dataset.nodeId === node.id);
          target?.scrollIntoView({block: 'center'});
          target?.click();
        }
      }
    };
    return button;
  }));
}

function renderFallback() {
  renderStructure(documentData, model, $('pages'), selectNode);
  $('thumbnails').replaceChildren();
  const pages = [...$('pages').children];
  pages.forEach((page, index) => {
    const button = document.createElement('button');
    button.className = 'thumb';
    const sheet = document.createElement('span');
    sheet.className = 'sheet';
    const number = document.createElement('span');
    number.className = 'n';
    number.textContent = String(index + 1).padStart(2, '0');
    button.setAttribute('aria-label', `${t('structure')} ${index + 1}`);
    button.append(sheet, number);
    button.onclick = () => page.scrollIntoView({block: 'start'});
    $('thumbnails').append(button);
  });
  $('pageCounter').textContent = `1 / ${pages.length}`;
  $('canvas').onscroll = () => {
    const top = $('canvas').getBoundingClientRect().top + 64;
    let nearest = 0;
    let distance = Infinity;
    pages.forEach((page, index) => {
      const difference = Math.abs(page.getBoundingClientRect().top - top);
      if (difference < distance) {
        nearest = index;
        distance = difference;
      }
    });
    $('pageCounter').textContent = `${nearest + 1} / ${pages.length}`;
  };
  zoom(pageView.zoom);
}

function refreshEdits() {
  $('changes').replaceChildren(...changeCards(model, revert));
  renderOutline();
  if (structure) {
    renderFallback();
  } else {
    pageView.updateEdits(model.edits);
  }
  updateControls();
  call('saveDraft', model.patch()).catch(error => status('failed', {reason: errorReason(error)}, true));
}

function revert(id) {
  if (busy || (slip.dirty && !confirm(t('discard')))) {
    return;
  }
  slip.close();
  model.revert(id);
  refreshEdits();
}

function historyAction(redo = false) {
  if (!model || busy || (slip.dirty && !confirm(t('discard')))) {
    return;
  }
  slip.close();
  if (redo) {
    model.redo();
  } else {
    model.undo();
  }
  refreshEdits();
}

async function renderRecent() {
  try {
    const recent = await call('recent');
    $('recentItems').replaceChildren(...recent.map((item, index) => {
      const button = document.createElement('button');
      button.className = 'doc-card';
      button.textContent = item.name;
      button.onclick = () => openDocument('openRecent', index);
      return button;
    }));
  } catch {
    $('recentItems').replaceChildren();
  }
}

function beginOpen() {
  if (busy || (hasUnsavedText() && !confirm(t('discard')))) {
    return false;
  }
  busy = true;
  updateControls();
  status('busy');
  return true;
}

async function completeOpen(result, failure = null) {
  try {
    if (failure) {
      throw Error(failure);
    }
    if (!result) {
      $('status').textContent = '';
      lastStatus = null;
      return;
    }
    slip.close(false);
    documentData = result.document;
    model = new EditModel(documentData);
    structure = false;
    substitutions = [];
    $('filename').textContent = documentData.name;
    $('format').hidden = false;
    $('format').textContent = documentData.format.toUpperCase();
    $('empty').hidden = true;
    $('workspace').hidden = false;
    $('structureChip').hidden = true;
    if (result.draft?.baseSha256 === documentData.sha256 && result.draft.operations?.length && confirm(t('restore'))) {
      model.restore(result.draft);
    }
    renderOutline();
    renderWarnings();
    const bytes = await call('documentBytes');
    const rendered = await pageView.open(bytes, documentData.nodes);
    if (rendered) {
      pageView.updateEdits(model.edits);
      $('status').textContent = '';
      lastStatus = null;
    }
    $('changes').replaceChildren(...changeCards(model, revert));
    renderRecent();
  } catch (error) {
    status('openFailed', {reason: errorReason(error)}, true);
    if (documentData) {
      pageView.clear();
      structure = true;
      $('structureChip').hidden = false;
      renderFallback();
    }
  } finally {
    busy = false;
    updateControls();
  }
}

async function openDocument(method = 'open', ...args) {
  if (!beginOpen()) {
    return;
  }
  try {
    await completeOpen(await call(method, ...args));
  } catch (error) {
    await completeOpen(null, error.message);
  }
}

async function review() {
  if (!model || busy || !Object.keys(model.edits).length) {
    return;
  }
  if (slip.dirty) {
    status('unstaged', {}, true);
    return;
  }
  slip.close();
  busy = true;
  updateControls();
  try {
    await call('plan', model.patch());
    $('reviewChanges').replaceChildren(...changeCards(model));
    $('reviewWarnings').replaceChildren(...warningCards());
    $('acknowledge').checked = false;
    $('reviewDialog').showModal();
  } catch (error) {
    status('failed', {reason: errorReason(error)}, true);
  } finally {
    busy = false;
    updateControls();
  }
}

async function save() {
  if (busy || !$('acknowledge').checked) {
    return;
  }
  busy = true;
  updateControls();
  try {
    const result = await call('saveCopy', model.patch(), true);
    if (result) {
      $('reviewDialog').close();
      status('saved', {name: result.name});
      const finder = document.createElement('button');
      finder.className = 'btn ghost';
      finder.textContent = t('showInFinder');
      finder.onclick = () => call('showSaved').catch(error => status('failed', {reason: errorReason(error)}, true));
      $('status').append(finder);
    }
  } catch (error) {
    status('failed', {reason: errorReason(error)}, true);
  } finally {
    busy = false;
    updateControls();
  }
}

function zoom(value) {
  const percent = pageView.setZoom(value);
  $('zoomValue').textContent = `${percent}%`;
  if (structure) {
    $('pages').querySelectorAll('.structure-page').forEach(page => {
      page.style.zoom = percent / 100;
    });
  }
  slip.position();
}

function railMode(outline) {
  document.body.classList.toggle('outline-mode', outline);
  document.querySelector('.rail').classList.toggle('outline-mode', outline);
  $('outlineItems').hidden = !outline;
  $('thumbnails').hidden = outline;
  for (const [id, active] of [['pagesToggle', !outline], ['outlineToggle', outline]]) {
    $(id).classList.toggle('on', active);
    $(id).setAttribute('aria-pressed', String(active));
  }
}

function panelTab(name) {
  for (const item of ['changes', 'agent', 'checks']) {
    $(`${item}Panel`).hidden = item !== name;
    $(`${item}Tab`).classList.toggle('on', item === name);
    $(`${item}Tab`).setAttribute('aria-selected', String(item === name));
    $(`${item}Tab`).tabIndex = item === name ? 0 : -1;
  }
}

$('open').onclick = () => openDocument();
$('emptyOpen').onclick = () => openDocument();
$('demo').onclick = () => openDocument('openDemo');
$('save').onclick = review;
$('review').onclick = review;
$('confirmSave').onclick = save;
$('acknowledge').onchange = updateControls;
$('closeReview').onclick = () => $('reviewDialog').close();
$('cancelReview').onclick = () => $('reviewDialog').close();
$('pagesToggle').onclick = () => railMode(false);
$('outlineToggle').onclick = () => railMode(true);
$('zoomIn').onclick = () => zoom(pageView.stepZoom(1));
$('zoomOut').onclick = () => zoom(pageView.stepZoom(-1));
$('zoomValue').onclick = () => zoom('fit');
$('language').onclick = () => {
  toggleLanguage();
  updateControls();
  slip.relabel();
  pageView.relabel();
  if (documentData) {
    renderOutline();
    renderWarnings();
    if (structure) {
      $('pages').querySelectorAll('.opaque').forEach(element => {
        element.textContent = t('preserved');
      });
      $('pages').querySelectorAll('.text-node').forEach(element => {
        const node = model.nodes.get(element.dataset.nodeId);
        element.textContent = model.current(node) || t('empty');
      });
      [...$('thumbnails').children].forEach((button, index) => {
        button.setAttribute('aria-label', `${t('structure')} ${index + 1}`);
      });
    }
    $('changes').replaceChildren(...changeCards(model, revert));
    $('reviewChanges').replaceChildren(...changeCards(model));
    $('reviewWarnings').replaceChildren(...warningCards());
  }
  if (lastStatus) {
    status(lastStatus.key, lastStatus.values, lastStatus.error);
  }
};
for (const name of ['changes', 'agent', 'checks']) {
  $(`${name}Tab`).onclick = () => panelTab(name);
}
document.querySelector('.tabs').onkeydown = event => {
  const tabs = ['changes', 'agent', 'checks'];
  const current = tabs.findIndex(name => $(`${name}Tab`) === document.activeElement);
  if (['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) {
    event.preventDefault();
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? 2 : (current + (event.key === 'ArrowRight' ? 1 : 2)) % 3;
    panelTab(tabs[index]);
    $(`${tabs[index]}Tab`).focus();
  }
};

document.addEventListener('keydown', event => {
  if (!(event.metaKey || event.ctrlKey) || event.isComposing) {
    return;
  }
  const key = event.key.toLowerCase();
  if (key === 'o') {
    event.preventDefault();
    openDocument();
  } else if (key === 's') {
    event.preventDefault();
    review();
  } else if (key === 'z' && document.activeElement !== slip.editor) {
    event.preventDefault();
    historyAction(event.shiftKey);
  } else if (['+', '=', '-', '0'].includes(key) && document.activeElement !== slip.editor) {
    event.preventDefault();
    zoom(key === '0' ? 'fit' : pageView.stepZoom(key === '-' ? -1 : 1));
  }
});
window.addEventListener('resize', () => slip.position());
window.addEventListener('beforeunload', event => {
  if (hasUnsavedText()) {
    event.preventDefault();
    event.returnValue = '';
  }
});
// Preload obtains a dropped file's path only from a trusted OS event.
// The renderer accepts the switch before main changes the active document.
window.hwp?.onDrop(beginOpen, completeOpen);
// A drop never navigates to local document content.
document.addEventListener('dragover', event => event.preventDefault());
document.addEventListener('drop', event => {
  event.preventDefault();
});
applyLanguage();
panelTab('changes');
updateControls();
renderRecent();
