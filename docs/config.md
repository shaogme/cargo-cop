# cargo-cop configuration

[简体中文](config_CN.md) · [Project README](../README.md)

`cargo-cop` reads an optional TOML configuration file named `cargo-cop.toml`. Configuration has schema version `2`. Unknown fields, rule IDs, values, and duplicate feature entries are errors; they are not silently ignored.

## Example

```toml
version = 2
allow_warnings = false

[i18n]
locale = "en-US"

[scan]
targets = "default"
target = "host"
features = ["serde", "logging"]
default_features = true

[rules.no_parent_private_dependency]
enabled = false

[rules.merge_common_prefix_imports]
enabled = true
severity = "warning"

[rules.parameter_limit]
severity = "error"
```

An omitted `[rules.<ID>]` entry keeps that rule enabled at `error` severity. Within a rule table, `enabled` defaults to `true` and `severity` defaults to `error`.

## File discovery and precedence

- Without `--config`, the tool looks for `cargo-cop.toml` at the selected manifest's workspace root. For a standalone package, this is the package directory.
- If no automatically discovered file exists, built-in defaults are used.
- With `--config <PATH>`, only that file is loaded; it does not merge with an automatically discovered file. An explicit path that does not name a regular file is an error.
- A relative `--config` path is resolved from the process's current working directory.
- Command-line settings override matching TOML settings. TOML settings override built-in defaults.
- Configuration settings do not change manifest and package selection. Use CLI options such as `--manifest-path`, `--workspace`, and `--package` for that.

The config file must contain the integer `version = 2`. The top-level fields are `version`, `allow_warnings`, `i18n`, `scan`, and `rules` only. Tables and fields not documented here are rejected.

## Top-level fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `version` | integer | Required | Must be `2`. |
| `allow_warnings` | boolean | `false` | When `true`, warning diagnostics are omitted from the diagnostic list and counted in `summary.suppressed_warnings`. Errors remain visible. This applies to rule and analysis warnings. |
| `[i18n].locale` | non-empty string | Environment locale, then `zh-CN` | Diagnostic and report language. Use `en-US` or `zh-CN`. CLI `--locale` takes precedence. |
| `[scan].targets` | `"default"` or `"all"` | `"default"` | Select default `lib`/`bin`/`proc-macro` targets, or all declared Cargo targets. CLI `--all-targets` selects `all`. |
| `[scan].target` | non-empty string | Unset | Target triple, or `"host"` to use the host triple. CLI `--target` takes precedence. |
| `[scan].features` | array of unique non-empty strings | `[]` | Features requested for selected root packages. CLI `--features` replaces this list when supplied. |
| `[scan].default_features` | boolean | `true` | Whether selected root packages enable their `default` feature. CLI `--no-default-features` can turn this off. |
| `[rules.<ID>].enabled` | boolean | `true` | Whether the named rule runs. CLI `--disable-rule <ID>` can turn it off. |
| `[rules.<ID>].severity` | `"warning"` or `"error"` | `"error"` | Severity assigned to diagnostics from the named rule. CLI `--severity ID=LEVEL` takes precedence. |

`[rules]` may contain any of the 19 IDs in the rule catalog below. Each rule table accepts only `enabled` and `severity`. A setting for one rule has no effect on other rules.

## Rule IDs

```toml
[rules.module_file_layout]
enabled = true
severity = "error"

[rules.parameter_limit]
enabled = true
severity = "warning"
```

Available IDs:

`module_file_layout`, `no_lint_relaxation`, `item_spacing`, `private_child_modules`, `no_parent_private_dependency`, `child_module_line_count`, `impl_locality`, `private_fields_for_behavior_types`, `child_public_free_functions`, `declaration_before_use`, `parameter_limit`, `tuple_arity`, `no_inline_qualified_path`, `merge_common_prefix_imports`, `extern_crate_at_scope_start`, `imports_at_scope_start`, `out_of_line_modules_at_scope_start`, `cfg_use_group_spacing`, and `cfg_inside_use_tree`.

Rule diagnostics can be disabled or changed to `warning`. Tool errors and analysis issues are not rules, so rule settings cannot hide them. An analysis warning can still make the overall scan status `partial` even when `allow_warnings = true` suppresses it from the printed diagnostic list.

## Scan target and feature behavior

When `[scan].targets` is `"default"`, `cargo-cop` selects `lib`, `bin`, and `proc-macro` targets. `"all"` also selects other targets declared in Cargo metadata, such as examples, tests, benches, and build scripts. Targets whose `required-features` are not enabled are skipped.

Target resolution order is:

1. CLI `--target`.
2. `[scan].target`.
3. `CARGO_BUILD_TARGET`.
4. Cargo `build.target` configuration.
5. The host triple reported by `rustc -vV`.

For proc-macro and build-script targets, the host triple is used. A configured target value of `"host"` explicitly selects the host triple.

Feature requests are resolved using feature definitions and dependency declarations from Cargo metadata. The resolver handles default and all features, optional dependencies, dependency feature requests, target-specific dependencies, and feature propagation among selected workspace packages. Unqualified feature names apply to selected packages that declare them; `PACKAGE/FEATURE` can target a particular selected package. A package-qualified feature must name a selected package.

The CLI `--all-features` option is not a TOML field. Use it when all features should be enabled for a particular run.

## Built-in defaults

If no config file is found, the effective defaults are:

```toml
version = 2
allow_warnings = false

[scan]
targets = "default"
features = []
default_features = true
```

`scan.target` is unset by default, so Cargo environment/configuration or the host triple determines the target. The repository's checked-in `cargo-cop.toml` sets `allow_warnings = true`; this is a repository choice, not the built-in default.

## Invalid configuration

The tool exits with an incomplete/error result if the TOML is malformed, `version` is missing or unsupported, a field has the wrong type, a field is unknown, a feature is duplicated, or a rule ID or severity is invalid. It does not fall back to built-in defaults after a configuration error.
