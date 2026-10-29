# cargo-cop 配置说明

[English](config.md) · [项目 README](../README_CN.md)

`cargo-cop` 可选读取名为 `cargo-cop.toml` 的 TOML 配置文件，配置 schema 版本为 `3`。未知字段、规则 ID、取值以及重复条目都会报错，不会被静默忽略。

## 示例

```toml
version = 3
allow_warnings = false

[i18n]
locale = "zh-CN"

[scan]
targets = "default"
target = "host"
features = ["serde", "logging"]
default_features = true

[[cfg]]
package = "my-crate"
target = "x86_64-unknown-linux-gnu"
name = "custom_mode"
value = "fast"

[rules.no_parent_private_dependency]
enabled = false

[rules.merge_common_prefix_imports]
enabled = true
severity = "warning"

[rules.parameter_limit]
severity = "error"
```

未列出的 `[rules.<ID>]` 仍保持启用，级别为 `error`。规则表中的 `enabled` 默认是 `true`，`severity` 默认是 `error`。

## 文件发现与优先级

- 未使用 `--config` 时，工具会在所选 manifest 的 workspace 根目录查找 `cargo-cop.toml`。独立 package 的 workspace 根目录就是 package 目录。
- 如果自动发现的配置文件不存在，则使用内置默认值。
- 使用 `--config <PATH>` 后只读取指定文件，不会与自动发现的文件合并。显式指定的路径若不存在或不是普通文件，会报错。
- 相对 `--config` 路径以进程当前工作目录为基准解析。
- 命令行设置覆盖对应 TOML 设置；TOML 设置覆盖内置默认值。
- 配置不改变 manifest 和 package 的选择；请用 `--manifest-path`、`--workspace` 和 `--package` 等 CLI 选项设置扫描范围。

配置文件必须包含整数 `version = 3`。顶层只允许 `version`、`allow_warnings`、`i18n`、`scan`、`rules` 和 `cfg`。本文未列出的字段或表都会被拒绝。版本 2 配置须将版本号更新为 3；原有字段语义不变。

