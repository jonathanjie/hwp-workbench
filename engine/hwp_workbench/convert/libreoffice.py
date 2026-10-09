"""Optional headless import/validation, isolated from the user's office profile."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from ..errors import MAX_FILE, WorkbenchError, require
from ..workspace import Workspace

TIMEOUT = 60
MAC_EXECUTABLE = Path('/Applications/LibreOffice.app/Contents/MacOS/soffice')


def executable():
    found = shutil.which('soffice') or shutil.which('libreoffice')
    if found:
        return found
    return str(MAC_EXECUTABLE) if MAC_EXECUTABLE.is_file() and os.access(MAC_EXECUTABLE, os.X_OK) else None


def convert(payload, source_format, target_format):
    require(source_format in {'doc', 'odt', 'rtf', 'docx'} and target_format in {'docx', 'pdf'},
            'BAD_ARGUMENTS', 'Unsupported LibreOffice conversion route.')
    require(isinstance(payload, bytes) and 0 < len(payload) <= MAX_FILE, 'LIMIT', 'LibreOffice input exceeds file limit.')
    program = executable()
    if program is None:
        return None, {'engine': 'LibreOffice', 'available': False, 'reason': 'LibreOffice is not installed.'}
    with tempfile.TemporaryDirectory(prefix='hwp-convert-') as directory:
        folder = Path(directory)
        # TemporaryDirectory creates a private 0700 folder. Both document copies
        # and the fresh security profile are removed on success, error or timeout.
        profile = folder / 'profile'
        profile.mkdir(mode=0o700)
        (profile / 'user').mkdir(mode=0o700)
        (profile / 'user/registrymodifications.xcu').write_text(
            '<?xml version="1.0" encoding="UTF-8"?><oor:items xmlns:oor="http://openoffice.org/2001/registry">'
            '<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>'
            '<item oor:path="/org.openoffice.Office.Writer/Content"><prop oor:name="Update/Link" oor:op="fuse"><value>0</value></prop></item></oor:items>',
            encoding='utf-8')
        workspace = Workspace(folder)
        workspace.write_new('input.' + source_format, payload)
        output = folder / 'output'
        output.mkdir(mode=0o700)
        command = [program, '-env:UserInstallation=' + profile.as_uri(), '--headless', '--norestore',
                   '--convert-to', target_format, '--outdir', str(output), str(folder / ('input.' + source_format))]
        try:
            completed = subprocess.run(command, shell=False, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, timeout=TIMEOUT, check=False)
        except subprocess.TimeoutExpired as error:
            raise WorkbenchError('LIBREOFFICE_TIMEOUT', 'LibreOffice exceeded the 60-second timeout.') from error
        except OSError as error:
            raise WorkbenchError('LIBREOFFICE_FAILED', 'LibreOffice could not start: ' + str(error)) from error
        require(completed.returncode == 0, 'LIBREOFFICE_FAILED', f'LibreOffice conversion failed (exit {completed.returncode}).')
        # LibreOffice exits 0 even when it cannot load the source file.
        require((output / ('input.' + target_format)).is_file(), 'LIBREOFFICE_FAILED',
                'LibreOffice could not load the file; no output was written.')
        result = workspace.read('output/input.' + target_format)
        if target_format == 'pdf':
            require(result.startswith(b'%PDF-'), 'LIBREOFFICE_FAILED', 'LibreOffice did not create a PDF.')
        return result, {'engine': 'LibreOffice', 'available': True, 'converted': True,
                        'from': source_format, 'to': target_format, 'freshProfile': True, 'timeoutSeconds': TIMEOUT}
