item-spacing = 相邻定义之间应有 { $expected } 个空行，当前有 { $actual } 个
module-file-layout = 子模块入口文件不能使用 mod.rs 布局
out-of-line-modules-at-scope-start = 外部模块声明必须位于 extern crate 声明之后、use 声明和其他 item 之前，并连续排列
child-module-visibility = 非私有子模块 { $module } 只能包含声明项，以及私有、公开或 pub(crate) 可见性的 use 导入
child-module-line-count = 子模块文件 { $file } 的有效代码行数为 { $actual }，要求在 { $minimum } 到 { $maximum } 行之间
no-parent-glob-import = 子模块不得使用 super::* 导入父模块实现
no-parent-private-path = 子模块路径依赖了祖先模块 { $ancestor } 的私有项 { $item }
impl-cross-file = impl 目标 { $target } 定义在其他源文件，impl 必须与类型定义位于同一文件
impl-external-trait-target = trait impl 的目标 { $root } 不属于允许的标准库类型
impl-inherent-target = 固有 impl 的目标必须是在当前源文件定义的类型
impl-target-unresolved = 无法解析 impl 目标 { $target } 的类型归属
private-fields-for-behavior-type = 存在 impl 的类型 { $name } 必须将所有字段设为私有
child-public-free-function = 子模块中的独立函数必须保持私有
no-lint-relaxation = 禁止通过 allow 属性放宽 lint 检查
function-parameter-limit = 函数参数数量为 { $actual }，最多允许 { $maximum } 个
closure-parameter-limit = 闭包参数数量为 { $actual }，最多允许 { $maximum } 个
tuple-arity = tuple 元素数量为 { $actual }，最多允许 { $maximum } 个
declaration-before-use = 名称 { $name } 在其声明之前被引用
no-inline-qualified-path = 路径 { $path } 直接以外部 crate 为根，应先导入所需名称
extern-crate-at-scope-start = extern crate 声明必须位于模块作用域开头
use-at-scope-start = 模块作用域中的 use 声明必须位于外部模块声明之后、其他 item 之前
block-use-at-scope-start = block 作用域中的 use 声明必须位于首个非 use 语句之前
merge-import-prefix = 此导入与第 { $line } 行的导入共享前缀 { $prefix }，可合并为一条 use tree
merge-block-import-prefix = 此 block 导入与第 { $line } 行共享前缀 { $prefix }，可合并为一条 use tree
cfg-use-group-split = cfg 条件组 { $condition } 被其他导入组拆开
cfg-use-group-spacing = 不同 cfg 导入组之间应恰好有一个空行，当前有 { $empty_lines } 个
cfg-inside-use-tree = use tree 花括号内部不能嵌套 cfg 属性
cfg-dependency-unknown = 依赖 { $dependency } 的 target 条件无法静态确定
include-dynamic-input = include! 会在编译时引入源码，静态模块图无法确认其内容
macro-generated-module = 项级宏 { $name } 未展开，可能生成模块声明
module-cycle = 模块 { $name } 再次载入当前递归链中的源文件
path-module-missing = #[path] 指定的模块文件不存在
module-layout-ambiguous = 模块 { $name } 同时匹配 name.rs 和 name/mod.rs 布局
module-missing = 找不到模块 { $name }；已检查 name.rs 和 name/mod.rs 布局
module-source-read-error = 读取 Rust 源文件失败 { $path}：{ $detail }
module-file-metadata-error = 检查模块文件失败 { $path}：{ $detail }
parser-coverage = { $issue }
malformed-cfg-attribute = cfg/path 属性无法可靠解释，相关模块分析可能不完整
unknown-cfg = 无法静态确定 cfg 条件 { $condition }
syntax-unclosed-block-comment = 块注释缺少结束符 */
syntax-unknown-character = 无法识别字符 { $character }
syntax-unclosed-raw-string = raw 字符串缺少结束符
syntax-unclosed-literal = 字面量缺少结束引号
syntax-expect-literal-or-lifetime = 单引号后需要字符字面量或生命周期名称
syntax-invalid-byte-character = byte 字符字面量格式无效
syntax-expect-radix-digit = 基数前缀后需要至少一个数字
syntax-invalid-radix-digit = 数字字面量包含无效的 { $radix } 进制数字
syntax-inner-attribute-position = 内层属性必须位于作用域开头
syntax-expect-attribute-bracket = 属性标记 # 后需要方括号
syntax-attribute-unclosed = 属性缺少结束方括号
syntax-visibility-unclosed = 可见性限定缺少结束括号
syntax-expect-module-name = mod 后需要模块名称
syntax-expect-module-body = mod 声明需要分号或模块体
syntax-inline-module-unclosed = inline module 缺少结束花括号
syntax-use-missing-semicolon = use 声明缺少分号
syntax-extern-crate-missing-semicolon = extern crate 声明需要分号
syntax-named-item-expect-name = 具名声明关键字后需要名称
syntax-trait-expect-name = trait 后需要名称
syntax-trait-expect-body = trait 声明需要模块体
syntax-impl-boundary-unknown = 无法确定 impl 块的边界
syntax-impl-expect-target = impl 后需要类型目标
syntax-function-expect-name = fn 后需要函数名称
syntax-function-missing-parameters = 函数签名缺少参数列表
syntax-function-parameters-unclosed = 函数参数列表缺少结束括号
syntax-function-body-unclosed = 函数体缺少结束花括号
syntax-item-unsupported = 当前 item 语法尚未纳入 AST 覆盖范围
syntax-delimiter-unclosed = 开始分隔符缺少匹配的结束符
syntax-unmatched-close-delimiter = 出现未匹配或次序错误的结束分隔符
syntax-declaration-body-unclosed = 声明体缺少结束花括号
syntax-orphaned-outer-attribute = 外层属性没有后续 item
config-file-missing = 显式配置文件 { $path } 不存在或不是普通文件
config-file-read-error = 读取配置文件 { $path } 失败：{ $detail }
config-parse-error = 解析配置文件 { $path } 失败：{ $detail }
config-invalid-root = 配置文件 { $path } 的根必须是 TOML 表
config-version-type = 配置文件 { $path } 的字段 version 必须是整数
config-unsupported-version = 配置文件 { $path } 使用了不支持的版本 { $version }，当前支持版本 { $supported }
config-invalid-field = 配置文件 { $path } 的字段 { $field}：{ $reason ->
    [unknown-field] 未知字段
    [must-be-table] 必须是表
    [must-be-string] 必须是字符串
    [must-be-nonempty-string] 必须是非空字符串
    [must-be-string-array] 必须是字符串数组
    [must-be-boolean] 必须是布尔值
    [unknown-rule-id] 未知规则 ID
    [unknown-severity] 未知级别 { $value }
    [unknown-value] 未知值 { $value }
    [duplicate] 重复值 { $value }
   *[invalid] 值无效
}
locale-invalid-cli = 无效的 --locale 值：{ $locale }
locale-invalid-config = 无效的 [i18n].locale 配置值：{ $locale }
cargo-error = Cargo 操作失败：{ $detail }
cargo-operation-error = 处理 Cargo 配置 { $path } 失败：{ $detail }
cargo-workspace-conflict = --workspace 不能与 --package 同时使用
cargo-target-cfg-missing = 缺少 target { $target } 的 cfg 信息
cargo-feature-context-missing = 缺少 package { $package } 的 feature 上下文
cargo-no-scan-packages = manifest 未匹配到可扫描的 workspace package
cargo-package-not-found = workspace 中不存在 package { $name }
cargo-package-ambiguous = package 名称 { $name } 不唯一，请拆分 workspace 或明确 package 名
cargo-package-duplicate = --package 重复指定 { $name }
cargo-feature-package-not-selected = --features 指定了未选中的 package { $package }
cargo-feature-not-found = package { $package } 没有 feature { $feature }
cargo-feature-not-declared = 选中的 package 均未声明 feature { $feature }
cargo-config-target-type = Cargo 配置 { $path } 的 build.target 必须是字符串
cargo-target-cfg-invalid = 无法解析 Cargo target 条件 { $condition }
cargo-target-cfg-query-error = 查询 target { $target } 的 cfg 值失败：{ $detail }
cargo-manifest-unavailable = 无法访问 manifest { $path }：{ $detail }
cargo-host-triple-missing = rustc -vV 输出缺少 host triple
argument-error = 命令行参数无效：{ $detail }
argument-unknown-rule = 未知规则 ID：{ $rule }
argument-duplicate-rule = 规则 { $rule } 被重复指定
argument-severity-format = --severity 格式错误：{ $value }，应为 ID=warning|error
argument-unknown-severity = 未知级别：{ $severity }
argument-severity-duplicate = 规则 { $rule } 的 --severity 被重复指定
io-error = 读取当前目录失败：{ $detail }
cli-about = 检查 Rust 源码是否符合仓库规范
cli-help = 用法：cargo-cop [选项]
    检查 Rust 源码是否符合仓库规范。

    选项：
      --manifest-path <PATH>    指定 Cargo.toml 路径
      --workspace               扫描整个 workspace
      -p, --package <NAME>      选择要扫描的 package（可重复指定）
      --target <TRIPLE>         指定目标三元组
      --features <LIST>         启用逗号分隔的 features
      --all-features            启用所有 features
      --no-default-features     禁用默认 features
      --all-targets             扫描所有 targets
      --config <PATH>           指定配置文件
      --locale <TAG>            选择输出语言（zh-CN、en-US）
      --format <FORMAT>         输出格式：human 或 json
      --disable-rule <ID>       禁用规则（可重复指定）
      --severity <ID=LEVEL>     设置规则级别（可重复指定）
      -h, --help                显示帮助
      -V, --version             显示版本
cli-parse-error = { $kind ->
    [unknown] 命令行选项或子命令未知。使用 --help 查看可用选项。
    [missing] 缺少必需的命令行选项或值。使用 --help 查看用法。
    [invalid] 命令行选项的值或格式无效。使用 --help 查看有效格式。
    [conflict] 提供的命令行选项不能同时使用。
   *[other] 命令行参数无效。使用 --help 查看用法。
}
cli-version = cargo-cop { $version }
output-write-error = 写入诊断失败：{ $detail }
report-summary = cargo-cop：状态 { $status ->
    [complete] 已完成
    [partial] 部分完成
   *[incomplete] 未完成
}；错误 { $errors } 项，警告 { $warnings } 项，已抑制警告 { $suppressed_warnings } 项，共 { $diagnostics } 项
report-detail-reference = 详见 detail 字段
report-namespace-label = 命名空间
report-cfg-label = 配置条件
