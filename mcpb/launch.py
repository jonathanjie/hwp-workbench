"""Start the HWP Workbench MCP server with a supported Python.

Claude Desktop starts bundle servers with a minimal PATH, where `python3` is
often Apple's /usr/bin/python3 (3.9). The engine needs Python 3.11 or newer,
so this launcher stays compatible with Python 3.8 and hands over to the first
supported interpreter it finds. It never installs anything.
"""
import os
import subprocess
import sys

MINIMUM_VERSION = (3, 11)
BUNDLE_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
ENGINE_DIRECTORY = os.path.join(BUNDLE_DIRECTORY, "engine")

# The installed HWP Workbench app carries its own Python and Electron.
APP_BUNDLES = [
    "/Applications/HWP Workbench.app",
    os.path.expanduser("~/Applications/HWP Workbench.app"),
]
APP_PYTHONS = [os.path.join(bundle, "Contents/Resources/python/bin/python3") for bundle in APP_BUNDLES]

# Common install locations on macOS, newest conventions first.
CANDIDATE_INTERPRETERS = [
    os.environ.get("HWP_WORKBENCH_PYTHON", ""),
] + APP_PYTHONS + [
    "/opt/homebrew/bin/python3",
    "/usr/local/bin/python3",
    os.path.expanduser("~/.local/bin/python3"),
    os.path.expanduser("~/.pyenv/shims/python3"),
    "/Library/Frameworks/Python.framework/Versions/Current/bin/python3",
    "/opt/miniconda3/bin/python3",
    "/usr/bin/python3",
]


def interpreter_version(path):
    """Return (major, minor) for an interpreter, or None if it cannot run."""
    if not path or not os.path.isfile(path) or not os.access(path, os.X_OK):
        return None
    try:
        output = subprocess.run(
            [path, "-c", "import sys; print(sys.version_info[0], sys.version_info[1])"],
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        ).stdout
        major, minor = output.split()
        return int(major), int(minor)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def supported_interpreter():
    if sys.version_info[:2] >= MINIMUM_VERSION:
        return sys.executable
    for path in CANDIDATE_INTERPRETERS:
        version = interpreter_version(path)
        if version is not None and version >= MINIMUM_VERSION:
            return path
    return None


def add_app_node(environment):
    """Let the rhwp check use the installed app's Electron as Node."""
    if environment.get("HWP_WORKBENCH_NODE"):
        return
    for bundle in APP_BUNDLES:
        electron = os.path.join(bundle, "Contents/MacOS/HWP Workbench")
        if os.path.isfile(electron) and os.access(electron, os.X_OK):
            environment["HWP_WORKBENCH_NODE"] = electron
            environment["ELECTRON_RUN_AS_NODE"] = "1"
            return


def main():
    interpreter = supported_interpreter()
    if interpreter is None:
        sys.stderr.write(
            "HWP Workbench needs Python 3.11 or newer. Install it (for example "
            "`brew install python`) or set HWP_WORKBENCH_PYTHON to its path.\n"
        )
        return 1
    environment = dict(os.environ)
    environment["PYTHONPATH"] = ENGINE_DIRECTORY
    add_app_node(environment)
    arguments = [interpreter, "-m", "hwp_workbench"] + sys.argv[1:]
    # Replace this process so stdio stays connected to the MCP client.
    os.execve(interpreter, arguments, environment)
    return 0


if __name__ == "__main__":
    sys.exit(main())
