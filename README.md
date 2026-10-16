# cargo-cop

[![CI](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml/badge.svg)](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml)

[简体中文](README_CN.md)

`cargo-cop` is a static checker for Rust source code. It checks selected Cargo packages and targets against repository conventions, including module layout, visibility, declaration order, imports, and source structure.

The project provides 19 configurable rules, human-readable and JSON reports, and English and Simplified Chinese diagnostic messages. It can run as the `cargo-cop` executable or as the `cargo cop` Cargo subcommand.

## What it checks

- Cargo packages, targets, features, and target-specific `cfg` conditions.
- Rust module files discovered from crate roots, including inline modules, out-of-line modules, and `#[path]` attributes.
- Source conventions represented by the rule catalog below.
- Human-readable or JSON diagnostics, with stable rule IDs and source locations.

`cargo-cop` uses its own lexer, parser, module analysis, and symbol index. It is not a replacement for `rustc` or Clippy and does not compile the checked project. It does not execute build scripts or expand macros. An `include!`, an item macro that may declare modules, an unknown `cfg`, or unsupported Rust syntax can therefore produce a partial or incomplete scan result. Check the reported status when using the result as a CI gate.

## Requirements

- Rust and Cargo 1.75 or later (the workspace MSRV is Rust 1.75).
- `cargo` and `rustc` available on `PATH`.

At runtime the tool queries Cargo metadata and Rust target `cfg` information. It does not build the inspected package or its dependencies.

## Build and install

From a checkout of this repository:

```sh
cargo build -p cargo-cop
cargo run -p cargo-cop -- --help
```

To install the executable in Cargo's binary directory:

```sh
cargo install --path crates/cargo-cop
```

To install a prebuilt release with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall cargo-cop
```

After installation, use either `cargo-cop` directly or `cargo cop`:

```sh
cargo-cop --workspace
cargo cop --workspace
```

Both entry points accept the same options and return the same scan exit codes.

## Quick start

Run from the repository or project you want to inspect:

```sh
cargo cop
```

To inspect this repository from its checkout without installing the binary:

```sh
cargo run -p cargo-cop -- --manifest-path Cargo.toml --workspace
```

Select packages, targets, or an output format as needed:

```sh
cargo cop --manifest-path path/to/Cargo.toml --workspace
cargo cop --manifest-path path/to/Cargo.toml --package my-crate
cargo cop --all-targets --target x86_64-unknown-linux-gnu
cargo cop --features serde,logging --no-default-features
cargo cop --format json > cargo-cop-report.json
```

When the selected manifest is the workspace root, all workspace member packages are selected by default. A package manifest selects that package by default. Use `--workspace` to scan all workspace members from a package manifest, or repeat `--package` to select specific packages. `--workspace` and `--package` cannot be used together.

By default the tool scans `lib`, `bin`, and `proc-macro` targets. `--all-targets` includes all targets declared in the selected packages, such as examples, tests, benches, and build scripts. Targets with unsatisfied `required-features` are skipped.

## Command-line options

| Option | Description |
| --- | --- |
| `--manifest-path <PATH>` | Cargo manifest to inspect. Defaults to `Cargo.toml` in the current directory. |
| `--workspace` | Select all workspace member packages. Conflicts with `--package`. |
| `-p, --package <NAME>` | Select a workspace package; repeat to select multiple packages. |
| `--target <TRIPLE>` | Target triple for target-dependent analysis. `host` selects the host triple. |
| `--features <LIST>` | Enable comma-separated package features; the option may be repeated. `PACKAGE/FEATURE` may qualify a feature. |
| `--all-features` | Enable all features for selected root packages. |
| `--no-default-features` | Disable default features for selected root packages. |
| `--all-targets` | Include every Cargo target declared by selected packages. |
| `--config <PATH>` | Read this TOML configuration instead of automatically discovered configuration. Relative paths are based on the current directory. |
| `--locale <TAG>` | Select diagnostic language, usually `en-US` or `zh-CN`. |
| `--format human\|json` | Choose human-readable or JSON output. Defaults to `human`. |
| `--disable-rule <ID>` | Disable a rule; may be repeated. |
| `--severity <ID=warning\|error>` | Override a rule's severity; may be repeated. |
| `-h, --help` | Print command help. |
| `-V, --version` | Print the tool version. |

Target selection precedence is `--target`, `scan.target` in the config, `CARGO_BUILD_TARGET`, Cargo's `build.target` setting, then the Rust host triple. For proc-macro and build-script targets, the host target is used.

Feature selection follows Cargo feature definitions for selected packages, including default features, optional dependencies, and workspace package feature propagation. The `--features` option replaces the configured feature list when supplied.

## Rules

All rules are enabled by default and use `error` severity unless configured otherwise. See [Configuration](docs/config.md) for settings and overrides.

| Rule ID | Check |
| --- | --- |
| `module_file_layout` | An out-of-line child module file must not use the `mod.rs` layout. Crate roots are exempt. |
| `no_lint_relaxation` | Reject `allow` lint attributes, including `cfg_attr` attributes that inject `allow`. |
| `item_spacing` | Enforce blank-line spacing between adjacent definitions in an item scope. Consecutive `const`, `static`, or type-alias definitions of the same kind require no blank line; other checked pairs require one. |
| `child_module_visibility` | Child modules are private by default. `pub` and `pub(crate)` are allowed only for modules containing nested modules, `use` items with private, `pub`, or `pub(crate)` visibility, `macro_rules!` definitions, type aliases, structs, enums, traits, unions, constants, or statics. |
| `no_parent_private_dependency` | A child module must not import `super::*` or resolve a `super::`/`crate::` path to a private ancestor item. |
| `child_module_line_count` | Each out-of-line child-module file must contain 70–800 effective code lines. Blank lines, comments, and `#[cfg(test)]` code are excluded. |
| `impl_locality` | Keep inherent impls with the repository type's definition; restrict external trait impl targets to the permitted built-in and standard-library roots. |
| `private_fields_for_behavior_types` | Fields of a struct, enum, or union with an impl block must be private. |
| `child_public_free_functions` | Free functions in child modules must be private. |
| `declaration_before_use` | A named declaration must precede references to it within the applicable source scope. |
| `parameter_limit` | Functions, methods, and closures may have at most five parameters. |
| `tuple_arity` | Tuple types and expressions may have at most two elements. |
| `no_inline_qualified_path` | Import external crate names before using them as the root of a qualified path. |
| `merge_common_prefix_imports` | Merge eligible adjacent imports that share a path prefix and have compatible visibility and `cfg` conditions. |
| `extern_crate_at_scope_start` | Keep module-scope `extern crate` declarations at the start of each module scope. |
| `imports_at_scope_start` | Place module-scope `use` declarations after out-of-line modules and before other items; keep block imports before other statements. |
| `out_of_line_modules_at_scope_start` | Keep out-of-line modules together after `extern crate` declarations and before `use` declarations and other items. |
| `cfg_use_group_spacing` | Keep imports with different `cfg` conditions in distinct groups separated by one blank line. |
| `cfg_inside_use_tree` | Put `cfg` attributes on a complete `use` item, not inside a braced use tree. |

