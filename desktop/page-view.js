import {controlMarks, inlineFieldMarks} from './control-marks.js';
import {changedRanges} from './preview.js';
import {matchBounds} from './search-layout.js';
import {createFontMapper} from './fonts.js';
import {mapTextRuns, paragraphBounds} from './hit-map.js';
import {fitZoom} from './panel-zoom.js';

const VENDOR = 'app://workbench/vendor/rhwp/';
const ZOOM_STEPS = [50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200];
let runtime;

function rectangle(element, bounds, width, height) {
  element.style.left = `${bounds.x / width * 100}%`;
  element.style.top = `${bounds.y / height * 100}%`;
  element.style.width = `${bounds.w / width * 100}%`;
  element.style.height = `${bounds.h / height * 100}%`;
}

export class PageView {
  constructor({pages, thumbnails, canvas, onSelect, onCounter, onFonts, onFailure, onZoom, translate}) {
    this.pages = pages;
    this.thumbnails = thumbnails;
    this.canvas = canvas;
    this.onSelect = onSelect;
    this.onCounter = onCounter;
    this.onZoom = onZoom;
    this.onFonts = onFonts;
    this.onFailure = onFailure;
    this.translate = translate;
    this.zoom = 100;
    this.fit = false;
    this.controlMarksVisible = false;
    this.entries = [];
    this.resize = new ResizeObserver(() => {
      if (this.fit) {
        this.setZoom('fit');
      }
    });
    this.resize.observe(canvas);
  }

  suspend() {
    this.observer?.disconnect();
    this.resize.disconnect();
    if (this.idleRender) {
      cancelIdleCallback(this.idleRender);
      this.idleRender = null;
    }
  }

  clear(clearDom = true) {
    this.suspend();
    this.releasePrevious?.();
    this.releasePrevious = null;
    for (const entry of this.entries) {
      if (entry.url) {
        URL.revokeObjectURL(entry.url);
      }
    }
    this.document?.free();
    this.document = null;
    this.entries = [];
    if (clearDom) {
      this.pages.replaceChildren();
      this.thumbnails.replaceChildren();
    }
  }

  async open(bytes, nodes, {detached = false, visible = [0], requestedFonts = []} = {}) {
    this.clear();
    this.detached = detached;
    this.resize.observe(this.canvas);
    this.nodes = nodes;
    this.edits = {};
    this.searchMatches = [];
    this.searchActive = -1;
    this.fonts = createFontMapper(this.onFonts);
    this.fonts.requested(requestedFonts);
    globalThis.measureTextWidth = this.fonts.measure;
    runtime ??= import(`${VENDOR}rhwp.js`).then(async module => {
      await module.default({module_or_path: `${VENDOR}rhwp_bg.wasm`});
      return module;
    }).catch(error => {
      runtime = null;
      throw error;
    });
    try {
      const {HwpDocument} = await runtime;
      this.document = new HwpDocument(new Uint8Array(bytes));
      const count = this.document.pageCount();
      if (!count) {
        throw Error('No rendered pages');
      }
      // Read identities for every page so repeated text on later pages keeps
      // its document-order mapping. SVG rendering remains lazy.
      const layouts = Array.from({length: count}, (_, index) => JSON.parse(this.document.getPageTextLayout(index)).runs);
      const allRuns = layouts.flat();
      for (let index = 0; index < count; index += 1) {
        const page = document.createElement('article');
        page.className = 'page rendered-page';
        page.tabIndex = -1;
        page.dataset.page = index;
        const thumbnail = document.createElement('button');
        thumbnail.className = 'thumb';
        thumbnail.setAttribute('aria-label', `${this.translate('pages')} ${index + 1}`);
        const sheet = document.createElement('span');
        sheet.className = 'sheet';
        const number = document.createElement('span');
        number.className = 'n';
        number.textContent = String(index + 1).padStart(2, '0');
        thumbnail.append(sheet, number);
        thumbnail.onclick = () => page.scrollIntoView({block: 'start'});
        this.pages.append(page);
        this.thumbnails.append(thumbnail);
        this.entries.push({page, thumbnail, sheet, runs: layouts[index], mapped: mapTextRuns(layouts[index], nodes, allRuns), width: 794, height: 1123});
      }
      this.setZoom(this.fit ? 'fit' : this.zoom);
      for (const index of visible) {
        this.render(Math.min(index, count - 1));
      }
      if (!detached) {
        this.activate();
      }
      return true;
    } catch (error) {
      this.clear();
      if (detached) {
        throw error;
      }
      this.onFailure(error);
      return false;
    }
  }

