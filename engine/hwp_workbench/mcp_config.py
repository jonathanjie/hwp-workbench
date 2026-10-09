"""Print client setup without running clients or changing their configuration."""
import json
from pathlib import Path
import shlex

from .errors import WorkbenchError, require


def setup(client: str, root) -> str:
    try:
        workspace = Path(root).expanduser().resolve(strict=True)
    except OSError as error:
        raise WorkbenchError('BAD_ROOT', 'Workspace must be an existing document directory.') from error
    require(workspace.is_dir(), 'BAD_ROOT', 'Workspace must be an existing directory.')
    require(workspace not in {Path('/'), Path.home().resolve()}, 'BAD_ROOT',
            'The home directory and / grant too much access. Choose a documents subfolder.')
    project = Path(__file__).resolve().parents[2]
    command = str(project / 'scripts' / 'hwp-workbench')
    arguments = ['mcp', '--root', str(workspace)]
    if client == 'claude-code':
        return shlex.join(['claude', 'mcp', 'add', 'hwp-workbench', '--scope', 'user', '--',
                           command, *arguments])
    if client == 'codex':
        return shlex.join(['codex', 'mcp', 'add', 'hwp-workbench', '--', command, *arguments])
    if client == 'claude-desktop':
        return str(project / 'dist' / 'hwp-workbench.mcpb')
    return json.dumps({'mcpServers': {'hwp-workbench': {'command': command, 'args': arguments}}}, indent=2)
