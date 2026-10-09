#!/usr/bin/env python3
"""Regenerate self-authored documents and their hash-bound example patches."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / 'engine'), str(ROOT / 'tests')]
from fixtures import make_hwp, make_hwpx
from hwp_workbench.document import Document


def main():
    output = ROOT / 'fixtures/generated'
    output.mkdir(parents=True, exist_ok=True)
    fixtures = [
        ('review-form.hwpx', 'review.patch.json', make_hwpx()),
        ('legacy-review.hwp', 'legacy.patch.json', make_hwp()),
    ]
    for fixture_name, patch_name, payload in fixtures:
        patch_path = ROOT / 'examples' / patch_name
        example = json.loads(patch_path.read_text())
        document = Document(payload, fixture_name)
        example['baseSha256'] = document.hash
        # Validate existing node IDs and expected text before refreshing hashes.
        document.patch(example)
        (output / fixture_name).write_bytes(payload)
        patch_path.write_text(json.dumps(example, ensure_ascii=False, indent=2) + '\n')
    print('Generated 2 self-authored fixtures and refreshed their example patches.')
    print('These are not Hancom-certified or public-source documents.')


if __name__ == '__main__':
    main()
