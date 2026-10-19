mod index;
mod local_bindings;

use std::path::{Path, PathBuf};

use cop_model::SourceSpan;
pub use index::{PathResolution, SymbolIndex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Module,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
    UseAlias,
    ExternalCrate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolVisibility {
    Private,
    Public,
    Restricted(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathResolutionKind {
    Local,
    ExternalCrate,
    Unresolved,
    Ambiguous,
    AliasCycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityResult {
    Public,
    Reachable,
    Private,
    Unknown,
}

pub(crate) struct SymbolRecordData {
    pub(crate) name: String,
    pub(crate) kind: SymbolKind,
    pub(crate) visibility: SymbolVisibility,
    pub(crate) context_index: usize,
    pub(crate) target_name: String,
    pub(crate) namespace: Vec<String>,
    pub(crate) target_namespace: Option<Vec<String>>,
    pub(crate) target_path: Option<Vec<String>>,
    pub(crate) external_crate: Option<String>,
    pub(crate) file_path: PathBuf,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) cfg_condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolRecord {
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    context_index: usize,
    target_name: String,
    namespace: Vec<String>,
    target_namespace: Option<Vec<String>>,
    target_path: Option<Vec<String>>,
    external_crate: Option<String>,
    file_path: PathBuf,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

impl SymbolRecord {
    pub(crate) fn new(data: SymbolRecordData) -> Self {
        Self {
            name: data.name,
            kind: data.kind,
            visibility: data.visibility,
            context_index: data.context_index,
            target_name: data.target_name,
            namespace: data.namespace,
            target_namespace: data.target_namespace,
            target_path: data.target_path,
            external_crate: data.external_crate,
            file_path: data.file_path,
            span: data.span,
            cfg_condition: data.cfg_condition,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> SymbolKind {
        self.kind
    }

    pub fn visibility(&self) -> &SymbolVisibility {
        &self.visibility
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub(crate) fn context_index(&self) -> usize {
        self.context_index
    }

    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    pub fn target_namespace(&self) -> Option<&[String]> {
        self.target_namespace.as_deref()
    }

    pub fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub fn external_crate(&self) -> Option<&str> {
        self.external_crate.as_deref()
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.cfg_condition.as_deref()
    }

    pub fn is_visible_from(&self, namespace: &[String]) -> VisibilityResult {
        match &self.visibility {
            SymbolVisibility::Public => VisibilityResult::Public,
            SymbolVisibility::Private => {
                if namespace.starts_with(&self.namespace) {
                    VisibilityResult::Reachable
                } else {
                    VisibilityResult::Private
                }
            }
            SymbolVisibility::Restricted(scope) => {
                let Some(restricted) = restricted_namespace(scope, &self.namespace) else {
                    return VisibilityResult::Unknown;
                };
                if namespace.starts_with(&restricted) {
                    VisibilityResult::Reachable
                } else {
                    VisibilityResult::Private
                }
            }
        }
    }
}

fn restricted_namespace(scope: &str, declaration_namespace: &[String]) -> Option<Vec<String>> {
    let restriction = scope.trim_matches(['(', ')', ' ']);
    match restriction {
        "crate" => Some(Vec::new()),
        "self" => Some(declaration_namespace.to_vec()),
        "super" => {
            Some(declaration_namespace[..declaration_namespace.len().saturating_sub(1)].to_vec())
        }
        value if value.starts_with("in ") => {
            let path = value.strip_prefix("in ")?.trim();
            if path == "crate" {
                return Some(Vec::new());
            }
            if path == "self" {
                return Some(declaration_namespace.to_vec());
            }
            let mut namespace = declaration_namespace.to_vec();
            for segment in path.split("::") {
                match segment {
                    "crate" => namespace.clear(),
                    "self" => {}
                    "super" => {
                        namespace.pop()?;
                    }
                    "" => {}
                    name => namespace.push(name.to_owned()),
                }
            }
            Some(namespace)
        }
        _ => None,
    }
}
