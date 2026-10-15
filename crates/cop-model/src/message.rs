use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MessageId(&'static str);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageArgumentKind {
    Text,
    Number,
    Boolean,
    Opaque,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged)]
pub enum MessageArgument {
    Text(String),
    Number(i64),
    Boolean(bool),
    Opaque(String),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LocalizedMessage {
    id: MessageId,
    arguments: BTreeMap<String, MessageArgument>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageSchemaError {
    id: MessageId,
    argument: String,
}

const TEXT: MessageArgumentKind = MessageArgumentKind::Text;
const NUMBER: MessageArgumentKind = MessageArgumentKind::Number;
const OPAQUE: MessageArgumentKind = MessageArgumentKind::Opaque;
pub const ITEM_SPACING: MessageId = MessageId::new("item-spacing");
pub const MODULE_FILE_LAYOUT: MessageId = MessageId::new("module-file-layout");
pub const OUT_OF_LINE_MODULES_AT_SCOPE_START: MessageId =
    MessageId::new("out-of-line-modules-at-scope-start");
pub const PRIVATE_CHILD_MODULE: MessageId = MessageId::new("private-child-module");
pub const CHILD_MODULE_LINE_COUNT: MessageId = MessageId::new("child-module-line-count");
pub const NO_PARENT_GLOB_IMPORT: MessageId = MessageId::new("no-parent-glob-import");
pub const NO_PARENT_PRIVATE_PATH: MessageId = MessageId::new("no-parent-private-path");
pub const IMPL_CROSS_FILE: MessageId = MessageId::new("impl-cross-file");
pub const IMPL_EXTERNAL_TRAIT_TARGET: MessageId = MessageId::new("impl-external-trait-target");
pub const IMPL_INHERENT_TARGET: MessageId = MessageId::new("impl-inherent-target");
pub const IMPL_TARGET_UNRESOLVED: MessageId = MessageId::new("impl-target-unresolved");
pub const PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE: MessageId =
    MessageId::new("private-fields-for-behavior-type");
pub const CHILD_PUBLIC_FREE_FUNCTION: MessageId = MessageId::new("child-public-free-function");
pub const NO_LINT_RELAXATION: MessageId = MessageId::new("no-lint-relaxation");
pub const FUNCTION_PARAMETER_LIMIT: MessageId = MessageId::new("function-parameter-limit");
pub const CLOSURE_PARAMETER_LIMIT: MessageId = MessageId::new("closure-parameter-limit");
pub const TUPLE_ARITY: MessageId = MessageId::new("tuple-arity");
pub const DECLARATION_BEFORE_USE: MessageId = MessageId::new("declaration-before-use");
pub const NO_INLINE_QUALIFIED_PATH: MessageId = MessageId::new("no-inline-qualified-path");
pub const USE_AT_SCOPE_START: MessageId = MessageId::new("use-at-scope-start");
pub const BLOCK_USE_AT_SCOPE_START: MessageId = MessageId::new("block-use-at-scope-start");
pub const MERGE_IMPORT_PREFIX: MessageId = MessageId::new("merge-import-prefix");
pub const MERGE_BLOCK_IMPORT_PREFIX: MessageId = MessageId::new("merge-block-import-prefix");
pub const CFG_USE_GROUP_SPLIT: MessageId = MessageId::new("cfg-use-group-split");
pub const CFG_USE_GROUP_SPACING: MessageId = MessageId::new("cfg-use-group-spacing");
pub const CFG_INSIDE_USE_TREE: MessageId = MessageId::new("cfg-inside-use-tree");
pub const CFG_DEPENDENCY_UNKNOWN: MessageId = MessageId::new("cfg-dependency-unknown");
pub const INCLUDE_DYNAMIC_INPUT: MessageId = MessageId::new("include-dynamic-input");
pub const MACRO_GENERATED_MODULE: MessageId = MessageId::new("macro-generated-module");
pub const MODULE_CYCLE: MessageId = MessageId::new("module-cycle");
pub const PATH_MODULE_MISSING: MessageId = MessageId::new("path-module-missing");
pub const MODULE_LAYOUT_AMBIGUOUS: MessageId = MessageId::new("module-layout-ambiguous");
pub const MODULE_MISSING: MessageId = MessageId::new("module-missing");
pub const MODULE_SOURCE_READ_ERROR: MessageId = MessageId::new("module-source-read-error");
pub const MODULE_FILE_METADATA_ERROR: MessageId = MessageId::new("module-file-metadata-error");
pub const PARSER_COVERAGE: MessageId = MessageId::new("parser-coverage");
pub const MALFORMED_CFG_ATTRIBUTE: MessageId = MessageId::new("malformed-cfg-attribute");
pub const UNKNOWN_CFG: MessageId = MessageId::new("unknown-cfg");
pub const SYNTAX_UNCLOSED_BLOCK_COMMENT: MessageId =
    MessageId::new("syntax-unclosed-block-comment");
pub const SYNTAX_UNKNOWN_CHARACTER: MessageId = MessageId::new("syntax-unknown-character");
pub const SYNTAX_UNCLOSED_RAW_STRING: MessageId = MessageId::new("syntax-unclosed-raw-string");
pub const SYNTAX_UNCLOSED_LITERAL: MessageId = MessageId::new("syntax-unclosed-literal");
pub const SYNTAX_EXPECT_LITERAL_OR_LIFETIME: MessageId =
    MessageId::new("syntax-expect-literal-or-lifetime");
pub const SYNTAX_INVALID_BYTE_CHARACTER: MessageId =
    MessageId::new("syntax-invalid-byte-character");
pub const SYNTAX_EXPECT_RADIX_DIGIT: MessageId = MessageId::new("syntax-expect-radix-digit");
pub const SYNTAX_INVALID_RADIX_DIGIT: MessageId = MessageId::new("syntax-invalid-radix-digit");
pub const SYNTAX_INNER_ATTRIBUTE_POSITION: MessageId =
    MessageId::new("syntax-inner-attribute-position");
pub const SYNTAX_EXPECT_ATTRIBUTE_BRACKET: MessageId =
    MessageId::new("syntax-expect-attribute-bracket");
pub const SYNTAX_ATTRIBUTE_UNCLOSED: MessageId = MessageId::new("syntax-attribute-unclosed");
pub const SYNTAX_VISIBILITY_UNCLOSED: MessageId = MessageId::new("syntax-visibility-unclosed");
pub const SYNTAX_EXPECT_MODULE_NAME: MessageId = MessageId::new("syntax-expect-module-name");
pub const SYNTAX_EXPECT_MODULE_BODY: MessageId = MessageId::new("syntax-expect-module-body");
pub const SYNTAX_INLINE_MODULE_UNCLOSED: MessageId =
    MessageId::new("syntax-inline-module-unclosed");
pub const SYNTAX_USE_MISSING_SEMICOLON: MessageId = MessageId::new("syntax-use-missing-semicolon");
pub const SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON: MessageId =
    MessageId::new("syntax-extern-crate-missing-semicolon");
pub const SYNTAX_NAMED_ITEM_EXPECT_NAME: MessageId =
    MessageId::new("syntax-named-item-expect-name");
pub const SYNTAX_TRAIT_EXPECT_NAME: MessageId = MessageId::new("syntax-trait-expect-name");
pub const SYNTAX_TRAIT_EXPECT_BODY: MessageId = MessageId::new("syntax-trait-expect-body");
pub const SYNTAX_IMPL_BOUNDARY_UNKNOWN: MessageId = MessageId::new("syntax-impl-boundary-unknown");
pub const SYNTAX_IMPL_EXPECT_TARGET: MessageId = MessageId::new("syntax-impl-expect-target");
pub const SYNTAX_FUNCTION_EXPECT_NAME: MessageId = MessageId::new("syntax-function-expect-name");
pub const SYNTAX_FUNCTION_MISSING_PARAMETERS: MessageId =
    MessageId::new("syntax-function-missing-parameters");
pub const SYNTAX_FUNCTION_PARAMETERS_UNCLOSED: MessageId =
    MessageId::new("syntax-function-parameters-unclosed");
pub const SYNTAX_FUNCTION_BODY_UNCLOSED: MessageId =
    MessageId::new("syntax-function-body-unclosed");
pub const SYNTAX_ITEM_UNSUPPORTED: MessageId = MessageId::new("syntax-item-unsupported");
pub const SYNTAX_DELIMITER_UNCLOSED: MessageId = MessageId::new("syntax-delimiter-unclosed");
pub const SYNTAX_UNMATCHED_CLOSE_DELIMITER: MessageId =
    MessageId::new("syntax-unmatched-close-delimiter");
pub const SYNTAX_DECLARATION_BODY_UNCLOSED: MessageId =
    MessageId::new("syntax-declaration-body-unclosed");
pub const SYNTAX_ORPHANED_OUTER_ATTRIBUTE: MessageId =
    MessageId::new("syntax-orphaned-outer-attribute");
pub const CONFIG_FILE_MISSING: MessageId = MessageId::new("config-file-missing");
pub const CONFIG_FILE_READ_ERROR: MessageId = MessageId::new("config-file-read-error");
pub const CONFIG_PARSE_ERROR: MessageId = MessageId::new("config-parse-error");
pub const CONFIG_INVALID_ROOT: MessageId = MessageId::new("config-invalid-root");
pub const CONFIG_VERSION_TYPE: MessageId = MessageId::new("config-version-type");
pub const CONFIG_UNSUPPORTED_VERSION: MessageId = MessageId::new("config-unsupported-version");
pub const CONFIG_INVALID_FIELD: MessageId = MessageId::new("config-invalid-field");
pub const LOCALE_INVALID_CLI: MessageId = MessageId::new("locale-invalid-cli");
pub const LOCALE_INVALID_CONFIG: MessageId = MessageId::new("locale-invalid-config");
pub const CARGO_ERROR: MessageId = MessageId::new("cargo-error");
pub const CARGO_OPERATION_ERROR: MessageId = MessageId::new("cargo-operation-error");
pub const CARGO_WORKSPACE_CONFLICT: MessageId = MessageId::new("cargo-workspace-conflict");
pub const CARGO_TARGET_CFG_MISSING: MessageId = MessageId::new("cargo-target-cfg-missing");
pub const CARGO_FEATURE_CONTEXT_MISSING: MessageId =
    MessageId::new("cargo-feature-context-missing");
pub const CARGO_NO_SCAN_PACKAGES: MessageId = MessageId::new("cargo-no-scan-packages");
pub const CARGO_PACKAGE_NOT_FOUND: MessageId = MessageId::new("cargo-package-not-found");
pub const CARGO_PACKAGE_AMBIGUOUS: MessageId = MessageId::new("cargo-package-ambiguous");
pub const CARGO_PACKAGE_DUPLICATE: MessageId = MessageId::new("cargo-package-duplicate");
pub const CARGO_FEATURE_PACKAGE_NOT_SELECTED: MessageId =
    MessageId::new("cargo-feature-package-not-selected");
pub const CARGO_FEATURE_NOT_FOUND: MessageId = MessageId::new("cargo-feature-not-found");
pub const CARGO_FEATURE_NOT_DECLARED: MessageId = MessageId::new("cargo-feature-not-declared");
pub const CARGO_CONFIG_TARGET_TYPE: MessageId = MessageId::new("cargo-config-target-type");
pub const CARGO_TARGET_CFG_INVALID: MessageId = MessageId::new("cargo-target-cfg-invalid");
pub const CARGO_TARGET_CFG_QUERY_ERROR: MessageId = MessageId::new("cargo-target-cfg-query-error");
pub const CARGO_MANIFEST_UNAVAILABLE: MessageId = MessageId::new("cargo-manifest-unavailable");
pub const CARGO_HOST_TRIPLE_MISSING: MessageId = MessageId::new("cargo-host-triple-missing");
pub const ARGUMENT_ERROR: MessageId = MessageId::new("argument-error");
pub const ARGUMENT_UNKNOWN_RULE: MessageId = MessageId::new("argument-unknown-rule");
pub const ARGUMENT_DUPLICATE_RULE: MessageId = MessageId::new("argument-duplicate-rule");
pub const ARGUMENT_SEVERITY_FORMAT: MessageId = MessageId::new("argument-severity-format");
pub const ARGUMENT_UNKNOWN_SEVERITY: MessageId = MessageId::new("argument-unknown-severity");
pub const ARGUMENT_SEVERITY_DUPLICATE: MessageId = MessageId::new("argument-severity-duplicate");
pub const IO_ERROR: MessageId = MessageId::new("io-error");
pub const CLI_ABOUT: MessageId = MessageId::new("cli-about");
pub const CLI_HELP: MessageId = MessageId::new("cli-help");
pub const CLI_PARSE_ERROR: MessageId = MessageId::new("cli-parse-error");
pub const CLI_VERSION: MessageId = MessageId::new("cli-version");
pub const OUTPUT_WRITE_ERROR: MessageId = MessageId::new("output-write-error");
pub const REPORT_SUMMARY: MessageId = MessageId::new("report-summary");
pub const REPORT_DETAIL_REFERENCE: MessageId = MessageId::new("report-detail-reference");
pub const REPORT_NAMESPACE_LABEL: MessageId = MessageId::new("report-namespace-label");
pub const REPORT_CFG_LABEL: MessageId = MessageId::new("report-cfg-label");

impl MessageId {
    pub const ALL: &'static [Self] = &[
        ITEM_SPACING,
        MODULE_FILE_LAYOUT,
        OUT_OF_LINE_MODULES_AT_SCOPE_START,
        PRIVATE_CHILD_MODULE,
        CHILD_MODULE_LINE_COUNT,
        NO_PARENT_GLOB_IMPORT,
        NO_PARENT_PRIVATE_PATH,
        IMPL_CROSS_FILE,
        IMPL_EXTERNAL_TRAIT_TARGET,
        IMPL_INHERENT_TARGET,
        IMPL_TARGET_UNRESOLVED,
        PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE,
        CHILD_PUBLIC_FREE_FUNCTION,
        NO_LINT_RELAXATION,
        FUNCTION_PARAMETER_LIMIT,
        CLOSURE_PARAMETER_LIMIT,
        TUPLE_ARITY,
        DECLARATION_BEFORE_USE,
        NO_INLINE_QUALIFIED_PATH,
        USE_AT_SCOPE_START,
        BLOCK_USE_AT_SCOPE_START,
        MERGE_IMPORT_PREFIX,
        MERGE_BLOCK_IMPORT_PREFIX,
        CFG_USE_GROUP_SPLIT,
        CFG_USE_GROUP_SPACING,
        CFG_INSIDE_USE_TREE,
        CFG_DEPENDENCY_UNKNOWN,
        INCLUDE_DYNAMIC_INPUT,
        MACRO_GENERATED_MODULE,
        MODULE_CYCLE,
        PATH_MODULE_MISSING,
        MODULE_LAYOUT_AMBIGUOUS,
        MODULE_MISSING,
        MODULE_SOURCE_READ_ERROR,
        MODULE_FILE_METADATA_ERROR,
        PARSER_COVERAGE,
        MALFORMED_CFG_ATTRIBUTE,
        UNKNOWN_CFG,
        SYNTAX_UNCLOSED_BLOCK_COMMENT,
        SYNTAX_UNKNOWN_CHARACTER,
        SYNTAX_UNCLOSED_RAW_STRING,
        SYNTAX_UNCLOSED_LITERAL,
        SYNTAX_EXPECT_LITERAL_OR_LIFETIME,
        SYNTAX_INVALID_BYTE_CHARACTER,
        SYNTAX_EXPECT_RADIX_DIGIT,
        SYNTAX_INVALID_RADIX_DIGIT,
        SYNTAX_INNER_ATTRIBUTE_POSITION,
        SYNTAX_EXPECT_ATTRIBUTE_BRACKET,
        SYNTAX_ATTRIBUTE_UNCLOSED,
        SYNTAX_VISIBILITY_UNCLOSED,
        SYNTAX_EXPECT_MODULE_NAME,
        SYNTAX_EXPECT_MODULE_BODY,
        SYNTAX_INLINE_MODULE_UNCLOSED,
        SYNTAX_USE_MISSING_SEMICOLON,
        SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON,
        SYNTAX_NAMED_ITEM_EXPECT_NAME,
        SYNTAX_TRAIT_EXPECT_NAME,
        SYNTAX_TRAIT_EXPECT_BODY,
        SYNTAX_IMPL_BOUNDARY_UNKNOWN,
        SYNTAX_IMPL_EXPECT_TARGET,
        SYNTAX_FUNCTION_EXPECT_NAME,
        SYNTAX_FUNCTION_MISSING_PARAMETERS,
        SYNTAX_FUNCTION_PARAMETERS_UNCLOSED,
        SYNTAX_FUNCTION_BODY_UNCLOSED,
        SYNTAX_ITEM_UNSUPPORTED,
        SYNTAX_DELIMITER_UNCLOSED,
        SYNTAX_UNMATCHED_CLOSE_DELIMITER,
        SYNTAX_DECLARATION_BODY_UNCLOSED,
        CONFIG_FILE_MISSING,
        SYNTAX_ORPHANED_OUTER_ATTRIBUTE,
        CONFIG_FILE_READ_ERROR,
        CONFIG_PARSE_ERROR,
        CONFIG_INVALID_ROOT,
        CONFIG_VERSION_TYPE,
        CONFIG_UNSUPPORTED_VERSION,
        CONFIG_INVALID_FIELD,
        LOCALE_INVALID_CLI,
        LOCALE_INVALID_CONFIG,
        CARGO_ERROR,
        CARGO_OPERATION_ERROR,
        ARGUMENT_ERROR,
        CARGO_WORKSPACE_CONFLICT,
        CARGO_TARGET_CFG_MISSING,
        CARGO_FEATURE_CONTEXT_MISSING,
        CARGO_NO_SCAN_PACKAGES,
        CARGO_PACKAGE_NOT_FOUND,
        CARGO_PACKAGE_AMBIGUOUS,
        CARGO_PACKAGE_DUPLICATE,
        CARGO_FEATURE_PACKAGE_NOT_SELECTED,
        CARGO_FEATURE_NOT_FOUND,
        CARGO_FEATURE_NOT_DECLARED,
        CARGO_CONFIG_TARGET_TYPE,
        CARGO_TARGET_CFG_INVALID,
        CARGO_TARGET_CFG_QUERY_ERROR,
        CARGO_MANIFEST_UNAVAILABLE,
        CARGO_HOST_TRIPLE_MISSING,
        ARGUMENT_UNKNOWN_RULE,
        ARGUMENT_DUPLICATE_RULE,
        ARGUMENT_SEVERITY_FORMAT,
        ARGUMENT_UNKNOWN_SEVERITY,
        ARGUMENT_SEVERITY_DUPLICATE,
        IO_ERROR,
        CLI_ABOUT,
        CLI_HELP,
        CLI_PARSE_ERROR,
        CLI_VERSION,
        OUTPUT_WRITE_ERROR,
        REPORT_SUMMARY,
        REPORT_DETAIL_REFERENCE,
        REPORT_NAMESPACE_LABEL,
        REPORT_CFG_LABEL,
    ];

    const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }

