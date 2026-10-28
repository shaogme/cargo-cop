item-spacing = Adjacent definitions require { $expected } blank lines; found { $actual ->
    [one] { $actual } blank line
   *[other] { $actual } blank lines
}
module-file-layout = Submodule entry files must not use the mod.rs layout
out-of-line-modules-at-scope-start = Out-of-line module declarations must follow extern crate declarations and stay before use declarations and other items
child-module-visibility = Non-private submodule { $module } may contain only declarations and use items with private, public, or pub(crate) visibility
child-module-line-count = The effective line count of submodule file { $file } is { $actual }; it must be between { $minimum } and { $maximum }
no-parent-path-attribute = A #[path] attribute must not traverse to a parent directory
no-parent-glob-import = Child modules must not use super::* to import parent implementation details
no-parent-private-path = The child module path depends on private item { $item } from ancestor module { $ancestor }
impl-cross-file = The impl target { $target } is defined in another source file; impl blocks must be in the same file as the type definition
impl-external-trait-target = The trait impl target root { $root } is not an allowed standard library type
impl-inherent-target = An inherent impl target must be a type defined in the current source file
impl-target-unresolved = Could not resolve the type ownership of impl target { $target }
private-fields-for-behavior-type = All fields of type { $name } with an impl block must be private
unnecessary-pub-crate = The crate-visible declaration { $name } can be private
child-public-free-function = Free functions in child modules must be private
no-lint-relaxation = Do not relax lint checks with an allow attribute
function-parameter-limit = This function has { $actual } parameters; the maximum is { $maximum }
closure-parameter-limit = This closure has { $actual } parameters; the maximum is { $maximum }
tuple-arity = This tuple has { $actual } elements; the maximum is { $maximum }
declaration-before-use = Name { $name } is referenced before its declaration
no-inline-qualified-path = Explicitly qualified path { $path } should use an imported name instead
extern-crate-at-scope-start = extern crate declarations must appear at the start of a module scope
use-at-scope-start = Module-scope use declarations must follow out-of-line modules and precede other items
block-use-at-scope-start = A use declaration in a block must appear before the first non-use statement
merge-import-prefix = This import shares the prefix { $prefix } with the import on line { $line }; merge them into one use tree
merge-block-import-prefix = This block import shares the prefix { $prefix } with the import on line { $line }; merge them into one use tree
cfg-use-group-split = The cfg import group { $condition } is split by another import group
cfg-use-group-spacing = Different cfg import groups must have exactly one blank line between them; found { $empty_lines ->
    [one] { $empty_lines } blank line
   *[other] { $empty_lines } blank lines
}
cfg-inside-use-tree = Do not nest cfg attributes inside braces of a use tree
cfg-dependency-unknown = Could not statically determine the target condition for dependency { $dependency }
include-dynamic-input = include! brings source code in at compile time, so the static module graph cannot determine its contents
macro-generated-module = The item macro { $name } was not expanded and may generate a module declaration
module-cycle = Module { $name } loads a source file that is already active in the current recursion chain
path-module-missing = The module file specified by #[path] does not exist
module-layout-ambiguous = Module { $name } matches both the name.rs and name/mod.rs layouts
module-missing = Could not find module { $name }; checked the name.rs and name/mod.rs layouts
module-source-read-error = Could not read Rust source file { $path}: { $detail }
module-file-metadata-error = Could not inspect module file { $path}: { $detail }
parser-coverage = { $issue }
malformed-cfg-attribute = Could not reliably interpret a cfg/path attribute; module analysis may be incomplete
unknown-cfg = Could not statically determine cfg condition { $condition }
syntax-unclosed-block-comment = Block comment is missing its closing */
syntax-unknown-character = Unrecognized character { $character }
syntax-unclosed-raw-string = Raw string is missing its closing delimiter
syntax-unclosed-literal = Literal is missing its closing quote
syntax-expect-literal-or-lifetime = A character literal or lifetime name is required after a single quote
syntax-invalid-byte-character = Invalid byte character literal
syntax-expect-radix-digit = At least one digit is required after the radix prefix
syntax-invalid-radix-digit = Numeric literal contains a digit that is invalid in base { $radix }
syntax-inner-attribute-position = Inner attributes must be at the beginning of a scope
syntax-expect-attribute-bracket = An opening bracket is required after the attribute marker #
syntax-attribute-invalid = Attribute path or metadata structure is invalid
syntax-attribute-unclosed = Attribute is missing its closing bracket
syntax-visibility-unclosed = Visibility restriction is missing its closing parenthesis
syntax-expect-module-name = A module name is required after mod
syntax-expect-module-body = A module declaration must end with a semicolon or have a module body
syntax-inline-module-unclosed = Inline module is missing its closing brace
syntax-use-missing-semicolon = use declaration is missing its semicolon
syntax-use-tree-invalid = Use tree path, group, alias, or glob structure is invalid
syntax-extern-crate-missing-semicolon = extern crate declaration must end with a semicolon
syntax-named-item-expect-name = A name is required after the named item keyword
syntax-trait-expect-name = A name is required after trait
syntax-trait-expect-body = Trait declaration must have a body
syntax-impl-boundary-unknown = Could not determine the boundary of the impl block
syntax-impl-expect-target = A type target is required after impl
syntax-function-expect-name = A function name is required after fn
syntax-function-missing-parameters = Function signature is missing its parameter list
syntax-function-parameters-unclosed = Function parameter list is missing its closing parenthesis
syntax-function-body-unclosed = Function body is missing its closing brace
syntax-item-unsupported = This item syntax is not yet covered by the AST
syntax-expression-unsupported = This expression syntax is outside the supported stable syntax set
syntax-unsupported-edition = Rust edition {$edition} is not supported
syntax-macro-opaque = Macro input is opaque and its internal Rust syntax is not analyzed
syntax-parse-error-lexical-error = Lexing encountered an invalid or unrecognized token
syntax-parse-error-unexpected-token = Unexpected token in this syntax position
syntax-parse-error-unexpected-eof = The syntax construct was incomplete at end of input
syntax-parse-error-unclosed-delimiter = An opening delimiter has no matching closing delimiter
syntax-parse-error-invalid-attribute = The attribute structure is invalid
syntax-parse-error-unsupported-edition = Rust edition {$edition} is not supported
syntax-unsupported-item = This item syntax is not modeled
syntax-unsupported-associated-item = This associated item syntax is not modeled
syntax-unsupported-type = This type syntax is not modeled
syntax-unsupported-pattern = This pattern syntax is not modeled
syntax-unsupported-statement = This statement syntax is not modeled
syntax-unsupported-expression = This expression syntax is not modeled
syntax-unsupported-attribute = This attribute syntax is not modeled
syntax-unsupported-visibility = This visibility syntax is not modeled
syntax-unsupported-use-tree = This use tree syntax is not modeled
syntax-unsupported-token = This token structure is not modeled
syntax-delimiter-unclosed = Opening delimiter is missing its matching closing delimiter
syntax-unexpected-token = Unexpected token in this syntax position
syntax-unmatched-close-delimiter = Closing delimiter has no matching opener or is out of order
syntax-declaration-body-unclosed = Declaration body is missing its closing brace
syntax-orphaned-outer-attribute = Outer attribute is not followed by an item
config-file-missing = Explicit configuration file { $path } does not exist or is not a regular file
config-file-read-error = Could not read configuration file { $path}: { $detail }
config-parse-error = Could not parse configuration file { $path}: { $detail }
config-invalid-root = The root of configuration file { $path } must be a TOML table
config-version-type = Field version in configuration file { $path } must be an integer
config-unsupported-version = Configuration file { $path } uses unsupported version { $version}; the supported version is { $supported }
config-invalid-field = Field { $field} in configuration file { $path}: { $reason ->
    [unknown-field] unknown field
    [must-be-table] must be a table
    [must-be-string] must be a string
    [must-be-nonempty-string] must be a non-empty string
    [must-be-string-array] must be an array of strings
    [must-be-boolean] must be a boolean
    [unknown-rule-id] unknown rule ID
    [unknown-severity] unknown severity { $value }
    [unknown-value] unknown value { $value }
    [duplicate] duplicate value { $value }
   *[invalid] invalid value
}
locale-invalid-cli = Invalid --locale value: { $locale }
locale-invalid-config = Invalid [i18n].locale configuration value: { $locale }
cargo-error = Cargo operation failed: { $detail }
cargo-operation-error = Could not process Cargo configuration at { $path}: { $detail }
cargo-workspace-conflict = --workspace cannot be used together with --package
cargo-target-cfg-missing = cfg information is missing for target { $target }
cargo-feature-context-missing = feature context is missing for package { $package }
cargo-no-scan-packages = The manifest did not match a scannable workspace package
cargo-package-not-found = Package { $name } does not exist in the workspace
cargo-package-ambiguous = Package name { $name } is ambiguous; split the workspace or specify a unique package name
cargo-package-duplicate = --package specifies { $name } more than once
cargo-feature-package-not-selected = --features references package { $package }, which is not selected
cargo-feature-not-found = Package { $package } does not have feature { $feature }
cargo-feature-not-declared = None of the selected packages declares feature { $feature }
cargo-config-target-type = Cargo configuration { $path } must set build.target to a string
cargo-target-cfg-invalid = Could not parse Cargo target condition { $condition }
cargo-target-cfg-query-error = Could not query cfg values for target { $target}: { $detail }
cargo-manifest-unavailable = Could not access manifest { $path}: { $detail }
cargo-host-triple-missing = rustc -vV output is missing the host triple
argument-error = Invalid command-line arguments: { $detail }
argument-unknown-rule = Unknown rule ID: { $rule }
argument-duplicate-rule = Rule { $rule } was specified more than once
argument-severity-format = Invalid --severity value { $value }; expected ID=warning|error
argument-unknown-severity = Unknown severity: { $severity }
argument-severity-duplicate = --severity was specified more than once for rule { $rule }
io-error = Could not read current directory: { $detail }
cli-about = Check whether Rust source code follows repository conventions
cli-help = Usage: cargo-cop [OPTIONS]
    Check whether Rust source code follows repository conventions.

    Options:
      --manifest-path <PATH>    Path to Cargo.toml
      --workspace               Scan the entire workspace
      -p, --package <NAME>      Select packages to scan; may be repeated
      --target <TRIPLE>         Target triple to scan
      --features <LIST>         Enable comma-separated features
      --all-features            Enable all features
      --no-default-features     Disable default features
      --all-targets             Scan all targets
      --config <PATH>           Configuration file path
      --locale <TAG>            Output locale (zh-CN, en-US)
      --format <FORMAT>         Output format: human or json
      --disable-rule <ID>       Disable a rule; may be repeated
      --severity <ID=LEVEL>     Set rule severity; may be repeated
      -h, --help                Print help
      -V, --version             Print version
cli-parse-error = { $kind ->
    [unknown] Unknown command-line option or subcommand. Use --help to see available options.
    [missing] A required command-line option or value is missing. Use --help to see usage.
    [invalid] A command-line option has an invalid value or format. Use --help to see valid forms.
    [conflict] The supplied command-line options cannot be used together.
   *[other] Invalid command-line arguments. Use --help to see usage.
}
cli-version = cargo-cop { $version }
output-write-error = Could not write diagnostics: { $detail }
report-summary = cargo-cop: status { $status ->
    [complete] complete
    [partial] partial
   *[incomplete] incomplete
}; { $errors ->
    [one] { $errors } error
   *[other] { $errors } errors
}, { $warnings ->
    [one] { $warnings } warning
   *[other] { $warnings } warnings
}, { $suppressed_warnings ->
    [one] { $suppressed_warnings } warning suppressed
   *[other] { $suppressed_warnings } warnings suppressed
}, { $diagnostics ->
    [one] { $diagnostics } diagnostic
   *[other] { $diagnostics } diagnostics
}
report-detail-reference = see the detail field
report-namespace-label = namespace
report-cfg-label = cfg
