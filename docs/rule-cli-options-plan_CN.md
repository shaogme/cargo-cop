# 规则查询命令选项实现方案

> 本方案已实施。`--list-rules` 和 `--explain-rule <ID>` 可通过 `cargo-cop` 或 `cargo cop` 使用；具体行为和示例见 [README_CN.md](../README_CN.md#命令行选项)。

## 目标

新增两个互斥的 CLI 选项，让用户无需扫描 Cargo package 就能查询规则：

```sh
cargo cop --list-rules
cargo cop --explain-rule parameter_limit
cargo cop --list-rules --locale zh-CN --format json
```

`--list-rules` 输出全部规则 ID 和简短说明；`--explain-rule <ID>` 输出指定规则的完整说明、默认状态和配置方式。两个入口在 `cargo-cop` 和 `cargo cop` 下保持相同。

## 当前实现依据

- CLI 参数和主流程位于 `crates/cargo-cop/src/application.rs`。扫描参数由 `CliOptions` 定义；规则查询在配置发现和 Cargo metadata 查询之前分流，扫描仍继续使用 `ProjectAnalyzer` 与 `RuleEngine`。
- `RuleId`、`RuleId::ALL`、稳定字符串 ID 和 ID 解析位于 `crates/cop-model/src/diagnostic.rs`。规则枚举是规则目录的权威列表。
- 默认规则配置在 `crates/cop-config/src/resolve.rs` 中解析；规则默认启用且级别为 `error`。
- 英文和简体中文文案由 `crates/cop-i18n/schema.u18n`、`locales/en-US.u18n` 和 `locales/zh-CN.u18n` 编译生成；规则目录摘要、完整说明和人类可读字段标签也定义在这些目录中。
- `crates/cop-report/src/lib.rs` 输出扫描诊断及其 schema 版本 `4`，不适合直接承载另一种数据结构的规则目录。
- `README.md` 和 `README_CN.md` 维护规则 ID 与说明；简介和 crate 职责表已与 `RuleId::ALL`、配置文档同步为 21 条规则。

## 命令行为

### 选项与冲突

| 选项 | 行为 |
| --- | --- |
| `--list-rules` | 按 `RuleId::ALL` 顺序列出所有 ID、简短说明、默认启用状态和默认级别。 |
| `--explain-rule <ID>` | 显示单条规则的 ID、完整说明、默认启用状态、默认级别、对应的 `[rules.<ID>]` 配置键，以及 `--disable-rule` / `--severity` 覆盖方式。 |

两个选项互斥，也不能与扫描范围、配置覆盖等扫描参数组合。`--format human|json` 和 `--locale <TAG>` 可与它们组合。未知 ID 使用现有的未知规则参数错误并返回参数错误退出码 `2`。`--help` 和 `--version` 仍按现有优先级处理。

规则查询模式不读取配置文件、不发现 workspace、不调用 Cargo metadata，也不执行源码分析。它们是静态目录查询，默认值描述的是内置默认配置，不是某个项目配置文件中的有效值。用户可通过 `--locale` 或环境 locale 选择语言；规则查询不采用项目配置中的 `[i18n].locale`，避免为查询目录而发现和读取项目配置。

### 输出

- Human 格式下，列表采用稳定 ID 顺序逐条显示；单条说明以 ID 为标题，并清楚标明默认状态和配置入口。
- JSON 格式使用独立的规则目录 schema，不改变扫描诊断 JSON schema `4`。根对象包含 `schema_version: 1` 和 `kind: "rule_catalog"`；列表使用 `rules` 数组，单条说明使用 `rule` 对象。每个规则对象含 `rule_id`、`summary`、`description`、`default_enabled` 和 `default_severity`。
- 输出写到 stdout，成功返回 `0`。规则查询错误仍使用现有工具错误报告路径，且遵循 `--format`。

## 实施步骤

1. **定义本地化规则元数据**：在 `cop-i18n` 的 schema 与英文、简体中文目录中，为每个 `RuleId::ALL` 项提供简短说明和完整介绍。完整介绍应解释触发范围、阈值及例外，不直接复用可能缺少语境的诊断句子。
2. **建立 CLI 规则目录适配层**：新增 `crates/cargo-cop/src/rule_catalog.rs`，以 `RuleId` 为键取得本地化说明，并提供列表和单条查询所需的数据。目录遍历从 `RuleId::all()` 获取，避免另维护一份 ID 列表；该层不依赖扫描器或 Cargo 工程状态。
3. **接入命令参数和分流**：在 `CliOptions` 添加两个参数，检查互斥关系及与扫描专属选项的组合。调整 `Application::run` 的解析与 locale 初始化顺序，使规则查询在项目配置发现和 Cargo metadata 之前分流；扫描模式保留现有配置语言优先级。
4. **增加专用输出**：为 human 与 JSON 规则目录提供单独渲染路径，不修改 `Reporter` 对扫描诊断的 schema。复用当前 locale 选择与输出错误处理。
5. **同步用户文档**：更新两份 README 的命令选项、使用示例和规则说明，加入配置文档到规则查询选项的引用（配置 schema 本身不变）。将文档中的规则数量修正为 21，并检查中英文规则 ID 和说明一致。
6. **补充验收覆盖**：验证两个命令入口、两种 locale、human/JSON 输出、全部 21 个规则及其顺序、已知和未知 ID、参数冲突、没有 Cargo 项目的目录也能查询，以及正常扫描行为不回归。检查英文与中文目录为每个 ID 都提供元数据。

## 预计涉及文件

- `crates/cargo-cop/src/application.rs`
- `crates/cargo-cop/src/main.rs`
- `crates/cargo-cop/src/rule_catalog.rs`（新增）
- `crates/cop-i18n/schema.u18n`
- `crates/cop-i18n/locales/en-US.u18n`
- `crates/cop-i18n/locales/zh-CN.u18n`
- `crates/cargo-cop/tests/acceptance.rs`
- `README.md`、`README_CN.md`、`docs/config.md`、`docs/config_CN.md`

不需要修改 `cop-config` 的 TOML schema 或 `cop-rules` 的扫描规则执行逻辑。后续正式实施并提交前，应按仓库要求运行格式检查、Clippy 和测试套件。
