import {applyLanguage, toggleLanguage, t, warningLabel, errorReason} from './i18n.js';
import {PageView} from './page-view.js';
import {ProofSlip} from './slip.js';
import {EditModel} from './edit-model.js';
import {changeCards} from './changes.js';
import {headingNodes, nodeLocations} from './navigation.js';
import {renderChecks, compatibilityTags} from './checks.js';
import {Toast} from './toast.js';
import {FindBar} from './find-bar.js';
import {findMatches} from './search.js';
import {CommandPalette} from './palette.js';
import {element} from './dom.js';
import {AgentsCard} from './agents-card.js';
import {AgentPanel} from './agent-panel.js';
import {textError} from './edit-model.js';
import {previewNodes, commitPagePreview} from './preview.js';
import {PanelZoom} from './panel-zoom.js';
import {renderStructure} from './structure-view.js';

const $ = id => document.getElementById(id);
const toast = new Toast($('toast'));
let reviewFocus;
let documentData = null;
let model = null;
let busy = false;
let structure = false;
let substitutions = [];
let locations = new Map();
let lastStatus = null;
let activeTab = 'changes';
let panelDismissed = false;

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
  panelDismissed = false;
  refreshEdits();
  stagedToast(1);
  return true;
}, id => revert(id));

function selectNode(node, location) {
  if (busy || (slip.dirty && !confirm(t('discard')))) {
    return;
  }
  node = model.nodes.get(node.id);
  call('agentSelection', {nodeId: node.id, text: model.current(node)}).catch(error => status('failed', {reason: errorReason(error)}, true));
  slip.open(node, model.current(node), documentData.format, location, model.lengthChangingText);
}

function pageOptions(pages = $('pages'), thumbnails = $('thumbnails')) {
  return {
  pages,
  thumbnails,
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
    pageView.clear();
    structure = true;
    status('previewFailed');
    $('structureChip').hidden = false;
    if (model) {
      renderFallback();
    }
  },
  translate: t
  };
}
let pageView = new PageView(pageOptions());

const panelZoom = new PanelZoom(pageView);

const agentsCard = new AgentsCard([$('welcomeAgents'), $('panelAgents')], call, toast, error => status('failed', {reason: errorReason(error)}, true));

const agentPanel = new AgentPanel({
  host: $('agentInbox'), switchControl: $('agentLinkSwitch'), chip: $('agentChip'), toast,
  toggle: async enabled => {
    try {
      agentPanel.update(await call('agentLink', enabled), false);
    } catch (error) {
      agentPanel.render();
      status('agentLinkFailed', {}, true);
    }
  },
  decide: decideProposal
});

async function decideProposal(file, ids, accepted) {
  if (busy || (slip.dirty && !confirm(t('discard')))) {
    return;
  }
  const row = agentPanel.state.proposals.find(row => row.file === file);
  const changes = (row?.patch?.operations || []).filter(operation => ids.includes(operation.nodeId));
  if (accepted && changes.some(operation => {
    const node = model.nodes.get(operation.nodeId);
    return !node?.editable || operation.expectedText !== node.text || Object.hasOwn(model.edits, node.id) ||
      textError(operation.text, node.text, model.lengthChangingText);
  })) {
    status('proposalConflict', {}, true);
    return;
  }
  if (accepted && new Set([...Object.keys(model.edits), ...ids]).size > 1000) {
    status('tooManyEdits', {}, true);
    return;
  }
  slip.close();
  busy = true;
  agentPanel.busy = true;
  agentPanel.render();
  updateControls();
  try {
    const result = await call('agentDecide', file, ids, accepted);
    if (result.operations.length) {
      model.stageBatch(result.operations.map(operation => ({node: model.nodes.get(operation.nodeId), text: operation.text, author: result.author})));
      panelDismissed = false;
      refreshEdits();
    }
  } catch (error) {
    status('failed', {reason: errorReason(error)}, true);
  } finally {
    busy = false;
    agentPanel.busy = false;
    agentPanel.render();
    updateControls();
  }
}

$('agentChip').onclick = () => {
  panelTab('agent');
  updatePanel();
};
window.hwp?.onAgentInbox(state => {
  if (documentData && !busy) {
    agentPanel.update(state);
  } else if (documentData) {
    agentPanel.update(state, false);
  }
});

