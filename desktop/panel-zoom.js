// The canvas reserves 54 px on each side for page margins and proof marks.
const CANVAS_MARGIN = 108;

export class PanelZoom {
  constructor(view) {
    this.view = view;
    this.visible = false;
    this.previous = null;
  }

  changed() {
    this.previous = null;
  }

  update(visible) {
    if (visible === this.visible) {
      return;
    }
    this.visible = visible;
    const view = this.view;
    if (visible) {
      const width = view.entries[0]?.width || 794;
      if (!view.fit && width * view.zoom / 100 > view.canvas.clientWidth - CANVAS_MARGIN) {
        this.previous = view.zoom;
        view.setZoom('fit');
      }
    } else if (this.previous !== null) {
      view.setZoom(this.previous);
      this.previous = null;
    }
  }
}
