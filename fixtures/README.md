# Document fixtures

`generated/` contains self-authored, frozen samples. The deleted
`scripts/generate-fixtures.py` made them; see commit `6d8f17f` in Git history.
New test fixtures are built in Rust. These samples are not certified forms and
do not establish interoperability through a synthetic round-trip.

`public-manifest.json` records source URLs, immutable Git blob hashes, sizes and
test purposes. Download with `node scripts/fetch-public-fixtures.mjs`. Downloaded
files stay ignored by Git and are not redistributed. Review fixture-specific
rights before redistribution. Rust corpus tests use these files when present
and report absent files as skipped.
