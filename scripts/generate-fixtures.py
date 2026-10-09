#!/usr/bin/env python3
from pathlib import Path
import sys
root=Path(__file__).resolve().parents[1]
sys.path[:0]=[str(root/'engine'),str(root/'tests')]
from fixtures import make_hwpx,make_hwp
out=root/'fixtures/generated';out.mkdir(parents=True,exist_ok=True)
(out/'review-form.hwpx').write_bytes(make_hwpx())
(out/'legacy-review.hwp').write_bytes(make_hwp())
print('Generated 2 synthetic fixtures. These are not Hancom-certified or public-source documents.')
