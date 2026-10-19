use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use cop_model::MessageId;
use fluent_syntax::{
    ast::{Entry, Expression, InlineExpression, Pattern, PatternElement},
    parser,
};
use unic_langid::LanguageIdentifier;

fn main() {
    println!("cargo:rerun-if-changed=locales");
    if let Err(errors) = validate_catalogs() {
        for error in errors {
            eprintln!("{error}");
        }
        panic!("Fluent catalog validation failed");
    }
}

fn validate_catalogs() -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let expected = MessageId::ALL
        .iter()
        .map(|message_id| (message_id.as_str().to_owned(), *message_id))
        .collect::<BTreeMap<_, _>>();
    if expected.len() != MessageId::ALL.len() {
        errors.push("cop-model::MessageId::ALL contains duplicate IDs".to_owned());
    }

    let locales_path = PathBuf::from("locales");
    let locale_entries = match fs::read_dir(&locales_path) {
        Ok(entries) => entries,
        Err(error) => {
            return Err(vec![format!(
                "could not read {}: {error}",
                locales_path.display()
            )]);
        }
    };
    let mut found_locales = BTreeSet::new();
    let mut supported_locales = BTreeSet::new();
    for entry in locale_entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("could not read locale directory entry: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let locale = entry.file_name().to_string_lossy().into_owned();
        found_locales.insert(locale.clone());
        match locale.parse::<LanguageIdentifier>() {
            Ok(locale_id) => {
                let locale_id = locale_id.to_string();
                if !supported_locales.insert(locale_id.clone()) {
                    errors.push(format!("duplicate locale identifier {locale_id}"));
                }
            }
            Err(error) => errors.push(format!("invalid locale directory {locale}: {error}")),
        }
        let mut files = Vec::new();
        if let Err(error) = collect_fluent_files(&entry.path(), &mut files) {
            errors.push(format!("locale {locale}: {error}"));
            continue;
        }
        let mut messages = BTreeMap::new();
        for path in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let source = match fs::read_to_string(&path) {
                Ok(source) => source,
                Err(error) => {
                    errors.push(format!("{}: could not read file: {error}", path.display()));
                    continue;
                }
            };
            let resource = match parser::parse(source.as_str()) {
                Ok(resource) => resource,
                Err((_, parse_errors)) => {
                    errors.push(format!(
                        "{}: invalid Fluent syntax: {parse_errors:?}",
                        path.display()
                    ));
                    continue;
                }
            };
            for entry in resource.body {
                if let Entry::Message(message) = entry {
                    let id = message.id.name.to_owned();
                    let mut arguments = BTreeSet::new();
                    if let Some(pattern) = message.value.as_ref() {
                        collect_pattern(pattern, &mut arguments);
                    } else {
                        errors.push(format!("locale {locale}: message {id} has no value"));
                    }
                    if messages.insert(id.clone(), arguments).is_some() {
                        errors.push(format!("locale {locale}: duplicate message ID {id}"));
                    }
                }
            }
        }
        validate_locale_messages(&locale, &messages, &expected, &mut errors);
    }

    for required in ["zh-CN", "en-US"] {
        if !found_locales.contains(required) {
            errors.push(format!("required locale directory {required} is missing"));
        }
    }
    if errors.is_empty() {
        let mut source = String::from("const SUPPORTED_LOCALES: &[&str] = &[\n");
        for locale in supported_locales {
            source.push_str(&format!("    {locale:?},\n"));
        }
        source.push_str("];\n");
        let output_directory =
            env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR for build scripts");
        fs::write(
            PathBuf::from(output_directory).join("supported_locales.rs"),
            source,
        )
        .expect("could not write supported locale list");
        Ok(())
    } else {
        Err(errors)
    }
}

fn collect_fluent_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            collect_fluent_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "ftl") {
            files.push(path);
        }
    }
    Ok(())
}

fn validate_locale_messages(
    locale: &str,
    messages: &BTreeMap<String, BTreeSet<String>>,
    expected: &BTreeMap<String, MessageId>,
    errors: &mut Vec<String>,
) {
    for message_id in expected.keys() {
        let Some(actual_arguments) = messages.get(message_id) else {
            errors.push(format!("locale {locale}: missing message ID {message_id}"));
            continue;
        };
        let expected_arguments = expected[message_id]
            .argument_schema()
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        if actual_arguments != &expected_arguments {
            errors.push(format!(
                "locale {locale}: message {message_id} arguments differ; expected {expected_arguments:?}, found {actual_arguments:?}"
            ));
        }
    }
    for message_id in messages.keys() {
        if !expected.contains_key(message_id) {
            errors.push(format!(
                "locale {locale}: unexpected message ID {message_id}"
            ));
        }
    }
}

fn collect_pattern(pattern: &Pattern<&str>, arguments: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let PatternElement::Placeable { expression } = element {
            collect_expression(expression, arguments);
        }
    }
}

fn collect_expression(expression: &Expression<&str>, arguments: &mut BTreeSet<String>) {
    match expression {
        Expression::Select { selector, variants } => {
            collect_inline_expression(selector, arguments);
            for variant in variants {
                collect_pattern(&variant.value, arguments);
            }
        }
        Expression::Inline(expression) => collect_inline_expression(expression, arguments),
    }
}

fn collect_inline_expression(
    expression: &InlineExpression<&str>,
    arguments: &mut BTreeSet<String>,
) {
    match expression {
        InlineExpression::VariableReference { id } => {
            arguments.insert(id.name.to_owned());
        }
        InlineExpression::Placeable { expression } => collect_expression(expression, arguments),
        InlineExpression::FunctionReference {
            arguments: values, ..
        } => {
            for value in &values.positional {
                collect_inline_expression(value, arguments);
            }
            for value in &values.named {
                collect_inline_expression(&value.value, arguments);
            }
        }
        InlineExpression::TermReference {
            arguments: Some(values),
            ..
        } => {
            for value in &values.positional {
                collect_inline_expression(value, arguments);
            }
            for value in &values.named {
                collect_inline_expression(&value.value, arguments);
            }
        }
        InlineExpression::StringLiteral { .. }
        | InlineExpression::NumberLiteral { .. }
        | InlineExpression::MessageReference { .. }
        | InlineExpression::TermReference {
            arguments: None, ..
        } => {}
    }
}
