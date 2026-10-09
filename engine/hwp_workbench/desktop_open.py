"""CLI-only desktop launch; document operations remain in the shared service."""
import os
from pathlib import Path
import subprocess
import sys

from .errors import WorkbenchError, require
from .workspace import Workspace

PROJECT = Path(__file__).resolve().parents[2]
APPLICATION_DIRECTORIES = (Path('/Applications'), Path.home() / 'Applications')


def _packaged_app() -> bool:
    return sys.platform == 'darwin' and any(
        (directory / 'HWP Workbench.app').is_dir() for directory in APPLICATION_DIRECTORIES
    )


def open_document(root, path: str) -> dict:
    workspace = Workspace(root)
    relative = workspace.relative_path(path)
    require(Path(relative).suffix.lower() in {'.hwp', '.hwpx'}, 'OPEN_FORMAT',
            'Desktop review requires an existing .hwp or .hwpx file.')
    # Validate regular-file and no-follow access before handing a path to the app.
    workspace.read(relative)
    document = str(workspace.root / relative)
    try:
        if _packaged_app():
            subprocess.run(['open', '-a', 'HWP Workbench', document], check=True,
                           capture_output=True, text=True, timeout=10)
            return {'opened': True, 'via': 'app'}
        electron = PROJECT / 'node_modules' / '.bin' / 'electron'
        source_checkout = (PROJECT / 'package.json').is_file() and (PROJECT / 'desktop' / 'main.cjs').is_file()
        require(source_checkout and electron.is_file() and os.access(electron, os.X_OK),
                'APP_UNAVAILABLE', 'HWP Workbench.app and a source checkout with installed Electron were not found.')
        subprocess.Popen([str(electron), str(PROJECT), '--open', document], cwd=PROJECT,
                         start_new_session=True, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return {'opened': True, 'via': 'electron'}
    except (OSError, subprocess.SubprocessError) as error:
        raise WorkbenchError('OPEN_FAILED', f'Could not launch HWP Workbench: {error}') from error