可用 [`cargo cop --list-rules`](../README_CN.md#命令行选项) 或 `cargo cop --explain-rule <ID>` 查看内置规则目录。这些静态查询不会读取本文配置文件或其他项目配置；语言由 `--locale` 或环境变量选择，显示的默认值是工具内置默认值。

## 顶层字段

| 字段 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `version` | 整数 | 必填 | 必须是 `3`。 |
| `allow_warnings` | 布尔值 | `false` | 设为 `true` 时，warning 诊断不会出现在诊断列表中，并计入 `summary.suppressed_warnings`；error 仍会显示。此设置同时作用于规则 warning 和分析 warning。 |
| `[i18n].locale` | 非空字符串 | 环境语言；除非明确识别为 `zh`，否则使用英文 | 诊断和报告语言。使用 `en-US` 或 `zh-CN`。命令行 `--locale` 优先。 |
| `[scan].targets` | `"default"` 或 `"all"` | `"default"` | 选择默认 `lib`/`bin`/`proc-macro` targets，或全部 Cargo targets。命令行 `--all-targets` 会选择 `all`。 |
| `[scan].target` | 非空字符串 | 未设置 | Target triple，或使用 `"host"` 选择 host triple。命令行 `--target` 优先。 |
| `[scan].features` | 非空且不重复的字符串数组 | `[]` | 为所选根 package 请求启用的 features。若提供命令行 `--features`，命令行列表会替代此列表。 |
| `[scan].default_features` | 布尔值 | `true` | 是否启用所选根 package 的 `default` feature。命令行 `--no-default-features` 可以将其关闭。 |
| `[[cfg]]` | 表数组 | `[]` | 为 `cfg!`、`#[cfg]`、`cfg_attr` 和 target-specific dependency 提供自定义 cfg 值。每项支持 `package`、`target`、`name` 和可选 `value`。 |
| `[rules.<ID>].enabled` | 布尔值 | `true` | 是否运行该规则。命令行 `--disable-rule <ID>` 可将其关闭。 |
| `[rules.<ID>].severity` | `"warning"` 或 `"error"` | `"error"` | 该规则诊断的级别。命令行 `--severity ID=LEVEL` 优先。 |

`[rules]` 可以包含下方目录中的 21 个 ID。每个规则表只接受 `enabled` 和 `severity`。一条规则的设置不会影响其他规则。

## 规则 ID

```toml
[rules.module_file_layout]
enabled = true
severity = "error"

[rules.parameter_limit]
enabled = true
severity = "warning"
```

可用 ID：

`module_file_layout`、`no_lint_relaxation`、`item_spacing`、`child_module_visibility`、`no_parent_path_attribute`、`no_parent_private_dependency`、`child_module_line_count`、`impl_locality`、`private_fields_for_behavior_types`、`unnecessary_pub_crate`、`child_public_free_functions`、`declaration_before_use`、`parameter_limit`、`tuple_arity`、`no_inline_qualified_path`、`merge_common_prefix_imports`、`extern_crate_at_scope_start`、`imports_at_scope_start`、`out_of_line_modules_at_scope_start`、`cfg_use_group_spacing`、`cfg_inside_use_tree`。

原来的 `private_child_modules` ID 已替换为 `child_module_visibility`。请将 `[rules.private_child_modules]`、`--disable-rule private_child_modules` 和 `--severity private_child_modules=...` 改用新 ID。配置 schema 版本为 `3`。

规则诊断可以关闭或改为 `warning`。工具错误和分析问题不属于规则，因此规则设置不能隐藏它们。即使 `allow_warnings = true` 隐藏了分析 warning，整体扫描状态仍可能是 `partial`。

## 扫描 targets 与 features

`[scan].targets = "default"` 时，`cargo-cop` 选择 `lib`、`bin` 和 `proc-macro` targets。设为 `"all"` 后，还会选择 Cargo metadata 声明的其他 targets，例如 examples、tests、benches 和 build scripts。未启用 `required-features` 的 target 会被跳过。

Target 的解析顺序如下：

1. CLI `--target`。
2. `[scan].target`。
3. `CARGO_BUILD_TARGET`。
4. Cargo `build.target` 配置。
5. `rustc -vV` 报告的 host triple。

proc-macro 和 build-script targets 使用 host triple。配置值 `"host"` 可显式选择 host triple。

Feature 请求根据 Cargo metadata 中的 feature 定义和依赖声明解析。解析器会处理 default 和 all features、可选依赖、依赖 feature 请求、target-specific 依赖，以及所选 workspace package 之间的 feature 传递。未限定 package 的 feature 名会应用于声明了该 feature 的所选 package；`PACKAGE/FEATURE` 可指定某个 package。限定 package 的 feature 必须属于已选 package。

`--all-features` 仅有命令行选项，没有对应 TOML 字段。需要在某次运行启用全部 features 时使用该选项。

`[[cfg]]` 的 `name` 表示 cfg flag 或 key，`value` 表示可选的字符串值。`package` 和 `target` 可单独或同时指定；`target` 可填写 Cargo target 名或 target triple。未指定作用域时，该值应用于所有选中的 package/target。重复 `[[cfg]]` 条目可为同一个 key 配置多个值。CLI 可重复传入 `--cfg name` 或 `--cfg 'name="value"'`；CLI 中出现的 key 会替换该 key 的全部配置文件值，并作为全局值应用。未提供的自定义 cfg 保持 Unknown，产生 `unknown_cfg` warning 并使扫描状态为 `partial`。工具不会运行 build script 来发现自定义 cfg。

## 内置默认值

未找到配置文件时，有效默认值如下：

```toml
version = 3
allow_warnings = false

[scan]
targets = "default"
features = []
default_features = true
```

默认不设置 `scan.target`，由 Cargo 环境/配置或 host triple 决定 target。仓库自身的 `cargo-cop.toml` 设置了 `allow_warnings = true`；这是本仓库的选择，不是内置默认值。

## 无效配置

如果 TOML 格式错误、缺少或使用不支持的 `version`、字段类型不正确、包含未知字段、feature 重复，或规则 ID / severity 无效，工具会返回不完整/错误结果。配置出错后不会回退到内置默认值。