function renderFonts() {
  $('fontSubstitutions').replaceChildren(...substitutions.map(({from, to}) => {
    const row = document.createElement('div');
    row.className = 'font-substitution';
    row.textContent = `${from} → ${to}`;
    return row;
  }));
}

function renderWarnings() {
  if (documentData) {
    renderChecks(documentData, $('checkTags'), $('warnings'));
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
  $('save').hidden = !documentData;
  updatePanel();
  for (const id of ['emptyOpen', 'demo']) {
    $(id).disabled = busy;
  }
  $('confirmSave').disabled = !$('acknowledge').checked || !count || busy;
}

function renderOutline() {
  const headings = headingNodes(documentData.nodes);
  $('outlineItems').replaceChildren(...headings.map(({node, nodes}) => {
    const button = element('button', 'outline-item', nodes.map(item => model.current(item)).join(''));
    button.classList.toggle('edited', nodes.some(item => Object.hasOwn(model.edits, item.id)));
    button.title = nodeLocation(node);
    button.onclick = () => locateNode(node, false);
    return button;
  }));
  if (!headings.length) {
    $('outlineItems').append(element('div', 'empty-outline', t('noHeadings')));
  }
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
    pageView.updateEdits(model.edits, documentData.nodes, model.origins);
  }
  updateControls();
  findBar.refresh();
  if ($('reviewDialog').open) {
    $('acknowledge').checked = false;
    renderReview();
    updateControls();
  }
  schedulePreview();
  call('saveDraft', model.patch()).catch(error => status('failed', {reason: errorReason(error)}, true));
}

let previewTimer;
let previewRevision = 0;

function schedulePreview() {
  const revision = ++previewRevision;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(() => renderPreview(revision), 300);
}

async function renderPreview(revision) {
  const activeModel = model;
  const patch = activeModel.patch();
  let candidate;
  try {
    const result = patch.operations.length ? await call('preview', patch) : null;
    const bytes = result ? Uint8Array.from(atob(result.bytesBase64), character => character.charCodeAt(0)) : await call('documentBytes');
    if (revision !== previewRevision || activeModel !== model) {
      return;
    }
    const scrollTop = $('canvas').scrollTop;
    const scrollLeft = $('canvas').scrollLeft;
    const canvasBounds = $('canvas').getBoundingClientRect();
    const visible = pageView.entries.flatMap((entry, index) => {
      const bounds = entry.page.getBoundingClientRect();
      return bounds.bottom >= canvasBounds.top && bounds.top <= canvasBounds.bottom ? [index] : [];
    });
    candidate = new PageView(pageOptions(element('div'), element('div')));
    candidate.zoom = pageView.zoom;
    candidate.fit = pageView.fit;
    await candidate.open(bytes, previewNodes(documentData.nodes, activeModel.edits), {detached: true, visible: visible.length ? visible : [0]});
    await Promise.all(candidate.entries.flatMap(entry => entry.url ? [entry.page.querySelector('.page-image').decode()] : []));
    if (revision !== previewRevision || activeModel !== model) {
      candidate.clear();
      return;
    }
    candidate.updateEdits(activeModel.edits, documentData.nodes, activeModel.origins);
    const previous = pageView;
    const previousStructure = structure;
    previous.onFailure = pageOptions().onFailure;
    $('previewStatus').textContent = t('preview');
    commitPagePreview(previous, candidate, {
      pages: $('pages'), thumbnails: $('thumbnails'),
      onMount: view => {
        pageView = view;
        panelZoom.view = view;
        globalThis.measureTextWidth = view.fonts.measure;
        structure = view === previous ? previousStructure : false;
        $('structureChip').hidden = !structure;
        $('canvas').scrollTop = scrollTop;
        $('canvas').scrollLeft = scrollLeft;
        reanchorSlip();
        findBar.refresh();
      },
      onRollback: error => previewFailure(error)
    });
  } catch (error) {
    candidate?.clear();
    if (revision === previewRevision) {
      previewFailure(error);
    }
  }
}

function previewFailure(error) {
  $('previewStatus').textContent = t('livePreviewFailed');
  status('failed', {reason: errorReason(error)}, true);
}

