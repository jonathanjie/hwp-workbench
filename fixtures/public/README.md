Public compatibility fixtures are downloaded on demand and never committed.

Run `node scripts/fetch-public-fixtures.mjs` from the repository root, then
`cargo test -p hwp-core --test corpus --locked`. Missing files remain skipped.
The downloader checks the immutable Git blob SHA-1 and exact byte count, records
SHA-256, and checks a manifest SHA-256 when supplied. It never replaces files.
Download reports use unique names under `outputs/`.

Sources, sizes, immutable Git blob hashes, and test purposes are in
`../public-manifest.json`. Downloading does not imply public redistribution
rights. These fixtures are for internal testing. No upstream source code or
font files are included in this repository.
