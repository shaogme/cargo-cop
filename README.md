# cargo-cop

[![CI](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml/badge.svg)](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml)

[简体中文](README_CN.md)

`cargo-cop` is a static checker for Rust source code. It checks selected Cargo packages and targets against repository conventions, including module layout, visibility, declaration order, imports, and source structure.

The project provides 21 configurable rules, human-readable and JSON reports, and English and Simplified Chinese rule descriptions and diagnostic messages. It can run as the `cargo-cop` executable or as the `cargo cop` Cargo subcommand.

## What it checks

- Cargo packages, targets, features, and target-specific `cfg` conditions.
- Rust module files discovered from crate roots, including inline modules, out-of-line modules, and `#[path]` attributes.
- Source conventions represented by the rule catalog below.
- Human-readable or JSON diagnostics, with stable rule IDs and source locations.

`cargo-cop` uses its own lexer, parser, module analysis, and symbol index. After parsing and `cfg` configuration, `cop-analysis` enumerates each module instance's AST nodes once, collecting symbols and local bindings, references, API declaration ranges, impl ownership, coverage regions, and module syntax facts in the same pass. It resolves paths and builds API dependency indexes after the pass, then exposes them through an immutable `AnalysisFacts` snapshot. Semantic and structural rules query these facts for path resolution, item scopes, imports, fields, and syntax shapes. It is not a replacement for `rustc` or Clippy and does not compile the checked project. It does not execute build scripts or expand macros. An `include!`, an item macro that may declare modules, an unknown `cfg`, or unsupported Rust syntax can therefore produce a partial or incomplete scan result. Check the reported status when using the result as a CI gate.

## Requirements

- Rust and Cargo 1.85 or later (the workspace MSRV is Rust 1.85).
- `cargo` and `rustc` available on `PATH`.

At runtime the tool queries Cargo metadata and Rust target `cfg` information. It does not build the inspected package or its dependencies.

`#[cfg(...)]` controls source availability. `cfg!(...)` is a boolean expression: its value is recorded without removing code, so both branches of `if cfg!(...)` remain in normal analysis. The tool evaluates only `cfg!` calls it can confirm as the built-in macro. It does not execute build scripts, so custom cfg values injected by a build script must be supplied with repeated `--cfg name` or `--cfg 'name="value"'` options, or with `[[cfg]]` entries in the config file. An unprovided custom value remains Unknown.

## Install

```sh
cargo install cargo-cop
```

To install a prebuilt release with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall cargo-cop
```

### Install in GitHub Actions

Use this action in a workflow to download and install the latest prebuilt release by default:

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
  - run: cargo cop
```

`version` defaults to `latest`; set it to a cargo-cop release to install a specific version. The version may include or omit the `v` prefix. `from-source` defaults to `false`; set it to `true` to build the selected release tag from source. The action installs the stable Rust toolchain for source builds. To install a pinned prebuilt release:

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
    with:
      version: 0.2.4
  - run: cargo cop --workspace
```

To build that version from source:

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
    with:
      version: 0.2.4
      from-source: true
  - run: cargo cop --workspace
```

The action adds its installation directory to `PATH` for subsequent steps and exposes the installed version through its `version` output.

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

Browse the rule catalog without selecting or scanning a Cargo package:

```sh
cargo cop --list-rules
cargo cop --explain-rule parameter_limit
cargo cop --list-rules --locale zh-CN --format json
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
| `--cfg <CFG>` | Provide a custom cfg flag (for example `custom_mode`) or key/value (for example `custom_mode="fast"`); repeat the option as needed. |
| `--all-features` | Enable all features for selected root packages. |
| `--no-default-features` | Disable default features for selected root packages. |
| `--all-targets` | Include every Cargo target declared by selected packages. |
| `--config <PATH>` | Read this TOML configuration instead of automatically discovered configuration. Relative paths are based on the current directory. |
| `--locale <TAG>` | Select diagnostic language, usually `en-US` or `zh-CN`. |
| `--format human\|json` | Choose human-readable or JSON output. Defaults to `human`. |
| `--list-rules` | List all rule IDs, summaries, and built-in defaults without scanning a project. |
| `--explain-rule <ID>` | Show the full description and configuration options for one rule without scanning a project. |
| `--disable-rule <ID>` | Disable a rule; may be repeated. |
| `--severity <ID=warning\|error>` | Override a rule's severity; may be repeated. |
| `-h, --help` | Print command help. |
| `-V, --version` | Print the tool version. |

`--list-rules` and `--explain-rule` are mutually exclusive and cannot be combined with scan-selection or configuration-override options. They can be combined with `--format` and `--locale`. Rule queries use the CLI or environment locale and do not read project configuration, discover a workspace, or query Cargo metadata. Their default values describe the built-in defaults. Rule-catalog JSON uses schema version `1`, separate from diagnostic JSON schema version `4`.

Target selection precedence is `--target`, `scan.target` in the config, `CARGO_BUILD_TARGET`, Cargo's `build.target` setting, then the Rust host triple. For proc-macro and build-script targets, the host target is used.

Feature selection follows Cargo feature definitions for selected packages, including default features, optional dependencies, and workspace package feature propagation. The `--features` option replaces the configured feature list when supplied.

## Rules

All rules are enabled by default and use `error` severity unless configured otherwise. Use [`--list-rules`](#command-line-options) and [`--explain-rule <ID>`](#command-line-options) to browse rule descriptions. See [Configuration](docs/config.md) for settings and overrides.

| Rule ID | Check |
| --- | --- |
| `module_file_layout` | An out-of-line child module file must not use the `mod.rs` layout. Crate roots are exempt. |
| `no_lint_relaxation` | Reject `allow` lint attributes, including `cfg_attr` attributes that inject `allow`. |
| `item_spacing` | Enforce blank-line spacing between adjacent structs, enums, unions, type aliases, traits, constants, statics, functions, and impl blocks in an item scope. Every checked pair requires one blank line, including impl blocks next to any listed kind; consecutive constants, statics, or type aliases of the same kind require none. |
| `child_module_visibility` | Child modules are private by default. `pub` and `pub(crate)` are allowed only for modules containing nested modules, `use` items with private, `pub`, or `pub(crate)` visibility, `macro_rules!` definitions, type aliases, structs, enums, traits, unions, constants, or statics. |
| `no_parent_path_attribute` | An external module's `#[path]` attribute must not contain a `..` path component, including paths nested in `cfg_attr`. |
| `no_parent_private_dependency` | A child module must not import `super::*` or resolve a `super::`/`crate::` path to a private ancestor item. |
| `child_module_line_count` | Each out-of-line child-module file must contain 70–800 effective code lines. Blank lines, comments, and `#[cfg(test)]` code are excluded. |
| `impl_locality` | Keep inherent impls with the repository type's definition; restrict external trait impl targets to the permitted built-in and standard-library roots. |
| `private_fields_for_behavior_types` | Fields of a struct, enum, or union with an impl block must be private. |
| `unnecessary_pub_crate` | A `pub(crate)` item or associated item is retained when an outside reference or a retained crate-visible API's visible type interface requires it. Dependencies propagate through signatures, constraints, aliases, visible fields, enum payloads, and supported foreign items; function bodies and private fields do not count. Unresolved syntax and macro invocations are handled conservatively. |
| `child_public_free_functions` | Free functions in child modules must be private. |
| `declaration_before_use` | A named declaration must precede references to it within the applicable source scope. |
| `parameter_limit` | Functions, methods, and closures may have at most five parameters. |
| `tuple_arity` | Tuple types and expressions may have at most two elements. |
| `no_inline_qualified_path` | Report code paths rooted at an external crate name (including aliases) or explicitly at a module scope with `crate::`, `self::`, or `super::`; import the required names before use. |
| `merge_common_prefix_imports` | Merge imports that share a path prefix and have matching visibility and effective `cfg` conditions; module-scope imports need not be adjacent. |
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

Human output contains one diagnostic per line followed by a summary. JSON output has schema version `4` and includes the locale, scan status, summary counts, and diagnostics. Each diagnostic includes its kind, severity, rule or issue code, package and target where available, source path and span where available, a pre-rendered localized message, optional detail, and optional namespace or `cfg` condition.

| Exit code | Meaning |
| --- | --- |
| `0` | No error diagnostics and no incomplete scan. Warnings may be present or suppressed. |
| `1` | At least one error-severity diagnostic was found in a complete or partial scan. |
| `2` | The scan is incomplete, or an input, configuration, Cargo, I/O, or output error occurred. |

An analysis warning can mark the scan `partial`; this alone does not make the exit code nonzero. An analysis error or tool error marks the scan incomplete. A rule's severity can be lowered to `warning`, but warnings are only hidden when `allow_warnings = true` is set in the configuration; hidden warnings are counted in `summary.suppressed_warnings`.

The default diagnostic language is selected from `LC_ALL`, `LC_MESSAGES`, and `LANG` in that order. Chinese is selected only when the selected locale identifies the `zh` language; other locales, including `C` and `POSIX`, use English. If no environment locale can be identified, English is used. `--locale` overrides the environment and configured locale; `[i18n].locale` overrides the environment.

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
| `cop-cfg` | Shared `cfg` expression parsing, normalization, and three-valued evaluation. |
| `cop-syntax` | Project-owned Rust lexer, syntax tree, parser, and parser coverage status. |
| `cop-analysis` | `cfg` evaluation, module graph discovery, immutable definition/reference facts, path resolution, API dependency and impl-owner indexes, analysis status. |
| `cop-rules` | The 21 source convention rules. |
| `cop-model` | Shared rule IDs, diagnostics, source locations, and scan-status types. |
| `cop-report` | Stable diagnostic ordering and human/JSON rendering. |
| `cop-i18n` | English and Simplified Chinese messages. |

The scan flow is: resolve CLI and configuration → query Cargo metadata → select packages and targets → parse reachable module files → collect definitions and syntax references → resolve references and build immutable indexes → run enabled rules against facts and source locations → sort and render diagnostics. Syntax and layout rules still read the parsed tree for structural checks.

## Development

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

The cargo-cop workspace uses Rust 2021 edition and has an MSRV of 1.85. Diagnostic messages are compiled from English and Simplified Chinese catalogs with u18n. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE) for license terms.