function reanchorSlip() {
  if (!slip.slip) {
    return;
  }
  for (const entry of pageView.entries) {
    const mapped = entry.mapped.find(item => item.node?.id === slip.node.id);
    if (!mapped) {
      continue;
    }
    pageView.render(Number(entry.page.dataset.page));
    const group = entry.groups.find(group => group.runs.includes(mapped.run));
    const focus = [...entry.hits.children].find(hit => hit.dataset.nodeId === slip.node.id);
    slip.location = {page: entry.page, bounds: group.bounds, width: entry.width, height: entry.height, focus, paragraph: mapped.run.paraIdx};
    slip.position();
    return;
  }
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
    $('recentRow').hidden = recent.length === 0;
    $('recentItems').replaceChildren(...recent.map(item => {
      const button = document.createElement('button');
      button.className = 'doc-card';
      const mini = element('span', 'mini');
      if (item.thumbnail) {
        const image = element('img');
        image.src = item.thumbnail;
        image.alt = '';
        mini.append(image);
      }
      const meta = element('span', 'meta');
      meta.append(element('span', 'name', item.name), element('span', 'sub', item.format.toUpperCase()));
      button.append(mini, meta);
      button.onclick = () => openDocument('openRecent', item.id);
      return button;
    }));
  } catch {
    $('recentItems').replaceChildren();
    $('recentRow').hidden = true;
    status('recentFailed', {}, true);
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
    previewRevision += 1;
    clearTimeout(previewTimer);
    if ($('reviewDialog').open) {
      $('reviewDialog').close();
    }
    slip.close(false);
    findBar.close();
    documentData = result.document;
    agentsCard.refresh();
    model = new EditModel(documentData);
    locations = nodeLocations(documentData.view);
    agentPanel.reset(result.agentLink);
    activeTab = 'changes';
    panelDismissed = false;
    panelTab('changes');
    toast.clear();
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
      pageView.updateEdits(model.edits, documentData.nodes, model.origins);
      $('status').textContent = '';
      lastStatus = null;
    }
    if (Object.keys(model.edits).length) {
      schedulePreview();
    }
    $('changes').replaceChildren(...changeCards(model, revert));
    if (rendered && result.recentId) {
      try {
        const png = await pageView.thumbnail();
        if (png) {
          await call('cacheThumbnail', result.recentId, png);
        }
      } catch {
        status('thumbnailFailed');
      }
    }
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

function stagedToast(count, paragraphs) {
  const history = model.history.length;
  toast.show(paragraphs === undefined ? 'stagedToast' : 'replacedToast', {n: count, paragraphs}, {label: 'undo', run: () => {
    if (model.history.length === history) {
      historyAction();
    }
  }});
}

function pageForNode(id) {
  if (!structure) {
    const index = pageView.entries.findIndex(entry => entry.mapped.some(item => item.node?.id === id));
    return index === -1 ? null : index + 1;
  }
  const target = [...$('pages').querySelectorAll('[data-node-id]')].find(element => element.dataset.nodeId === id);
  return target ? [...$('pages').children].indexOf(target.closest('.page')) + 1 : null;
}

function renderReview() {
  const groups = new Map();
  for (const id of Object.keys(model.edits)) {
    const page = pageForNode(id);
    if (!groups.has(page)) {
      groups.set(page, []);
    }
    groups.get(page).push(id);
  }
  $('reviewChanges').replaceChildren(...[...groups].sort((a, b) => (a[0] ?? Infinity) - (b[0] ?? Infinity)).map(([page, ids]) => {
    const group = element('section', 'review-group');
    group.append(element('div', 'section-label', page ? t('pageLabel', {n: page}) : t('unlocated')));
    group.append(...changeCards(model, revert, ids, 'remove'));
    return group;
  }));
  $('reviewSummary').textContent = t('reviewSummary', {n: Object.keys(model.edits).length, pages: pageView.entries.length || $('pages').children.length});
  $('reviewWarnings').replaceChildren(...compatibilityTags(documentData));
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
    renderReview();
    $('acknowledge').checked = false;
    reviewFocus = document.activeElement;
    $('reviewDialog').showModal();
  } catch (error) {
    status('failed', {reason: errorReason(error)}, true);
  } finally {
    busy = false;
    updateControls();
  }
}

