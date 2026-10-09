"""Workspace-constrained I/O. POSIX directory descriptors prevent symlink races."""
from __future__ import annotations

import os
import hashlib
from datetime import datetime, timezone
from pathlib import Path
import re
import secrets
import stat

from .errors import MAX_FILE, WorkbenchError, require


class Workspace:
    def __init__(self, root):
        self.root = Path(root).expanduser().resolve(strict=True)
        require(self.root.is_dir(), 'BAD_ROOT', 'Workspace root must be a directory.')

    def _parts(self, path):
        require(isinstance(path, str) and bool(path) and '\x00' not in path, 'BAD_PATH', 'Invalid file path.')
        relative = Path(path)
        if relative.is_absolute():
            try:
                relative = relative.relative_to(self.root)
            except ValueError:
                raise WorkbenchError('OUTSIDE_WORKSPACE', 'Path is outside the allowed workspace.')
        parts = relative.parts
        require(bool(parts) and all(part not in {'.', '..', ''} for part in parts),
                'OUTSIDE_WORKSPACE', 'Relative traversal is prohibited.')
        return parts

    def relative_path(self, path) -> str:
        return str(Path(*self._parts(path)))

    def _directory(self, parts, *, create=False, missing_ok=False):
        flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, 'O_NOFOLLOW', 0)
        descriptor = os.open(self.root, flags)
        try:
            for name in parts:
                if create:
                    try:
                        os.mkdir(name, mode=0o700, dir_fd=descriptor)
                    except FileExistsError:
                        pass
                next_descriptor = os.open(name, flags, dir_fd=descriptor)
                os.close(descriptor)
                descriptor = next_descriptor
            return descriptor
        except OSError as error:
            os.close(descriptor)
            if missing_ok and isinstance(error, FileNotFoundError):
                return None
            raise WorkbenchError('UNSAFE_PATH', 'Parent directory is absent or contains a symlink.') from error

    def _parent(self, path, *, missing_ok=False):
        parts = self._parts(path)
        return self._directory(parts[:-1], missing_ok=missing_ok), parts[-1]

    def ensure_directory(self, path):
        descriptor = self._directory(self._parts(path), create=True)
        os.close(descriptor)

    def list_files(self, path, *, name_pattern, limit=200):
        """List at most limit matching names without following directory links."""
        descriptor = self._directory(self._parts(path), missing_ok=True)
        if descriptor is None:
            return []
        names = []
        try:
            with os.scandir(descriptor) as entries:
                for entry in entries:
                    if name_pattern.fullmatch(entry.name) is None:
                        continue
                    names.append(entry.name)
                    if len(names) == limit:
                        break
            return sorted(names)
        finally:
            os.close(descriptor)

    def read(self, path, *, max_bytes=MAX_FILE, missing_ok=False):
        parent, name = self._parent(path, missing_ok=missing_ok)
        if parent is None:
            return None
        try:
            # Nonblocking open lets fstat reject FIFOs without waiting for a writer.
            flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | os.O_NONBLOCK
            descriptor = os.open(name, flags, dir_fd=parent)
            try:
                info = os.fstat(descriptor)
                require(stat.S_ISREG(info.st_mode), 'UNSAFE_PATH', 'Only regular files are allowed.')
                require(info.st_size <= max_bytes, 'LIMIT', f'File exceeds {max_bytes} bytes.')
                with os.fdopen(descriptor, 'rb', closefd=False) as source:
                    data = source.read(max_bytes + 1)
                require(len(data) <= max_bytes, 'LIMIT', f'File exceeds {max_bytes} bytes.')
                return data
            finally:
                os.close(descriptor)
        except FileNotFoundError as error:
            if missing_ok:
                return None
            raise WorkbenchError('READ_FAILED', str(error)) from error
        except OSError as error:
            raise WorkbenchError('READ_FAILED', str(error)) from error
        finally:
            os.close(parent)

    def write_new(self, path, data):
        """Publish a new file atomically, never overwrite any existing destination."""
        require(len(data) <= MAX_FILE, 'LIMIT', 'Output exceeds 64 MiB.')
        parent, name = self._parent(path)
        temporary_name = '.hwp-workbench-' + secrets.token_hex(12) + '.tmp'
        temporary_created = False
        try:
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0)
            descriptor = os.open(temporary_name, flags, 0o600, dir_fd=parent)
            temporary_created = True
            with os.fdopen(descriptor, 'wb') as output:
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            # link fails rather than replacing any existing file in this directory.
            os.link(temporary_name, name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
            os.fsync(parent)
        except FileExistsError:
            raise WorkbenchError('OUTPUT_EXISTS', 'Destination already exists; use a new filename. Original files are never overwritten.')
        except OSError as error:
            raise WorkbenchError('WRITE_FAILED', str(error)) from error
        finally:
            if temporary_created:
                try:
                    os.unlink(temporary_name, dir_fd=parent)
                except FileNotFoundError:
                    pass
            os.close(parent)

    def _replacement_source(self, parent, name, expected_sha256):
        """Read bounded bytes and check write access without changing the file."""
        flags = os.O_RDWR | getattr(os, 'O_NOFOLLOW', 0) | os.O_NONBLOCK
        try:
            descriptor = os.open(name, flags, dir_fd=parent)
        except PermissionError as error:
            raise WorkbenchError('READ_ONLY_DOCUMENT', 'Document is not writable.') from error
        except OSError as error:
            raise WorkbenchError('UNSAFE_PATH', 'Document must be an existing regular file without symlinks.') from error
        try:
            info = os.fstat(descriptor)
            require(stat.S_ISREG(info.st_mode), 'UNSAFE_PATH', 'Only regular files are allowed.')
            require(info.st_mode & 0o222, 'READ_ONLY_DOCUMENT', 'Document has no write permission.')
            require(info.st_size <= MAX_FILE, 'LIMIT', 'Document exceeds 64 MiB.')
            with os.fdopen(descriptor, 'rb', closefd=False) as source:
                data = source.read(MAX_FILE + 1)
            require(len(data) <= MAX_FILE, 'LIMIT', 'Document exceeds 64 MiB.')
            require(hashlib.sha256(data).hexdigest() == expected_sha256,
                    'SOURCE_CHANGED', 'Document changed on disk; read it again before saving.')
            return data, info
        finally:
            os.close(descriptor)

    def _private_file(self, parent, data, mode):
        """Return an owned temporary name only after its bytes and mode are synced."""
        name = '.hwp-workbench-' + secrets.token_hex(12) + '.tmp'
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0)
        descriptor = os.open(name, flags, 0o600, dir_fd=parent)
        try:
            with os.fdopen(descriptor, 'wb') as output:
                output.write(data)
                output.flush()
                os.fchmod(output.fileno(), mode)
                os.fsync(output.fileno())
        except BaseException:
            os.unlink(name, dir_fd=parent)
            raise
        return name

    def _retain_recovery(self, path, parent, recovery_name):
        """Move owned recovery bytes into the private backup folder without pruning."""
        document = Path(self.relative_path(path))
        directory = str(document.parent / '.hwp-workbench' / 'backups')
        self.ensure_directory(directory)
        backup_parent = self._directory(self._parts(directory))
        timestamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
        name = f'{document.name}.recovery-{timestamp}-{secrets.token_hex(6)}'
        try:
            os.chmod(recovery_name, 0o600, dir_fd=parent, follow_symlinks=False)
            os.link(recovery_name, name, src_dir_fd=parent,
                    dst_dir_fd=backup_parent, follow_symlinks=False)
            os.fsync(backup_parent)
            os.unlink(recovery_name, dir_fd=parent)
            os.fsync(parent)
        finally:
            os.close(backup_parent)
        return str(Path(directory) / name)

    def replace_existing(self, path, data, expected_sha256):
        """Replace a hash-bound source atomically and return its disk-read SHA-256."""
        require(len(data) <= MAX_FILE, 'LIMIT', 'Output exceeds 64 MiB.')
        parent, name = self._parent(path)
        temporary_name = None
        recovery_name = None
        replaced = False
        try:
            original, info = self._replacement_source(parent, name, expected_sha256)
            mode = stat.S_IMODE(info.st_mode)
            temporary_name = self._private_file(parent, data, mode)
            # Retain recovery bytes until post-rename sync and disk verification
            # succeed. Rollback is allowed only while this output is still there.
            recovery_name = self._private_file(parent, original, mode)
            _, current_info = self._replacement_source(parent, name, expected_sha256)
            require((current_info.st_dev, current_info.st_ino) == (info.st_dev, info.st_ino),
                    'SOURCE_CHANGED', 'Document was replaced during saving.')
            published_info = os.stat(temporary_name, dir_fd=parent, follow_symlinks=False)
            expected_output = hashlib.sha256(data).hexdigest()
            os.replace(temporary_name, name, src_dir_fd=parent, dst_dir_fd=parent)
            temporary_name = None
            replaced = True
            os.fsync(parent)
            saved, saved_info = self._replacement_source(parent, name, expected_output)
            require((saved_info.st_dev, saved_info.st_ino) ==
                    (published_info.st_dev, published_info.st_ino),
                    'SOURCE_CHANGED', 'Document was replaced after saving.')
            # Cleanup is part of the guarded operation: an unlink failure must
            # restore the source instead of reporting a failed save after changing it.
            os.unlink(recovery_name, dir_fd=parent)
            recovery_name = None
            return hashlib.sha256(saved).hexdigest()
        except BaseException as error:
            if replaced:
                try:
                    _, current_info = self._replacement_source(parent, name, expected_output)
                    owns_output = ((current_info.st_dev, current_info.st_ino) ==
                                   (published_info.st_dev, published_info.st_ino))
                except (OSError, WorkbenchError):
                    owns_output = False
                if not owns_output:
                    # Keep an external revision, even if it has identical bytes.
                    # Do not let final cleanup discard recovery on backup failure.
                    retained_name = recovery_name
                    recovery_name = None
                    backup = self._retain_recovery(path, parent, retained_name)
                    raise WorkbenchError('SOURCE_CHANGED',
                                         'Document changed after publication; recovery kept at ' + backup) from error
                os.replace(recovery_name, name, src_dir_fd=parent, dst_dir_fd=parent)
                recovery_name = None
                os.fsync(parent)
            if isinstance(error, OSError):
                raise WorkbenchError('WRITE_FAILED', str(error)) from error
            raise
        finally:
            for owned_name in (temporary_name, recovery_name):
                if owned_name is not None:
                    os.unlink(owned_name, dir_fd=parent)
            os.close(parent)

    def backup_existing(self, path, data, backup_directory=None):
        """Keep private previous bytes; prune only this document's backup names."""
        document = Path(self.relative_path(path))
        directory = backup_directory
        if directory is None:
            directory = str(document.parent / '.hwp-workbench' / 'backups')
        self.ensure_directory(directory)
        source_identity = hashlib.sha256(os.fsencode(self.root / document)).hexdigest()[:8]
        timestamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
        name = f'{document.stem}.{source_identity}.{timestamp}-{secrets.token_hex(3)}{document.suffix}'
        backup = str(Path(directory) / name)
        self.write_new(backup, data)
        require(self.read(backup) == data, 'VERIFY_FAILED', 'Previous-version backup differs from the source.')
        pattern = re.compile(re.escape(document.stem + '.' + source_identity) +
                             r'\.[0-9]{8}T[0-9]{6}Z-[0-9a-f]{6}' + re.escape(document.suffix))
        self._prune_backups(directory, pattern, name)
        return backup

    def _prune_backups(self, directory, pattern, newest_name):
        descriptor = self._directory(self._parts(directory))
        try:
            # Keep only ten candidates in memory, even in a large directory.
            newest = []
            with os.scandir(descriptor) as entries:
                for entry in entries:
                    if pattern.fullmatch(entry.name) is None:
                        continue
                    info = entry.stat(follow_symlinks=False)
                    if not stat.S_ISREG(info.st_mode):
                        continue
                    rank = (entry.name == newest_name, info.st_mtime_ns, entry.name)
                    newest.append(rank)
                    newest.sort(reverse=True)
                    if len(newest) > 10:
                        newest.pop()
            kept_names = {item[2] for item in newest}
            with os.scandir(descriptor) as entries:
                for entry in entries:
                    if pattern.fullmatch(entry.name) is None or entry.name in kept_names:
                        continue
                    if stat.S_ISREG(entry.stat(follow_symlinks=False).st_mode):
                        os.unlink(entry.name, dir_fd=descriptor)
            os.fsync(descriptor)
        except OSError as error:
            raise WorkbenchError('WRITE_FAILED', 'Cannot prune previous versions: ' + str(error)) from error
        finally:
            os.close(descriptor)
