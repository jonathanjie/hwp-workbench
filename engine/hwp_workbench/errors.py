"""Stable error codes shared by desktop, CLI, and MCP."""
class WorkbenchError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
    def as_dict(self):
        return {"code": self.code, "message": str(self)}

MAX_FILE = 64 * 1024 * 1024
MAX_EXPANDED = 128 * 1024 * 1024
MAX_PART = 32 * 1024 * 1024
MAX_NODES = 100_000

def require(condition, code, message):
    if not condition:
        raise WorkbenchError(code, message)
