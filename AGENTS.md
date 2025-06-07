# Rust Development Guidelines

This repository is written in Rust and uses Cargo for building and dependency management. Contributors should follow these best practices when working on the project:

1. **Run `cargo fmt` and `cargo clippy`** before committing to ensure consistent code style and catch common mistakes.
2. **Write unit tests** for new functionality. Run `cargo test` in both the root crate and the `validator` crate.
3. **Document public APIs** using Rustdoc comments (`///`) so documentation can be generated with `cargo doc`.
4. **Prefer immutable data** and avoid unnecessary `mut` bindings.
5. **Handle errors with the `Result` type** instead of panicking where feasible.
6. **Use explicit version ranges** in `Cargo.toml` and keep dependencies up-to-date.
7. **Avoid unsafe code** unless absolutely necessary and document any usage clearly.
8. **Keep functions small and focused**; if a function grows too large, consider splitting it into helpers.
9. **Commit messages should be descriptive**, explaining what was changed and why.
10. **Check for `TODO` comments** and convert them into issues if more work is required.

These practices will help maintain a high-quality codebase and make collaboration easier.
