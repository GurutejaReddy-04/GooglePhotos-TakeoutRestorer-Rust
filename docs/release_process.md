# Release Process

1. **Feature Freeze:** Code is frozen in the `main` branch.
2. **Release Candidate:** Tag a commit as `vX.Y.Z-rc.1`.
3. **CI Pipeline:** GitHub Actions will automatically:
   - Run formatting (`cargo fmt`), linting (`cargo clippy`), and tests (`cargo test`).
   - Run security and license audits (`cargo-deny` checking RustSec advisories and crate licensing).
   - Compile optimized binaries (`cargo build --release`).
   - Package native bundles (`cargo packager`).
   - Generate CycloneDX SBOM.
4. **Validation:** Manually install and execute the generated bundles on target operating systems.
5. **Final Release:** If tests pass and validation succeeds, tag as `vX.Y.Z`.