async function save() {
  if (busy || !$('acknowledge').checked || !Object.keys(model?.edits || {}).length || !$('reviewDialog').open) {
    return;
  }
  busy = true;
  updateControls();
  try {
    const result = await call('saveCopy', model.patch(), true);
    if (result) {
      $('reviewDialog').close();
      toast.show('saved', {name: result.name}, {
        label: 'showInFinder',
        run: () => call('showSaved').catch(error => status('failed', {reason: errorReason(error)}, true))
      });
      $('status').textContent = '';
      lastStatus = null;
    }
  } catch (error) {
    status('failed', {reason: errorReason(error)}, true);
  } finally {
    busy = false;
    updateControls();
  }
}

function zoom(value) {
  panelZoom.changed();
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
  activeTab = name;
  panelDismissed = false;
  for (const item of ['changes', 'agent', 'checks']) {
    $(`${item}Panel`).hidden = item !== name;
    $(`${item}Tab`).classList.toggle('on', item === name);
    $(`${item}Tab`).setAttribute('aria-selected', String(item === name));
    $(`${item}Tab`).tabIndex = item === name ? 0 : -1;
  }
}

function updatePanel() {
  const content = Object.keys(model?.edits || {}).length > 0 || ['checks', 'agent'].includes(activeTab);
  const visible = !!documentData && content && !panelDismissed;
  $('panel').hidden = !visible;
  document.body.classList.toggle('panel-open', visible);
  panelZoom.update(visible);
  slip.position();
}

function togglePanel() {
  if (!documentData) {
    return;
  }
  if (activeTab !== 'checks' && !Object.keys(model.edits).length) {
    panelTab('checks');
  } else {
    panelDismissed = !$('panel').hidden;
  }
  updatePanel();
}

function locateNode(node, openSlip = true) {
  if (!model) {
    return;
  }
  let target;
  if (!structure) {
    const index = pageView.entries.findIndex(entry => entry.mapped.some(item => item.node?.id === node.id));
    if (index !== -1) {
      pageView.render(index);
      target = [...pageView.entries[index].page.querySelectorAll('[data-node-id]')].find(hit => hit.dataset.nodeId === node.id);
    }
  } else {
    target = [...$('pages').querySelectorAll('[data-node-id]')].find(hit => hit.dataset.nodeId === node.id);
  }
  if (target) {
    target.scrollIntoView({block: 'center'});
    if (openSlip) {
      target.click();
    }
  }
}

function nodeLocation(node) {
  return locations.get(node.id) || `P${node.anchor ? node.anchor.paragraph + 1 : node.paragraph || 1}`;
}

function paletteTextResults(query) {
  return findMatches(model, query).slice(0, 100).map(match => ({...match, location: nodeLocation(match.node)}));
}

function highlightMatches(matches, active) {
  pageView.updateSearch(matches, active);
  if (!structure || !model) {
    return;
  }
  for (const span of $('pages').querySelectorAll('.text-node')) {
    const node = model.nodes.get(span.dataset.nodeId);
    const text = model.current(node);
    const ranges = matches.filter(match => match.node.id === node.id);
    span.replaceChildren();
    let offset = 0;
    for (const match of ranges) {
      span.append(document.createTextNode(text.slice(offset, match.start)));
      const mark = element('mark', matches[active] === match ? 'active' : '', text.slice(match.start, match.end));
      span.append(mark);
      offset = match.end;
    }
    span.append(document.createTextNode(text.slice(offset) || (!text ? t('empty') : '')));
  }
}

const findBar = new FindBar($('findBar'), () => busy ? null : model, {
  beforeReplace: () => {
    if (busy) {
      return false;
    }
    if (slip.dirty) {
      status('unstaged', {}, true);
      return false;
    }
    slip.close();
    return true;
  },
  onHighlight: highlightMatches,
  onLocate: match => locateNode(match.node, false),
  onStaged: (count, paragraphs) => {
    panelDismissed = false;
    refreshEdits();
    stagedToast(count, paragraphs);
  },
  location: nodeLocation
});

function openFind(replace = false, query) {
  if (!documentData || busy) {
    return;
  }
  if (slip.dirty) {
    status('unstaged', {}, true);
    return;
  }
  slip.close();
  findBar.open(replace, query);
}

function setTheme(theme) {
  if (!['system', 'light', 'dark'].includes(theme)) {
    theme = 'system';
  }
  if (theme === 'system') {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = theme;
  }
  localStorage.setItem('hwp-theme', theme);
}