Rule IDs are stable configuration keys. Tool errors and analysis issues are not rules and cannot be disabled with `--disable-rule`.

Module-scope order example:

```rust
extern crate legacy_dep as legacy;

mod alpha;

use crate::alpha::Alpha;
use legacy::Thing;

fn run() {}
```

## Reports and exit codes

Human output contains one diagnostic per line followed by a summary. JSON output has schema version `3` and includes the locale, scan status, summary counts, and diagnostics. Each diagnostic includes its kind, severity, rule or issue code, package and target where available, source path and span where available, localized message, and optional namespace or `cfg` condition.

| Exit code | Meaning |
| --- | --- |
| `0` | No error diagnostics and no incomplete scan. Warnings may be present or suppressed. |
| `1` | At least one error-severity diagnostic was found in a complete or partial scan. |
| `2` | The scan is incomplete, or an input, configuration, Cargo, I/O, or output error occurred. |

An analysis warning can mark the scan `partial`; this alone does not make the exit code nonzero. An analysis error or tool error marks the scan incomplete. A rule's severity can be lowered to `warning`, but warnings are only hidden when `allow_warnings = true` is set in the configuration; hidden warnings are counted in `summary.suppressed_warnings`.

The default diagnostic language is selected from `LC_ALL`, `LC_MESSAGES`, and `LANG`, falling back to Simplified Chinese. `--locale` overrides the environment and configured locale; `[i18n].locale` overrides the environment.

## Configuration

The automatically discovered file is `cargo-cop.toml` at the workspace root. An explicit `--config` file replaces automatic discovery; files are not merged. With no configuration file, built-in defaults apply.

See the full [English configuration guide](docs/config.md) or [中文配置说明](docs/config_CN.md). The repository's own `cargo-cop.toml` is a minimal example.

## Workspace architecture

The workspace separates the command-line application from the analysis pipeline:

| Crate | Responsibility |
| --- | --- |
| `cargo-cop` | CLI, configuration and scan orchestration, exit codes. |
| `cop-config` | TOML schema validation, config discovery, and CLI-over-config resolution. |
| `cop-cargo` | Cargo metadata, package and target selection, features, dependencies, and target `cfg`. |
| `cop-syntax` | Project-owned Rust lexer, syntax tree, parser, and parser coverage status. |
| `cop-analysis` | `cfg` evaluation, module graph discovery, symbol and path resolution, analysis status. |
| `cop-rules` | The 19 source convention rules. |
| `cop-model` | Shared rule IDs, diagnostic, location, message, and scan-status types. |
| `cop-report` | Stable diagnostic ordering and human/JSON rendering. |
| `cop-i18n` | English and Simplified Chinese messages. |

The scan flow is: resolve CLI and configuration → query Cargo metadata → select packages and targets → parse reachable module files → evaluate `cfg` and build indexes → run enabled rules → sort and render diagnostics.

## Development

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

The workspace uses Rust 2021 edition and has an MSRV of 1.75. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE) for license terms.
