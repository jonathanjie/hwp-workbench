// Reserve canvas padding plus 54 px on each side for proof marks.
const CANVAS_MARGIN = 216;

export function fitZoom(canvasWidth, entries) {
  const width = Math.max(0, ...entries.map(entry => entry.width)) || 794;
  return Math.max(10, Math.min(200, (canvasWidth - CANVAS_MARGIN) / width * 100));
}

export class PanelZoom {
  constructor(view, nextFrame = callback => requestAnimationFrame(callback)) {
    this.view = view;
    this.nextFrame = nextFrame;
    this.revision = 0;
    this.visible = false;
    this.previous = null;
  }

  changed() {
    this.previous = null;
    this.revision += 1;
  }

  update(visible) {
    if (visible === this.visible) {
      return;
    }
    this.visible = visible;
    const revision = ++this.revision;
    if (visible) {
      // Measure after the panel layout and any preview swap have completed.
      this.nextFrame(() => {
        if (revision !== this.revision) {
          return;
        }
        const view = this.view;
        if (view.fit) {
          view.setZoom('fit');
        } else if (view.zoom > fitZoom(view.canvas.clientWidth, view.entries)) {
          this.previous = view.zoom;
          view.setZoom('fit');
        }
      });
    } else if (this.previous !== null) {
      this.view.setZoom(this.previous);
      this.previous = null;
    }
  }
}
