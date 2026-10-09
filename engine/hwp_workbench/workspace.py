"""Workspace-constrained I/O. POSIX directory descriptors prevent symlink races."""
from __future__ import annotations
import os, stat, secrets
from pathlib import Path
from .errors import require, WorkbenchError, MAX_FILE

class Workspace:
    def __init__(self, root):
        self.root=Path(root).expanduser().resolve(strict=True)
        require(self.root.is_dir(),'BAD_ROOT','Workspace root must be a directory.')

    def _parts(self,path):
        require(isinstance(path,str) and bool(path) and '\x00' not in path,'BAD_PATH','Invalid file path.')
        p=Path(path)
        if p.is_absolute():
            try: p=p.relative_to(self.root)
            except ValueError: raise WorkbenchError('OUTSIDE_WORKSPACE','Path is outside the allowed workspace.')
        parts=p.parts
        require(bool(parts) and all(x not in {'.','..',''} for x in parts),'OUTSIDE_WORKSPACE','Relative traversal is prohibited.')
        return parts

    def _parent(self,path):
        parts=self._parts(path)
        flags=os.O_RDONLY|os.O_DIRECTORY|getattr(os,'O_NOFOLLOW',0)
        fd=os.open(self.root,flags)
        try:
            for name in parts[:-1]:
                next_fd=os.open(name,flags,dir_fd=fd)
                os.close(fd);fd=next_fd
            return fd,parts[-1]
        except OSError:
            os.close(fd)
            raise WorkbenchError('UNSAFE_PATH','Parent directory is absent or contains a symlink.')

    def read(self,path):
        parent,name=self._parent(path)
        try:
            fd=os.open(name,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0),dir_fd=parent)
            try:
                info=os.fstat(fd)
                require(stat.S_ISREG(info.st_mode),'UNSAFE_PATH','Only regular files are allowed.')
                require(info.st_size<=MAX_FILE,'LIMIT','File exceeds 64 MiB.')
                with os.fdopen(fd,'rb',closefd=False) as f: data=f.read(MAX_FILE+1)
                require(len(data)<=MAX_FILE,'LIMIT','File exceeds 64 MiB.')
                return data
            finally: os.close(fd)
        except OSError as e: raise WorkbenchError('READ_FAILED',str(e))
        finally: os.close(parent)

    def write_new(self,path,data):
        """Publish a new file atomically, never overwrite any existing destination."""
        require(len(data)<=MAX_FILE,'LIMIT','Output exceeds 64 MiB.')
        parent,name=self._parent(path)
        temp='.hwp-workbench-'+secrets.token_hex(12)+'.tmp'
        try:
            fd=os.open(temp,os.O_WRONLY|os.O_CREAT|os.O_EXCL|getattr(os,'O_NOFOLLOW',0),0o600,dir_fd=parent)
            with os.fdopen(fd,'wb') as f: f.write(data);f.flush();os.fsync(f.fileno())
            # link() fails rather than replacing an existing file; both names are in the same directory.
            os.link(temp,name,src_dir_fd=parent,dst_dir_fd=parent,follow_symlinks=False)
            os.fsync(parent)
        except FileExistsError: raise WorkbenchError('OUTPUT_EXISTS','Destination already exists; use a new filename. Original files are never overwritten.')
        except OSError as e: raise WorkbenchError('WRITE_FAILED',str(e))
        finally:
            try: os.unlink(temp,dir_fd=parent)
            except FileNotFoundError: pass
            os.close(parent)
