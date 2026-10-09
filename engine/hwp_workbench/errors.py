"""Stable error codes and recovery actions shared by CLI, RPC and MCP."""

# retryable means retry after the stated correction, not repeat the same request.
RECOVERY = {
    'STALE_DOCUMENT': ('Re-run read or inspect on the source and rebuild the patch with its current SHA-256.', True),
    'STALE_TEXT': ('Re-run read or inspect and rebuild the patch with the current expectedText.', True),
    'NODE_NOT_FOUND': ('Re-run read or search and use a full node ID from the current source.', True),
    'HWP_LENGTH_CHANGE_BLOCKED': ('Use replacement text with the same UTF-16 code-unit length as the original node.', True),
    'SKILL_NOT_OWNED': ('Keep the existing skill folder; install only into an absent or application-marked folder.', True),
    'OUTPUT_EXISTS': ('Choose a new output filename; existing files cannot be replaced.', True),
    'OUTPUT_FORMAT': ('Use the required extension: the source format for native save, or the requested target for conversion.', True),
    'RISK_ACK_REQUIRED': ('Review the patch and compatibility warnings, then pass acknowledgeRisks: true to save.', True),
    'BAD_PATCH': ('Rebuild a schemaVersion-1 patch with the current hash, expectedText and 1–1000 unique targets.', True),
    'BAD_ARGUMENTS': ('Correct the arguments using CLI help or the MCP tools/list input schema.', True),
    'BAD_REGEX': ('Correct the regex syntax, or use a literal search with regex: false.', True),
    'REGEX_LIMIT': ('Use a regex pattern of at most 256 characters, or use literal search.', True),
    'REGEX_UNSUPPORTED': ('Use literals, character classes, escapes, dots and anchors only, or use literal search.', True),
    'CONTROL_TEXT': ('Use plain text without tabs, newlines, native controls or invalid Unicode characters.', True),
    'BAD_PATH': ('Supply a nonempty regular-file path inside the configured workspace.', True),
    'BAD_ROOT': ('Choose an existing directory as the workspace root.', True),
    'OUTSIDE_WORKSPACE': ('Use a path inside the configured workspace; relative traversal is prohibited.', True),
    'UNSAFE_PATH': ('Use an existing parent directory and regular file path without symlink components.', True),
    'READ_FAILED': ('Check that the source exists inside the workspace and is readable without symlinks.', True),
    'WRITE_FAILED': ('Check output directory permissions and free space, then use a new output filename.', True),
    'LIMIT': ('Reduce the file, request, pattern or result size to the documented limit.', True),
    'UNKNOWN_METHOD': ('Use an operation listed by capabilities.', True),
    'REQUEST_FAILED': ('Check the request JSON, required fields, workspace paths and file permissions, then retry.', True),
    'SAVE_BLOCKED': ('Read the warnings and use an editable source without protected or active content.', False),
    'UNSUPPORTED_ELEMENT': ('Select an editable plain-text node from read or inspect.', True),
    'VERIFY_FAILED': ('Keep the original file and report the verification failure; do not publish the output.', False),
}

MALFORMED_DOCUMENT_CODES = {
    'BAD_CFB', 'BAD_HWP_COMPRESSION', 'BAD_HWP_RECORD', 'BAD_HWP_TEXT',
    'BAD_XML', 'BAD_ZIP', 'NOT_HWP', 'NOT_HWPX', 'NOT_DOCX', 'UNSAFE_XML', 'UNSAFE_ZIP', 'XML_ENCODING',
}
UNSUPPORTED_DOCUMENT_CODES = {'ENCRYPTED', 'PROTECTED_HWP', 'UNSUPPORTED_FORMAT'}


def recovery(code: str) -> dict:
    if code in RECOVERY:
        hint, retryable = RECOVERY[code]
    elif code in MALFORMED_DOCUMENT_CODES:
        hint = 'Use an intact supported HWP/HWPX source; keep this file unchanged and report the parse failure.'
        retryable = False
    elif code in UNSUPPORTED_DOCUMENT_CODES:
        hint = 'Use a supported, unprotected HWP 5.x or HWPX document; inspect the compatibility limits.'
        retryable = False
    else:
        hint = 'Check the request and compatibility limits; report the code if the problem continues.'
        retryable = False
    return {'hint': hint, 'retryable': retryable}


class WorkbenchError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code

    def as_dict(self):
        return {'code': self.code, 'message': str(self), **recovery(self.code)}


def error_details(error: Exception) -> dict:
    if isinstance(error, WorkbenchError):
        return error.as_dict()
    return {'code': 'REQUEST_FAILED', 'message': str(error), **recovery('REQUEST_FAILED')}


MAX_FILE = 64 * 1024 * 1024
MAX_EXPANDED = 128 * 1024 * 1024
MAX_PART = 32 * 1024 * 1024
MAX_NODES = 100_000


def require(condition, code, message):
    if not condition:
        raise WorkbenchError(code, message)