    pub fn argument_schema(self) -> &'static [(&'static str, MessageArgumentKind)] {
        match self {
            ITEM_SPACING => &[("expected", NUMBER), ("actual", NUMBER)],
            CFG_USE_GROUP_SPACING => &[("empty_lines", NUMBER)],
            PRIVATE_CHILD_MODULE => &[("module", TEXT)],
            CHILD_MODULE_LINE_COUNT => &[
                ("file", TEXT),
                ("actual", NUMBER),
                ("minimum", NUMBER),
                ("maximum", NUMBER),
            ],
            NO_PARENT_PRIVATE_PATH => &[("ancestor", TEXT), ("item", TEXT)],
            IMPL_CROSS_FILE => &[("target", TEXT)],
            IMPL_EXTERNAL_TRAIT_TARGET => &[("root", TEXT)],
            IMPL_TARGET_UNRESOLVED => &[("target", TEXT)],
            PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE => &[("name", TEXT)],
            FUNCTION_PARAMETER_LIMIT | CLOSURE_PARAMETER_LIMIT | TUPLE_ARITY => {
                &[("actual", NUMBER), ("maximum", NUMBER)]
            }
            DECLARATION_BEFORE_USE => &[("name", TEXT)],
            NO_INLINE_QUALIFIED_PATH => &[("path", TEXT)],
            MERGE_IMPORT_PREFIX | MERGE_BLOCK_IMPORT_PREFIX => {
                &[("line", NUMBER), ("prefix", TEXT)]
            }
            CFG_USE_GROUP_SPLIT | UNKNOWN_CFG => &[("condition", TEXT)],
            CFG_DEPENDENCY_UNKNOWN => &[("dependency", TEXT)],
            MACRO_GENERATED_MODULE | MODULE_CYCLE => &[("name", TEXT)],
            MODULE_LAYOUT_AMBIGUOUS | MODULE_MISSING => &[("name", TEXT)],
            MODULE_SOURCE_READ_ERROR | MODULE_FILE_METADATA_ERROR => {
                &[("path", TEXT), ("detail", OPAQUE)]
            }
            PARSER_COVERAGE => &[("issue", TEXT)],
            SYNTAX_UNKNOWN_CHARACTER => &[("character", TEXT)],
            SYNTAX_INVALID_RADIX_DIGIT => &[("radix", NUMBER)],
            CONFIG_FILE_MISSING => &[("path", TEXT)],
            CONFIG_FILE_READ_ERROR | CONFIG_PARSE_ERROR => &[("path", TEXT), ("detail", OPAQUE)],
            CONFIG_INVALID_ROOT | CONFIG_VERSION_TYPE => &[("path", TEXT)],
            CONFIG_UNSUPPORTED_VERSION => {
                &[("path", TEXT), ("version", NUMBER), ("supported", NUMBER)]
            }
            CONFIG_INVALID_FIELD => &[
                ("path", TEXT),
                ("field", TEXT),
                ("reason", TEXT),
                ("value", TEXT),
            ],
            LOCALE_INVALID_CLI | LOCALE_INVALID_CONFIG => &[("locale", TEXT)],
            ARGUMENT_UNKNOWN_RULE | ARGUMENT_DUPLICATE_RULE | ARGUMENT_SEVERITY_DUPLICATE => {
                &[("rule", TEXT)]
            }
            ARGUMENT_SEVERITY_FORMAT => &[("value", TEXT)],
            ARGUMENT_UNKNOWN_SEVERITY => &[("severity", TEXT)],
            CLI_VERSION => &[("version", TEXT)],
            CARGO_TARGET_CFG_MISSING => &[("target", TEXT)],
            CARGO_FEATURE_CONTEXT_MISSING => &[("package", TEXT)],
            CARGO_PACKAGE_NOT_FOUND | CARGO_PACKAGE_AMBIGUOUS | CARGO_PACKAGE_DUPLICATE => {
                &[("name", TEXT)]
            }
            CARGO_FEATURE_PACKAGE_NOT_SELECTED => &[("package", TEXT)],
            CARGO_FEATURE_NOT_FOUND => &[("package", TEXT), ("feature", TEXT)],
            CARGO_FEATURE_NOT_DECLARED => &[("feature", TEXT)],
            CARGO_CONFIG_TARGET_TYPE => &[("path", TEXT)],
            CARGO_TARGET_CFG_INVALID => &[("condition", TEXT)],
            CARGO_TARGET_CFG_QUERY_ERROR => &[("target", TEXT), ("detail", OPAQUE)],
            CARGO_OPERATION_ERROR => &[("path", TEXT), ("detail", OPAQUE)],
            CARGO_MANIFEST_UNAVAILABLE => &[("path", TEXT), ("detail", OPAQUE)],
            CARGO_ERROR | ARGUMENT_ERROR | OUTPUT_WRITE_ERROR | IO_ERROR => &[("detail", OPAQUE)],
            CLI_PARSE_ERROR => &[("kind", TEXT)],
            REPORT_SUMMARY => &[
                ("status", TEXT),
                ("errors", NUMBER),
                ("warnings", NUMBER),
                ("suppressed_warnings", NUMBER),
                ("diagnostics", NUMBER),
            ],
            _ => &[],
        }
    }
}

