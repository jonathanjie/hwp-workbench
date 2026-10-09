# Third-party materials and distribution

This project is configured as private/internal. It is not published as an open-source package. No third-party HWP engine source is vendored. `@rhwp/core` 0.8.7 (MIT, Edward Kim, https://github.com/edwardkim/rhwp) is an npm dependency used as a render and independent-verification adapter; the Python engine remains the authoritative save path.

Electron and Playwright (Apache-2.0, test-only) are external development/runtime dependencies installed separately through npm and retains its own license and notices. Node.js, Python, Chromium and any optional test tools retain their respective licenses. A complete dependency inventory and notices review are required before distributing a packaged application; this file is not an SBOM.

Public HWP/HWPX fixture source metadata is recorded in `fixtures/public-manifest.json`. Those binaries are not included in this handoff or Git commits. Download and use them only within the source's permitted terms; review fixture-specific rights before any redistribution. Their public availability is not a copyright permission statement.

The included `fixtures/generated/` documents and screenshots are self-authored synthetic test materials. They are not official Korean government forms and are not claimed to be conformant, complete Hancom documents.

No font files are included. Future rendering must use appropriately available fonts and disclose substitutions; any font redistribution requires separate authorization.

The technical plan cites external projects to evaluate. Mentioning those projects does not mean their code, licensing, security or native fidelity has been independently approved for this application.
