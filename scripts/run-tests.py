#!/usr/bin/env python3
import sys,unittest,json,platform,time
from pathlib import Path
root=Path(__file__).resolve().parents[1];sys.path[:0]=[str(root/'engine'),str(root/'tests')]
start=time.monotonic();suite=unittest.defaultTestLoader.discover(str(root/'tests'),pattern='test_*.py');result=unittest.TextTestRunner(verbosity=2).run(suite)
report={'suite':'unit + integration (synthetic fixtures)','python':platform.python_version(),'platform':platform.platform(),'testsRun':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'skipped':len(result.skipped),'success':result.wasSuccessful(),'elapsedSeconds':round(time.monotonic()-start,3),'notTested':['macOS Electron runtime','Hancom interoperability','public binary fixture corpus','native page fidelity']}
(root/'reports').mkdir(exist_ok=True);(root/'reports/test-results.json').write_text(json.dumps(report,indent=2))
sys.exit(0 if result.wasSuccessful() else 1)
