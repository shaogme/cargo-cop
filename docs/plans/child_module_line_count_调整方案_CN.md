# `child_module_line_count` 调整方案

## 目标

调整 `child_module_line_count`：外部子模块源文件如果拥有至少一个直接的非 inline 子模块，则不检查 70 行下限；文件仍不得超过 800 行有效代码。没有直接非 inline 子模块的文件继续执行 70–800 行限制。

这是规则行为的有意变更，不增加兼容开关或配置项。规则 ID、启用状态和级别配置保持不变。

## 当前实现

- `crates/cop-rules/src/module_rules.rs` 中的 `child_module_line_count` 跳过 crate 根、inline 和 order-only 模块，将同一 package、target、target triple、namespace 和文件路径的模块实例分组，再选一个代表实例计数。
- 行数来自 `AnalysisFacts` 中的 `ModuleSyntaxFacts::code_line_count()`；`crates/cop-analysis/src/syntax_facts.rs` 负责排除空行、注释、禁用 cfg 内容及 `#[cfg(test)]` 范围。
- 当前诊断对所有模块实例都使用 70 和 800 两个边界。
- `ModuleGraph` 在 `crates/cop-analysis/src/module_graph.rs` 中建立实际解析到的模块节点。节点提供 `is_inline`、`is_order_only`、`context_index`、`namespace` 和 `parent_namespace`，可区分同一文件的直接外部子模块与 inline 模块中的后代。
- 现有验收用例覆盖边界、cfg、test target、target triple 和 namespace 去重，主要位于 `crates/cargo-cop/tests/acceptance.rs`。

## 规则语义

1. 检查对象仍为非 crate 根、非 inline、非 order-only 的外部模块文件。
2. “存在非 inline 子模块”限定为：模块图中存在一个已加载、非 inline、非 order-only 节点，其 `parent_namespace` 等于当前模块的 `namespace`，且属于同一个模块上下文。只检查直接子模块；inline 后代中的外部模块不使祖先文件豁免下限。
3. 对当前 `ModuleLineCountKey` 已合并的多个 cfg/test 模块实例，只要其中任一实例在其上下文中有这样的直接子模块，该文件实例就豁免下限。仍按现有逻辑选取代表实例计算行数，保持一组上下文只报告一次的行为。
4. 有直接非 inline 子模块时，只有有效行数大于 800 才报告；否则不报告。没有直接非 inline 子模块时，继续仅在有效行数小于 70 或大于 800 时报告。
5. 仅有 inline 子模块不豁免下限。cfg 排除或无法解析、因而没有进入模块图的子模块不触发豁免；未知 cfg 若被分析器作为可用模块加载，则按已存在处理。

通过检查模块图中的子节点而非单独查找 `mod name;` 语法，可确保豁免对应实际解析到的外部子模块，并正确处理 `#[path]`、cfg 与 inline 命名空间。

## 实施范围

### 规则实现与诊断

修改 `crates/cop-rules/src/module_rules.rs` 的分组检查：以现有模块上下文和 `parent_namespace` 关系，判断该分组是否有直接非 inline 子模块；然后分别应用“仅最大值”或“最小值与最大值”条件。代表模块的选择、行数计算、规则配置读取和诊断位置保持现状。

当有子模块的文件超过上限时，诊断应明确表达“最多 800 行”，避免继续显示“70 到 800 行”。建议在 `crates/cop-i18n/schema.u18n` 添加最大值专用消息，并在 `crates/cop-i18n/locales/en-US.u18n` 和 `crates/cop-i18n/locales/zh-CN.u18n` 提供对应文案；无子模块时继续使用现有区间消息。

### 规则说明

- 更新 `AGENTS.md` 的子模块有效行数约束，写明有直接非 inline 子模块时只受 800 行上限约束。
- 更新 `README.md`、`README_CN.md` 的规则摘要，以及两种 locale 的 `rule_catalog.child_module_line_count.summary` 和 `description`。
- `docs/config.md` 与 `docs/config_CN.md` 仅列出规则 ID 和通用配置格式，没有单独的该规则说明；因本方案不增加配置字段，无需变更配置 schema 或版本。

### 验收用例

在 `crates/cargo-cop/tests/acceptance.rs` 补充或调整集成用例，验证：

- 无子模块文件的 69、70、800、801 行边界保持现状。
- 有直接外部子模块文件的少于 70 行和恰好 70 行均不报告；800 行不报告，801 行报告最大值专用诊断。
- 只有 inline 子模块的文件仍执行 70 行下限；inline 作用域中的外部后代不豁免祖先文件。
- cfg 禁用的外部子模块不触发豁免；仅 test 上下文启用的子模块按分组语义触发豁免，并继续只产生一次诊断/结果。
- `#[path]` 外部子模块、多个 target triple、target 和 namespace 的去重行为保持正确。
- 已声明但缺少源文件的外部子模块不触发豁免，并由现有模块分析诊断反映解析失败。

## 验收标准

- 直接外部子模块存在时，下限被移除，上限 800 行仍包含在允许范围内。
- 其余文件继续遵循闭区间 `[70, 800]`。
- 中英文诊断准确表达对应边界，规则目录、README 和 `AGENTS.md` 与实现一致。
- 现有 cfg、target、test 上下文和模块去重行为不回退。

本文件只记录实现方案；尚未修改规则实现、测试或现有说明文件。