function shortcuts() {
  if ($('shortcutsDialog').open) {
    return;
  }
  const entries = [
    {label: 'commands', shortcut: '⌘K'},
    ...commandList.filter(command => command.shortcut),
    {label: 'togglePanel', shortcut: '⌥⌘I'},
    {label: 'stageShortcut', shortcut: '⌘↵'},
    {label: 'closeShortcut', shortcut: 'Esc'},
    {label: 'findNextShortcut', shortcut: '↵'},
    {label: 'findPreviousShortcut', shortcut: '⇧↵'},
    {label: 'paletteScopes', shortcut: 'Tab / ⇧Tab'},
    {label: 'paletteMove', shortcut: '↑ / ↓'},
    {label: 'paletteRun', shortcut: '↵'},
    {label: 'panelTabs', shortcut: '← / → / Home / End'}
  ];
  $('shortcutItems').replaceChildren(...entries.map(command => {
    const row = element('div', 'shortcut');
    row.append(element('span', '', t(command.label)), element('kbd', '', command.shortcut));
    return row;
  }));
  shortcutFocus = document.activeElement;
  $('shortcutsDialog').showModal();
}

let shortcutFocus;
const needsDocument = () => !!documentData && !busy;
const commandList = [
  {label: 'open', shortcut: '⌘O', run: () => openDocument()},
  {label: 'sample', run: () => openDocument('openDemo')},
  {label: 'saveCopy', shortcut: '⌘S', enabled: () => needsDocument() && Object.keys(model.edits).length > 0, run: review},
  {label: 'find', shortcut: '⌘F', enabled: needsDocument, run: () => openFind()},
  {label: 'replace', shortcut: '⌥⌘F', enabled: needsDocument, run: () => openFind(true)},
  {label: 'undo', shortcut: '⌘Z', enabled: () => needsDocument() && model.history.length > 0, run: () => historyAction()},
  {label: 'redo', shortcut: '⇧⌘Z', enabled: () => needsDocument() && model.future.length > 0, run: () => historyAction(true)},
  {label: 'zoomIn', shortcut: '⌘+', enabled: needsDocument, run: () => zoom(pageView.stepZoom(1))},
  {label: 'zoomOut', shortcut: '⌘−', enabled: needsDocument, run: () => zoom(pageView.stepZoom(-1))},
  {label: 'fitWidth', shortcut: '⌘0', enabled: needsDocument, run: () => zoom('fit')},
  {label: 'pages', enabled: needsDocument, run: () => railMode(false)},
  {label: 'outline', enabled: needsDocument, run: () => railMode(true)},
  ...['changes', 'agent', 'checks'].map(name => ({label: name, enabled: needsDocument, run: () => {
    panelTab(name);
    updatePanel();
  }})),
  {label: 'language', run: changeLanguage},
  ...['system', 'light', 'dark'].map(theme => ({label: `theme${theme[0].toUpperCase()}${theme.slice(1)}`, run: () => setTheme(theme)})),
  {label: 'shortcuts', shortcut: '⌘/', run: shortcuts}
];
const palette = new CommandPalette($('palette'), commandList, paletteTextResults, match => locateNode(match.node));
$('paletteButton').onclick = () => palette.open();
document.querySelector('.island').onclick = event => {
  if (!event.target.closest('button')) {
    palette.open();
  }
};
$('closeShortcuts').onclick = () => $('shortcutsDialog').close();
$('shortcutsDialog').addEventListener('close', () => shortcutFocus?.focus({preventScroll: true}));
setTheme(localStorage.getItem('hwp-theme') || 'system');

