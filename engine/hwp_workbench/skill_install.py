"""Install only this application's marked skill; never expose this through MCP."""
from contextlib import contextmanager
import os
from pathlib import Path
import re
import secrets
import shutil
import stat

from . import __version__
from .errors import WorkbenchError, require
from .workspace import Workspace

PROJECT = Path(__file__).resolve().parents[2]
SKILL_SOURCE = PROJECT / 'skills/hwp-workbench'
TARGETS = ('claude', 'codex')
MARKER = re.compile(r'<!-- installed by hwp-workbench ([A-Za-z0-9.+-]+) -->')
MAX_SKILL_FILE = 2 * 1024 * 1024
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | getattr(os, 'O_NOFOLLOW', 0)


def _home() -> Path:
    override = os.environ.get('HWP_WORKBENCH_SKILL_HOME')
    require(override is None or bool(override), 'BAD_ARGUMENTS', 'Skill home override must not be empty.')
    return Path(override).expanduser().resolve(strict=True) if override is not None else Path.home().resolve(strict=True)


def _folder(target: str) -> str:
    return f'.{target}/skills/hwp-workbench'


@contextmanager
def _parent(workspace: Workspace, target: str, *, create=False):
    descriptor = workspace._directory((f'.{target}', 'skills'), create=create, missing_ok=not create)
    try:
        yield descriptor
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _version(parent):
    """Return (exists, version) using no-follow reads of the folder and marker."""
    if parent is None:
        return False, None
    try:
        info = os.stat('hwp-workbench', dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return False, None
    if not stat.S_ISDIR(info.st_mode):
        return True, None
    folder = os.open('hwp-workbench', DIRECTORY_FLAGS, dir_fd=parent)
    try:
        flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | os.O_NONBLOCK
        try:
            descriptor = os.open('SKILL.md', flags, dir_fd=folder)
        except OSError:
            return True, None
        try:
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_SKILL_FILE:
                return True, None
            with os.fdopen(descriptor, 'rb', closefd=False) as source:
                content = source.read(MAX_SKILL_FILE + 1)
            if len(content) > MAX_SKILL_FILE:
                return True, None
            text = content.decode('utf-8')
            marker = MARKER.search(text)
            return True, marker.group(1) if marker else None
        except UnicodeError:
            return True, None
        finally:
            os.close(descriptor)
    finally:
        os.close(folder)


def status(workspace: Workspace) -> dict:
    result = {}
    for target in TARGETS:
        try:
            with _parent(workspace, target) as parent:
                _, version = _version(parent)
        except (WorkbenchError, OSError):
            version = None
        result[target] = {'installed': version is not None, 'version': version,
                          'path': str(workspace.root / _folder(target))}
    return result


def _targets(target: str) -> tuple[str, ...]:
    require(target in (*TARGETS, 'all'), 'BAD_ARGUMENTS', 'Skill target must be claude, codex or all.')
    return TARGETS if target == 'all' else (target,)


def _content() -> dict[str, bytes]:
    cli_path = str(PROJECT / 'scripts/hwp-workbench')
    marker = f'<!-- installed by hwp-workbench {__version__} -->'
    files = {}
    for name in ('SKILL.md', 'references/patch-format.md'):
        text = (SKILL_SOURCE / name).read_text(encoding='utf-8').replace('{{HWP_WORKBENCH}}', cli_path)
        if name == 'SKILL.md':
            text = text.rstrip() + '\n\n' + marker + '\n'
        files[name] = text.encode('utf-8')
    return files


def _remove_owned(parent) -> bool:
    exists, version = _version(parent)
    if not exists or version is None:
        return False
    # On supported POSIX hosts shutil uses directory descriptors internally.
    require(shutil.rmtree.avoids_symlink_attacks, 'UNSAFE_PATH', 'Safe skill removal is unavailable on this host.')
    shutil.rmtree('hwp-workbench', dir_fd=parent)
    return True


def _backup_owned(workspace: Workspace, target: str, parent) -> str:
    # Preserve the complete previous folder, including local user configuration.
    backup_name = '.hwp-workbench-backup-' + secrets.token_hex(12)
    os.mkdir(backup_name, mode=0o700, dir_fd=parent)
    backup = os.open(backup_name, DIRECTORY_FLAGS, dir_fd=parent)
    try:
        os.rename('hwp-workbench', 'skill', src_dir_fd=parent, dst_dir_fd=backup)
    finally:
        os.close(backup)
    return str(workspace.root / f'.{target}/skills' / backup_name / 'skill')


def install(workspace: Workspace, target: str, dry_run: bool) -> dict:
    selected = _targets(target)
    files = _content()
    # Complete preflight for all selected targets before creating or replacing any.
    for name in selected:
        with _parent(workspace, name) as parent:
            exists, version = _version(parent)
            require(not exists or version is not None, 'SKILL_NOT_OWNED',
                    f'Refusing to replace unmarked skill folder: {workspace.root / _folder(name)}')
    backups = []
    if not dry_run:
        for name in selected:
            with _parent(workspace, name, create=True) as parent:
                exists, version = _version(parent)
                require(not exists or version is not None, 'SKILL_NOT_OWNED', 'Skill ownership changed; no replacement allowed.')
                if exists:
                    backups.append(_backup_owned(workspace, name, parent))
                # Exclusive creation also refuses a destination introduced after preflight.
                os.mkdir('hwp-workbench', mode=0o700, dir_fd=parent)
            folder = _folder(name)
            workspace.ensure_directory(folder + '/references')
            # Write the ownership marker last so an incomplete install is not reported as installed.
            for filename in ('references/patch-format.md', 'SKILL.md'):
                workspace.write_new(folder + '/' + filename, files[filename])
    return {'dryRun': dry_run, 'targets': list(selected), 'backups': backups, **status(workspace)}


def uninstall(workspace: Workspace, target: str) -> dict:
    removed = []
    for name in _targets(target):
        try:
            with _parent(workspace, name) as parent:
                if parent is not None and _remove_owned(parent):
                    removed.append(name)
        except WorkbenchError as error:
            if error.code != 'UNSAFE_PATH':
                raise
    return {'removed': removed, **status(workspace)}


def run(command: str, *, target='all', dry_run=False) -> dict:
    workspace = Workspace(_home())
    if command == 'status':
        return status(workspace)
    if command == 'install':
        return install(workspace, target, dry_run)
    require(command == 'uninstall', 'BAD_ARGUMENTS', 'Unknown skill command.')
    return uninstall(workspace, target)
