// Previews are disposable. File operations keep their own ordered lane.
class LatestPreview {
  cancel() {
    this.controller?.abort();
    this.controller = null;
  }

  async run(work) {
    this.cancel();
    const controller = new AbortController();
    this.controller = controller;
    try {
      const result = await work(controller.signal);
      return controller.signal.aborted ? null : result;
    } catch (error) {
      if (controller.signal.aborted) {
        return null;
      }
      throw error;
    } finally {
      if (this.controller === controller) {
        this.controller = null;
      }
    }
  }
}
module.exports = {LatestPreview};