  activate() {
    this.detached = false;
    this.resize.observe(this.canvas);
    this.scheduleThumbnails();
    this.observer = new IntersectionObserver(entries => {
      for (const item of entries) {
        if (item.isIntersecting) {
          try {
            this.render(Number(item.target.dataset.page));
          } catch (error) {
            this.onFailure(error);
            break;
          }
        }
      }
    }, {root: this.canvas, rootMargin: '400px 0px'});
    this.entries.forEach(entry => this.observer.observe(entry.page));
    this.canvas.onscroll = () => this.updateCounter();
    this.updateCounter();
  }


  async thumbnail() {
    const entry = this.entries[0];
    const image = entry?.page.querySelector('.page-image');
    if (!image) {
      return null;
    }
    await image.decode();
    if (entry !== this.entries[0]) {
      return null;
    }
    const canvas = document.createElement('canvas');
    const scale = Math.min(128 / entry.width, 384 / entry.height);
    canvas.width = Math.max(1, Math.round(entry.width * scale));
    canvas.height = Math.max(1, Math.round(entry.height * scale));
    canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL('image/png');
  }

  scheduleThumbnails() {
    // One synchronous rhwp render per idle turn caps concurrency at one.
    // Existing page URLs also serve the rail; no duplicate render is needed.
    this.idleRender = requestIdleCallback(() => {
      this.idleRender = null;
      const index = this.entries.findIndex(entry => !entry.url);
      if (index === -1) {
        return;
      }
      try {
        this.render(index);
        this.scheduleThumbnails();
      } catch (error) {
        this.onFailure(error);
      }
    }, {timeout: 500});
  }

  render(index) {
    const entry = this.entries[index];
    if (!entry || entry.url) {
      return;
    }
    const svg = this.fonts.mapSvg(this.document.renderPageSvg(index));
    const dimensions = /viewBox="[\d.]+ [\d.]+ ([\d.]+) ([\d.]+)"/.exec(svg);
    if (!dimensions) {
      throw Error('Invalid page dimensions');
    }
    entry.width = Number(dimensions[1]);
    entry.height = Number(dimensions[2]);
    entry.url = URL.createObjectURL(new Blob([svg], {type: 'image/svg+xml'}));
    const image = document.createElement('img');
    image.className = 'page-image';
    image.src = entry.url;
    image.alt = `${this.translate('pages')} ${index + 1}`;
    image.onerror = () => {
      if (this.detached || !this.entries.includes(entry)) {
        return;
      }
      this.onFailure(Error('Page image unavailable'));
    };
    const thumb = image.cloneNode();
    thumb.alt = '';
    entry.sheet.append(thumb);
    entry.hits = document.createElement('div');
    entry.hits.className = 'hit-layer';
    entry.marks = document.createElement('div');
    entry.marks.className = 'mark-layer';
    entry.page.append(image, entry.marks, entry.hits);
    entry.groups = paragraphBounds(entry.runs);
    try {
      entry.controls = JSON.parse(this.document.getPageControlLayout(index)).controls || [];
    } catch {
      entry.controls = [];
    }
    for (const {run, node} of entry.mapped) {
      const hit = document.createElement('button');
      hit.className = 'text-hit';
      hit.dataset.nodeId = node?.id || '';
      hit.setAttribute('aria-label', node?.text || this.translate('locked'));
      hit.setAttribute('aria-readonly', String(!node?.editable));
      // Empty native paragraphs still need a target for insertion and review.
      rectangle(hit, {...run, w: Math.max(12, run.w)}, entry.width, entry.height);
      const group = entry.groups.find(group => group.runs.includes(run));
      hit.onmouseenter = () => this.hover(entry, group, node);
      hit.onmouseleave = () => this.drawMarks(entry);
      hit.onfocus = () => this.hover(entry, group, node);
      hit.onblur = () => this.drawMarks(entry);
      hit.onclick = () => {
        if (node && group) {
          this.onSelect(node, {page: entry.page, bounds: group.bounds, width: entry.width, height: entry.height, focus: hit, paragraph: run.paraIdx});
        }
      };
      entry.hits.append(hit);
    }
    if (this.fit) {
      // Lazy rendering can reveal a wider page than the initial estimate.
      this.setZoom('fit');
    } else {
      this.sizePage(entry);
    }
    this.drawMarks(entry);
  }

  overlay(entry, bounds, type) {
    const mark = document.createElement('div');
    mark.className = `mark-line ${type}`;
    rectangle(mark, bounds, entry.width, entry.height);
    entry.marks.append(mark);
    return mark;
  }

  hover(entry, group, node) {
    this.drawMarks(entry);
    if (!group) {
      return;
    }
    const editable = node?.editable;
    const mark = this.overlay(entry, group.bounds, editable ? 'hover' : 'locked');
    if (!editable) {
      const tag = document.createElement('span');
      tag.className = 'lock-tag';
      tag.textContent = this.translate('locked');
      mark.append(tag);
    }
  }

