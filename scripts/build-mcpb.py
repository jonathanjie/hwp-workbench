#!/usr/bin/env python3
"""Build a minimal Claude Desktop bundle without replacing an existing file."""
import argparse
from pathlib import Path
import sys
import zipfile

PROJECT = Path(__file__).resolve().parents[1]


def build(output: Path, *, project: Path = PROJECT) -> Path:
    sources = [
        (project / 'mcpb' / 'manifest.json', 'manifest.json'),
        (project / 'mcpb' / 'launch.py', 'launch.py'),
    ]
    package = project / 'engine' / 'hwp_workbench'
    for path in sorted(package.rglob('*.py')):
        if '__pycache__' not in path.parts:
            sources.append((path, path.relative_to(project).as_posix()))
    icon = project / 'mcpb' / 'icon.png'
    if icon.exists() or icon.is_symlink():
        sources.append((icon, 'icon.png'))
    # Read only selected source files before creating the exclusive output.
    contents = []
    for path, member in sources:
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'Bundle source must be a regular file: {path}')
        contents.append((member, path.read_bytes()))
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, mode='x', compression=zipfile.ZIP_DEFLATED) as bundle:
        for member, content in contents:
            bundle.writestr(member, content)
    return output


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=PROJECT / 'dist' / 'hwp-workbench.mcpb',
                        help='New bundle path (default: dist/hwp-workbench.mcpb).')
    args = parser.parse_args(argv)
    try:
        output = build(args.output)
    except (OSError, ValueError) as error:
        print(f'Bundle not built: {error}', file=sys.stderr)
        return 2
    print(output)
    if not (PROJECT / 'mcpb' / 'icon.png').is_file():
        print('Icon missing: add mcpb/icon.png before distribution.', file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main())
