"""Workspace-constrained I/O. POSIX directory descriptors prevent symlink races."""
from __future__ import annotations

import os
from pathlib import Path
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
