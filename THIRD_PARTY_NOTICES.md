# Third-party materials and distribution

HWP Workbench is a private developer alpha. The Apple Silicon package includes
Electron, the rhwp adapter, and a pinned CPython runtime. This inventory describes
the supplied runtime; it is not a complete SBOM or a release-license approval.

## Packaged application

| Material | License and source | Retained notice |
| --- | --- | --- |
| Electron 44.5.1 | MIT; https://github.com/electron/electron | `Contents/Resources/notices/LICENSE` |
| Chromium and Electron's embedded Node.js/V8 dependencies | BSD and other component licenses; supplied Electron distribution | `Contents/Resources/notices/LICENSES.chromium.html` |
| `@rhwp/core` 0.8.7, Edward Kim | MIT; https://github.com/edwardkim/rhwp | License files in `Contents/Resources/app/node_modules/@rhwp/core/` |
| CPython 3.13.15 | PSF License Agreement and historical Python licenses; https://www.python.org/ | `Contents/Resources/python/lib/python3.13/LICENSE.txt` |
| python-build-standalone release 20260924 | Build project: MPL-2.0; https://github.com/astral-sh/python-build-standalone | Build provenance pinned in `scripts/python-runtime.json`; the MPL license applies to the build project, not to every runtime library |

The archive is
`cpython-3.13.15+20260924-aarch64-apple-darwin-install_only_stripped.tar.gz`,
SHA-256 `064afb7c2fc0bbf511d886288adf98696af5105e36c138cdf2c199c0146fcf68`.
The packager verifies this hash before extraction. It copies this notice into the
app's `Contents/Resources/notices/` folder.

## Libraries in the pruned Python runtime

The supplied archive was inspected on Apple Silicon macOS. Its files include
`bin/python3.13`, `lib/libpython3.13.dylib`, the standard library, and optional
extension files. Most extension modules are built into CPython. Removing their
Python wrappers does not remove statically linked library code.

| Library | Runtime evidence | License family |
| --- | --- | --- |
| OpenSSL 3.5.8 | Built-in `_ssl` and `_hashlib`; `ssl.OPENSSL_VERSION` | Apache-2.0 |
| SQLite 3.53.1 | Built-in `_sqlite3`; `sqlite3.sqlite_version` | Public domain |
| libffi | Built-in `_ctypes` | MIT |
| zlib | Built-in `zlib`; `otool -L` lists macOS `/usr/lib/libz.1.dylib` | zlib; the system library is not copied |
| bzip2 1.0.8 | Built-in `_bz2`; `BZ2_bzlibVersion` in the runtime | bzip2 license |
| XZ/liblzma 5.4.3 | Built-in `_lzma`; `lzma_version_string` in the runtime | Component-specific public-domain/permissive notices; retain the upstream COPYING and license texts before distribution |
| mpdecimal 4.0.0 | Built-in `_decimal`; `_decimal.__libmpdec_version__` | BSD-2-Clause |
| Expat 2.8.5 | Built-in `pyexpat`; `pyexpat.EXPAT_VERSION` | MIT |
| mimalloc | Runtime configuration includes `--with-mimalloc` | MIT |
| macOS libedit, ncurses and panel | `otool -L` lists `/usr/lib/libedit.3.dylib`, `libncurses.5.4.dylib` and `libpanel.5.4.dylib` | System-provided libraries; not copied |

The archive has no separate notice files for these statically linked libraries.
Its CPython `LICENSE.txt` is retained, but it does not replace those library
notices. Obtain and review the corresponding upstream license and copyright
texts before external distribution. No dependency or license download was made
in this offline packaging task.

Pruning removes test directories, IDLE, Tkinter, turtle demos, ensurepip/pip,
lib2to3, Tcl/Tk libraries, headers and interpreter configuration tools. It removes
any separate readline extension file. **This pinned build has a built-in
`readline` module backed by macOS libedit, which file pruning cannot remove.**
A different or rebuilt runtime is needed to meet a strict no-readline requirement.

## Development tools and fixtures

Playwright (Apache-2.0) is a test-only dependency and is not copied into the app.
Electron's npm installer and other npm packages are not copied; only
`node_modules/@rhwp/core/` is included alongside the Electron application runtime.

Public HWP/HWPX fixture source metadata is in `fixtures/public-manifest.json`.
Those documents are not bundled. Public availability does not grant permission
to redistribute them. Only the self-authored `fixtures/generated/` documents are
copied. They are parser examples, not qualified Hancom documents.

**No font files are bundled.** Layout uses available fonts and can differ from
Hancom. Font redistribution requires a separate license review.

Projects cited in the technical plan remain evaluation candidates. Mentioning
them does not establish licensing, security or native-layout approval.
