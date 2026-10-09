# Contributing to 0xTerm

Thanks for helping. 0xTerm stays small on purpose, so please open an issue
to talk about a new feature before you build it. Bug fixes are welcome
straight away.

## Reporting a bug

Open an [issue](https://github.com/allocmemory/0xTerm/issues) with:

- your macOS version and Mac model
- the 0xTerm version (Finder → Get Info on 0xTerm.app), or the commit for a source build
- what you did, what you expected, and what happened
- your `~/.config/0xterm/config.toml`, if you have one

## Building

See [Build from source](README.md#build-from-source). In short: Rust stable,
Xcode with the Metal toolchain, then `cargo run -p oxterm`.

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep each pull request to one change, and target `main`.
- Match the surrounding code: its naming, comment style and module layout.
  Comments explain why, not what.
- Respect the crate boundaries described in the README. `oxt-task` and
  `oxt-session` have no UI dependency, and the toolbelt talks to panes only
  through events.
- Add a test when the change is testable without a window.
- Commit messages use [Conventional Commits](https://www.conventionalcommits.org),
  for example `fix(toolbelt): keep the tree open after a rename`.

## License

By contributing, you agree that your contributions are licensed under the
[GPL-3.0](LICENSE), like the rest of 0xTerm.