impl LocalizedMessage {
    pub fn new(id: MessageId) -> Self {
        Self {
            id,
            arguments: BTreeMap::new(),
        }
    }

    pub fn with_text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.arguments
            .insert(name.into(), MessageArgument::Text(value.into()));
        self
    }

    pub fn with_number(mut self, name: impl Into<String>, value: i64) -> Self {
        self.arguments
            .insert(name.into(), MessageArgument::Number(value));
        self
    }

    pub fn with_boolean(mut self, name: impl Into<String>, value: bool) -> Self {
        self.arguments
            .insert(name.into(), MessageArgument::Boolean(value));
        self
    }

    pub fn with_opaque(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.arguments
            .insert(name.into(), MessageArgument::Opaque(value.into()));
        self
    }

    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn arguments(&self) -> &BTreeMap<String, MessageArgument> {
        &self.arguments
    }

    pub fn validate(&self) -> Result<(), MessageSchemaError> {
        for (name, expected) in self.id.argument_schema() {
            let Some(argument) = self.arguments.get(*name) else {
                return Err(MessageSchemaError {
                    id: self.id,
                    argument: (*name).to_owned(),
                });
            };
            let actual = match argument {
                MessageArgument::Text(_) => MessageArgumentKind::Text,
                MessageArgument::Number(_) => MessageArgumentKind::Number,
                MessageArgument::Boolean(_) => MessageArgumentKind::Boolean,
                MessageArgument::Opaque(_) => MessageArgumentKind::Opaque,
            };
            if actual != *expected {
                return Err(MessageSchemaError {
                    id: self.id,
                    argument: (*name).to_owned(),
                });
            }
        }
        if let Some(name) = self.arguments.keys().find(|name| {
            !self
                .id
                .argument_schema()
                .iter()
                .any(|(expected, _)| expected == name)
        }) {
            return Err(MessageSchemaError {
                id: self.id,
                argument: name.clone(),
            });
        }
        Ok(())
    }
}

impl MessageSchemaError {
    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn argument(&self) -> &str {
        &self.argument
    }
}