  drawMarks(entry) {
    if (!entry.marks) {
      return;
    }
    entry.marks.replaceChildren();
    for (const {run, node} of entry.mapped) {
      if (!node) {
        continue;
      }
      this.searchMatches.forEach((match, index) => {
        const bounds = matchBounds(run, node, match);
        if (bounds) {
          this.overlay(entry, bounds, index === this.searchActive ? 'find active' : 'find');
        }
      });
    }
    if (this.controlMarksVisible) {
      entry.fieldMarks ??= inlineFieldMarks(this.document, entry.mapped);
      const marks = [...controlMarks(entry.controls || [], entry.mapped, entry.groups), ...entry.fieldMarks];
      for (const mark of marks) {
        const overlay = this.overlay(entry, mark.bounds, `control-mark ${mark.type}`);
        overlay.dataset.control = mark.type;
        overlay.textContent = mark.type === 'paragraphEnd' ? '¶' : mark.type === 'locked' ? '▣' : '';
        overlay.setAttribute('aria-hidden', 'true');
      }
    }
    const ids = Object.keys(this.edits);
    for (const group of entry.groups) {
      const changed = entry.mapped.filter(item => group.runs.includes(item.run) && item.node && Object.hasOwn(this.edits, item.node.id));
      if (!changed.length) {
        continue;
      }
      const origin = this.origins?.[changed[0].node.id] ? 'agent' : 'human';
      for (const {run, node} of changed) {
        const original = this.originalNodes?.get(node.id)?.text ?? node.text;
        for (const range of changedRanges(original, this.edits[node.id])) {
          const bounds = matchBounds(run, node, {node, text: node.text, ...range}, true);
          if (bounds) {
            this.overlay(entry, bounds, `${origin} range`);
          }
        }
      }
      const margin = document.createElement('div');
      margin.className = `mm ${origin} margin-mark`;
      margin.style.top = `${group.bounds.y / entry.height * 100}%`;
      const number = document.createElement('span');
      number.textContent = ids.indexOf(changed[0].node.id) + 1;
      margin.append(number);
      entry.marks.append(margin);
    }
  }

  showControlMarks(visible) {
    this.controlMarksVisible = visible;
    this.entries.forEach(entry => this.drawMarks(entry));
  }

  updateSearch(matches, active) {
    this.searchMatches = matches;
    this.searchActive = active;
    this.entries.forEach(entry => this.drawMarks(entry));
  }

  updateEdits(edits, originalNodes = this.nodes, origins = {}) {
    this.edits = edits;
    this.originalNodes = new Map(originalNodes.map(node => [node.id, node]));
    this.origins = origins;
    this.entries.forEach(entry => this.drawMarks(entry));
  }

  sizePage(entry) {
    entry.page.style.width = `${entry.width * this.zoom / 100}px`;
    entry.page.style.height = `${entry.height * this.zoom / 100}px`;
  }

  setZoom(value) {
    this.fit = value === 'fit';
    this.zoom = this.fit ? fitZoom(this.canvas.clientWidth, this.entries) : value;
    this.entries.forEach(entry => this.sizePage(entry));
    const percent = Math.round(this.zoom);
    this.onZoom(percent);
    return percent;
  }

  stepZoom(direction) {
    const next = direction > 0 ? ZOOM_STEPS.find(value => value > this.zoom + .1) : ZOOM_STEPS.toReversed().find(value => value < this.zoom - .1);
    return this.setZoom(next ?? (direction > 0 ? 200 : 50));
  }

  updateCounter() {
    const top = this.canvas.getBoundingClientRect().top + 64;
    let nearest = 0;
    let distance = Infinity;
    this.entries.forEach((entry, index) => {
      const difference = Math.abs(entry.page.getBoundingClientRect().top - top);
      if (difference < distance) {
        nearest = index;
        distance = difference;
      }
    });
    this.entries.forEach((entry, index) => entry.thumbnail.classList.toggle('active', index === nearest));
    this.onCounter(nearest + 1, this.entries.length);
  }

  relabel() {
    this.entries.forEach((entry, index) => {
      const label = `${this.translate('pages')} ${index + 1}`;
      entry.thumbnail.setAttribute('aria-label', label);
      const image = entry.page.querySelector('.page-image');
      if (image) {
        image.alt = label;
      }
      [...(entry.hits?.children || [])].forEach((hit, position) => {
        const node = entry.mapped[position].node;
        hit.setAttribute('aria-label', node?.text || this.translate('locked'));
      });
    });
  }
}
