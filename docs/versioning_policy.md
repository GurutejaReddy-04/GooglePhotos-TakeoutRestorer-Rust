# Versioning Policy

This project strictly adheres to [Semantic Versioning 2.0.0](https://semver.org/).

Given a version number MAJOR.MINOR.PATCH:
1. **MAJOR** version when making incompatible API changes, altering the persistence database schema, or removing significant UI workflow functionality.
2. **MINOR** version when adding functionality in a backward-compatible manner (e.g., parsing a new metadata field, adding a new CLI flag).
3. **PATCH** version when making backward-compatible bug fixes (e.g., fixing a crash, fixing a UI typo).

## Release Channels
- **Release Candidates (RC):** `v1.0.0-rc.1`. Used to validate installers and run manual smoke tests prior to public release.
- **Stable:** `v1.0.0`. The official, signed release.
