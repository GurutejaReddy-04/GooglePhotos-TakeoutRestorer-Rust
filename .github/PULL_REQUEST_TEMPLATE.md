## Description
Briefly describe the changes introduced by this Pull Request.

## Type of Change
- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Performance optimization / Refactoring
- [ ] Documentation update

## Checklist
- [ ] `cargo fmt --all -- --check` has been run and passes cleanly.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` has been run and emits 0 warnings.
- [ ] `cargo test --workspace` passes cleanly.
- [ ] Code follows project architecture rules (no UI logic in `crates/core`).
- [ ] No machine-specific local file paths or credentials have been committed.
