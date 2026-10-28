# cargo-cop

[![CI 状态](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml/badge.svg)](https://github.com/shaogme/cargo-cop/actions/workflows/ci.yml)

[English](README.md)

`cargo-cop` 是一个 Rust 源码静态检查工具。它会按照仓库规范检查选定的 Cargo package 和 target，覆盖模块布局、可见性、声明顺序、导入组织及源码结构等方面。

项目提供 20 条可配置规则、人类可读和 JSON 报告，以及英文和简体中文诊断信息。既可以直接运行 `cargo-cop`，也可以使用 Cargo 子命令 `cargo cop`。

## 检查内容

- Cargo package、target、features 和 target 特定的 `cfg` 条件。
- 从 crate 根文件发现的 Rust 模块，包括 inline module、外部模块和 `#[path]` 属性。
- 下方规则目录列出的源码规范。
- 带稳定规则 ID 和源码位置的人类可读或 JSON 诊断。

`cargo-cop` 使用自有词法分析器、解析器、模块分析和符号索引。解析和 cfg 配置完成后，`cop-analysis` 会为每个模块实例只枚举一次 AST 节点，并在同一轮收集符号与局部绑定、引用、API 声明范围、impl 所有者、覆盖状态和模块语法事实。遍历结束后再统一解析路径并构建 API 依赖等索引，生成不可变的 `AnalysisFacts`；语义规则与结构规则查询这些事实处理路径、作用域、导入、字段和语法形状。它不替代 `rustc` 或 Clippy，也不会编译被检查的项目；它不会执行 build script 或展开宏。因此，`include!`、可能声明模块的 item macro、未知 `cfg` 或尚不支持的 Rust 语法可能导致扫描结果为部分完成或不完整。将扫描结果用于 CI 门禁时，请检查报告中的状态。

## 环境要求

- Rust 和 Cargo 1.75 或更高版本（workspace 的最低支持版本为 Rust 1.75）。
- `cargo` 和 `rustc` 可从 `PATH` 调用。

运行时工具会查询 Cargo 元数据和 Rust target 的 `cfg` 信息，但不会构建被检查的 package 或其依赖。

`#[cfg(...)]` 控制源码可用性；`cfg!(...)` 是布尔表达式，只记录求值结果，不会删除代码，因此 `if cfg!(...)` 的两个分支都会继续接受常规分析。工具只对可确认的内置 `cfg!` 求值。它不会执行 build script；build script 注入的自定义 cfg 值需通过重复的 `--cfg name`、`--cfg 'name="value"'` 参数或配置文件中的 `[[cfg]]` 显式提供。未提供的自定义值会报告为 Unknown。

## 构建与安装

```sh
cargo install cargo-cop
```

也可以通过 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 安装预编译的 release：

```sh
cargo binstall cargo-cop
```

### 在 GitHub Actions 中安装

在 workflow 中使用此 action 会默认下载并安装最新 release 的预编译二进制：

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
  - run: cargo cop
```

`version` 默认是 `latest`，也可指定 cargo-cop release 版本；版本号可带或不带 `v` 前缀。`from-source` 默认为 `false`，设置为 `true` 后会从所选 release tag 的源码构建（action 会安装 stable Rust 工具链）。固定版本并下载预编译二进制：

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
    with:
      version: 0.2.2
  - run: cargo cop --workspace
```

如需从同一版本的源码构建：

```yaml
steps:
  - uses: actions/checkout@main
  - uses: shaogme/cargo-cop@main
    with:
      version: 0.2.2
      from-source: true
  - run: cargo cop --workspace
```

action 会将安装目录加入后续步骤的 `PATH`，并通过 `version` output 暴露实际安装的版本。

安装后可直接运行 `cargo-cop`，也可使用 Cargo 子命令 `cargo cop`：

```sh
cargo-cop --workspace
cargo cop --workspace
```

两种入口接受相同选项，并返回相同扫描退出码。

## 快速开始

在要检查的仓库或项目目录中运行：

```sh
cargo cop
```

在本仓库 checkout 中，也可以不安装直接检查本仓库：

```sh
cargo run -p cargo-cop -- --manifest-path Cargo.toml --workspace
```

按需选择 package、target 或输出格式：

```sh
cargo cop --manifest-path path/to/Cargo.toml --workspace
cargo cop --manifest-path path/to/Cargo.toml --package my-crate
cargo cop --all-targets --target x86_64-unknown-linux-gnu
cargo cop --features serde,logging --no-default-features
cargo cop --format json > cargo-cop-report.json
```

当指定的 manifest 是 workspace 根 manifest 时，默认选择 workspace 中的全部成员 package。指定 package manifest 时，默认只选择该 package。使用 `--workspace` 可从 package manifest 扫描整个 workspace；也可以重复使用 `--package` 选择特定 package。`--workspace` 和 `--package` 不能同时使用。

默认扫描 `lib`、`bin` 和 `proc-macro` targets。`--all-targets` 会纳入所选 package 声明的所有 targets，例如 examples、tests、benches 和 build scripts。未启用其 `required-features` 的 target 会被跳过。

## 命令行选项

| 选项 | 说明 |
| --- | --- |
| `--manifest-path <PATH>` | 要检查的 Cargo manifest。默认读取当前目录下的 `Cargo.toml`。 |
| `--workspace` | 选择 workspace 中的全部成员 package；不能与 `--package` 同用。 |
| `-p, --package <NAME>` | 选择 workspace package；可重复指定多个 package。 |
| `--target <TRIPLE>` | target 相关分析使用的 triple。`host` 表示当前 host triple。 |
| `--features <LIST>` | 启用逗号分隔的 package features；可重复指定。可用 `PACKAGE/FEATURE` 限定 package。 |
| `--cfg <CFG>` | 提供自定义 cfg flag（如 `custom_mode`）或 key/value（如 `custom_mode="fast"`）；可重复指定。 |
| `--all-features` | 为所选根 package 启用全部 features。 |
| `--no-default-features` | 为所选根 package 禁用 default features。 |
| `--all-targets` | 纳入所选 package 声明的所有 Cargo targets。 |
| `--config <PATH>` | 读取指定 TOML 配置文件，替代自动发现的配置。相对路径以当前目录为基准。 |
| `--locale <TAG>` | 选择诊断语言，常用值为 `zh-CN` 或 `en-US`。 |
| `--format human\|json` | 选择人类可读或 JSON 输出，默认 `human`。 |
| `--disable-rule <ID>` | 关闭一条规则；可重复指定。 |
| `--severity <ID=warning\|error>` | 覆盖规则级别；可重复指定。 |
| `-h, --help` | 显示命令帮助。 |
| `-V, --version` | 显示工具版本。 |

Target 选择优先级依次为：`--target`、配置中的 `scan.target`、`CARGO_BUILD_TARGET`、Cargo 的 `build.target` 设置、Rust host triple。proc-macro 和 build-script targets 使用 host target。

Feature 选择根据所选 package 的 Cargo feature 定义计算，处理 default features、可选依赖和 workspace package 间的 feature 传递。如果命令行提供了 `--features`，它会替代配置中的 feature 列表。

## 规则

默认启用全部规则，级别默认为 `error`。规则配置和覆盖方式见[配置说明](docs/config_CN.md)。

| 规则 ID | 检查内容 |
| --- | --- |
| `module_file_layout` | 外部子模块文件不能使用 `mod.rs` 布局；crate 根文件不受此项限制。 |
| `no_lint_relaxation` | 禁止 `allow` lint 属性，包括通过 `cfg_attr` 注入的 `allow`。 |
| `item_spacing` | 检查同一 item 作用域内相邻的结构体、枚举、union、类型别名、trait、常量、静态变量、函数和 impl 块之间的空行。所有受检组合之间留一行，包括 impl 块与其他列出类型的组合；同种的连续常量、静态变量或类型别名之间不留空行。 |
| `child_module_visibility` | 子模块默认私有；仅含嵌套模块、私有、`pub` 或 `pub(crate)` `use` 导入、`macro_rules!` 定义、类型别名、结构体、枚举、trait、union、常量或静态变量时，才可使用 `pub` 或 `pub(crate)`。 |
| `no_parent_path_attribute` | 外部模块的 `#[path]` 属性不能包含 `..` 路径分量，包括嵌套在 `cfg_attr` 中的路径属性。 |
| `no_parent_private_dependency` | 子模块不能使用 `super::*`，也不能通过 `super::` 或 `crate::` 路径访问祖先模块的私有 item。 |
| `child_module_line_count` | 每个外部子模块文件须有 70–800 行有效代码；空行、注释和 `#[cfg(test)]` 代码不计入。 |
| `impl_locality` | 固有 impl 应与仓库类型定义放在同一文件；外部 trait impl 的目标仅限允许的内置类型和标准库根路径。 |
| `private_fields_for_behavior_types` | 有 impl 块的 struct、enum 或 union，其字段必须保持私有。 |
| `unnecessary_pub_crate` | 只有当私有模块范围之外的引用或仍保留的 crate 可见 API 的可见类型接口需要时，item 或关联 item 才保留 `pub(crate)`。依赖会沿函数签名、约束、别名、可见字段、枚举 payload 和受支持的 foreign item 传递；函数体和私有字段不计入。遇到无法解析的语法或宏调用时会保守处理。 |
| `child_public_free_functions` | 子模块中的独立函数必须保持私有。 |
| `declaration_before_use` | 在适用的源码作用域中，具名声明必须先于对它的引用。 |
| `parameter_limit` | 函数、方法和闭包的参数数量最多为 5 个。 |
| `tuple_arity` | tuple 类型和表达式最多包含 2 个元素。 |
| `no_inline_qualified_path` | 报告代码中以外部 crate 名（含别名）或显式模块路径 `crate::`、`self::`、`super::` 为根的限定路径；使用前应导入所需名称。 |
| `merge_common_prefix_imports` | 合并路径有共同前缀且可见性和有效 `cfg` 条件相同的导入；模块作用域中的导入不要求相邻。 |
| `extern_crate_at_scope_start` | 每个模块作用域中的 `extern crate` 声明必须位于开头。 |
| `imports_at_scope_start` | 模块作用域中的 `use` 位于外部模块声明之后、其他 item 之前；block 导入放在其他语句之前。 |
| `out_of_line_modules_at_scope_start` | 外部模块声明位于 `extern crate` 之后、`use` 和其他 item 之前，并连续排列。 |
| `cfg_use_group_spacing` | 不同 `cfg` 条件的导入应分组排列，组间留一行空行。 |
| `cfg_inside_use_tree` | `cfg` 属性应放在完整的 `use` item 上，不能嵌套在花括号 use tree 中。 |

规则 ID 是稳定的配置键。工具错误和分析问题不是规则，不能通过 `--disable-rule` 关闭。

模块作用域的顺序示例：

```rust
extern crate legacy_dep as legacy;

mod alpha;

use crate::alpha::Alpha;
use legacy::Thing;

fn run() {}
```

## 报告与退出码

人类可读输出每条诊断占一行，最后附带汇总信息。JSON 输出使用 schema 版本 `3`，包含 locale、扫描状态、汇总计数和诊断数组。每条诊断在适用时包含诊断类型、级别、规则或问题代码、package、target、源码路径和范围、本地化消息，以及 namespace 或 `cfg` 条件。

| 退出码 | 含义 |
| --- | --- |
| `0` | 没有 error 诊断且没有不完整扫描；可以存在 warning，也可以抑制 warning。 |
| `1` | 完整或部分扫描中发现至少一条 `error` 级别诊断。 |
| `2` | 扫描不完整，或发生输入、配置、Cargo、I/O、输出错误。 |

分析 warning 会将状态标记为 `partial`，但单独出现时不会导致非零退出码。分析 error 或工具错误会使扫描不完整。规则可以降为 `warning`；只有配置 `allow_warnings = true` 时，warning 诊断才会从输出中隐藏，隐藏数量记入 `summary.suppressed_warnings`。

默认诊断语言按 `LC_ALL`、`LC_MESSAGES`、`LANG` 的顺序选择。仅当选中的 locale 明确标识 `zh` 语言时使用简体中文；其他 locale（包括 `C` 和 `POSIX`）使用英文。若无法识别任何环境 locale，也回退到英文。`--locale` 优先级高于环境和配置；`[i18n].locale` 优先于环境变量。

## 配置

自动发现的配置文件是 workspace 根目录下的 `cargo-cop.toml`。显式指定 `--config` 后只读取该文件，不会与自动发现的配置合并。若不存在配置文件，则使用内置默认值。

完整说明见[中文配置文档](docs/config_CN.md)或[英文配置文档](docs/config.md)。仓库根目录的 `cargo-cop.toml` 是一个最小示例。

## Workspace 架构

Workspace 将命令行应用与分析流程分层：

| Crate | 职责 |
| --- | --- |
| `cargo-cop` | CLI、配置与扫描流程编排、退出码。 |
| `cop-config` | TOML schema 校验、配置发现、CLI 覆盖解析。 |
| `cop-cargo` | Cargo 元数据、package 和 target 选择、features、依赖和 target `cfg`。 |
| `cop-cfg` | 共享 `cfg` 表达式解析、规范化和三态求值。 |
| `cop-syntax` | 自有 Rust 词法分析器、语法树、解析器和解析覆盖状态。 |
| `cop-analysis` | `cfg` 求值、模块图发现、不可变定义/引用事实、路径解析、API 依赖和 impl 所有者索引、分析状态。 |
| `cop-rules` | 20 条源码规范检查规则。 |
| `cop-model` | 共享规则 ID、诊断、位置、消息和扫描状态类型。 |
| `cop-report` | 诊断稳定排序、人类可读和 JSON 渲染。 |
| `cop-i18n` | 英文和简体中文消息。 |

扫描流程为：解析 CLI 和配置 → 查询 Cargo 元数据 → 选择 package 与 target → 解析可达模块文件 → 收集定义和语法引用 → 解析引用并构建不可变索引 → 规则查询事实与源码位置 → 排序并渲染诊断。语法和布局规则仍会读取解析后的语法树来检查结构。

## 开发

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Workspace 使用 Rust 2021 edition，最低支持 Rust 1.75。许可证条款见 [LICENSE-MIT](LICENSE-MIT) 和 [LICENSE-APACHE](LICENSE-APACHE)。