$('dirty').onclick = () => {
  panelTab('changes');
  updatePanel();
};
$('emptyOpen').onclick = () => openDocument();
$('demo').onclick = () => openDocument('openDemo');
$('save').onclick = review;
$('review').onclick = review;
$('confirmSave').onclick = save;
$('acknowledge').onchange = updateControls;
$('closeReview').onclick = () => $('reviewDialog').close();
$('cancelReview').onclick = () => $('reviewDialog').close();
$('reviewDialog').addEventListener('close', () => reviewFocus?.focus({preventScroll: true}));
$('pagesToggle').onclick = () => railMode(false);
$('outlineToggle').onclick = () => railMode(true);
$('zoomIn').onclick = () => zoom(pageView.stepZoom(1));
$('zoomOut').onclick = () => zoom(pageView.stepZoom(-1));
$('zoomValue').onclick = () => zoom('fit');
function changeLanguage() {
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
  }
  toast.render();
  agentPanel.render();
  agentsCard.render();
  findBar.refresh();
  if ($('reviewDialog').open) {
    renderReview();
  }
  if (lastStatus) {
    status(lastStatus.key, lastStatus.values, lastStatus.error);
  }
  if (palette.dialog.open) {
    palette.render();
  }
}
for (const name of ['changes', 'agent', 'checks']) {
  $(`${name}Tab`).onclick = () => {
    panelTab(name);
    updatePanel();
  };
}
document.querySelector('.tabs').onkeydown = event => {
  const tabs = ['changes', 'agent', 'checks'];
  const current = tabs.findIndex(name => $(`${name}Tab`) === document.activeElement);
  if (['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) {
    event.preventDefault();
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? 2 : (current + (event.key === 'ArrowRight' ? 1 : 2)) % 3;
    panelTab(tabs[index]);
    updatePanel();
    $(`${tabs[index]}Tab`).focus();
  }
};

document.addEventListener('keydown', event => {
  if (!(event.metaKey || event.ctrlKey) || event.isComposing) {
    return;
  }
  const key = event.code.startsWith('Key') ? event.code.slice(3).toLowerCase() : event.code === 'Slash' ? '/' : event.key.toLowerCase();
  if ($('reviewDialog').open || $('shortcutsDialog').open || (palette.dialog.open && key !== 'k')) {
    return;
  }
  const typing = document.activeElement?.matches('input, textarea, [contenteditable]');
  if (key === 'k') {
    event.preventDefault();
    palette.dialog.open ? palette.close() : palette.open();
  } else if (key === 'f') {
    event.preventDefault();
    openFind(event.altKey);
  } else if (key === '/') {
    event.preventDefault();
    shortcuts();
  } else if (key === 'i' && event.altKey) {
    event.preventDefault();
    togglePanel();
  } else if (key === 'o') {
    event.preventDefault();
    openDocument();
  } else if (key === 's') {
    event.preventDefault();
    review();
  } else if (key === 'z' && !typing) {
    event.preventDefault();
    historyAction(event.shiftKey);
  } else if (['+', '=', '-', '0'].includes(key) && !typing) {
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
window.hwp?.onHandoff(async (result, error) => {
  busy = true;
  updateControls();
  await completeOpen(result, error);
  await call('handoffReady');
});
document.addEventListener('input', event => {
  if (event.target.id === 'textEdit') {
    call('editing', slip.dirty).catch(() => {});
  }
});
$('slipHost').addEventListener('click', () => {
  call('editing', slip.dirty).catch(() => {});
});

// Preload obtains a dropped file's path only from a trusted OS event.
// The renderer accepts the switch before main changes the active document.
window.hwp?.onDrop(beginOpen, completeOpen);
window.hwp?.onCommand(command => {
  if (command === 'language') {
    changeLanguage();
  } else if (command === 'palette') {
    palette.open();
  } else if (command === 'shortcuts') {
    shortcuts();
  } else if (command === 'undo' || command === 'redo') {
    if (document.activeElement?.matches('input, textarea, [contenteditable]')) {
      document.execCommand(command);
    } else {
      historyAction(command === 'redo');
    }
  } else {
    openFind(command === 'replace');
  }
});
// A drop never navigates to local document content.
let dragDepth = 0;
document.addEventListener('dragenter', event => {
  if (event.dataTransfer?.types.includes('Files')) {
    event.preventDefault();
    dragDepth += 1;
    $('dropSheet').classList.add('over');
  }
});
document.addEventListener('dragleave', () => {
  dragDepth = Math.max(0, dragDepth - 1);
  if (!dragDepth) {
    $('dropSheet').classList.remove('over');
  }
});
document.addEventListener('dragover', event => {
  event.preventDefault();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'copy';
  }
});
document.addEventListener('drop', event => {
  event.preventDefault();
  dragDepth = 0;
  $('dropSheet').classList.remove('over');
});
applyLanguage();
panelTab('changes');
updateControls();
renderRecent();

agentsCard.refresh();
