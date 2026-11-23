//! Planning for `COPY` over the canonical tree.

use std::collections::{BTreeMap, BTreeSet};

use dosker_image::{
    Attributes, DosName, DosPath, LayerDelta, Tree, TreeDir, TreeFile, TreeNode, fixed_timestamp,
};

use crate::context::{BuildContext, ResolvedSource, SourceEntry, SourceKind};
use crate::error::BuildError;
use crate::normalize::{is_reserved_path, render_tokens, resolve_copy_destination};

/// The resolved changes and cache inputs of one `COPY` instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyPlan {
    /// The entries the instruction adds or changes.
    pub delta: LayerDelta,
    /// The context-relative source paths.
    pub normalized_sources: Vec<String>,
    /// The resolved destination path.
    pub normalized_destination: DosPath,
    /// The normalized instruction text.
    pub normalized_instruction: String,
    /// The context digest followed by the source hashes.
    pub input_hashes: Vec<String>,
}

/// Resolves a `COPY` instruction against a context and the staged tree.
///
/// Every source and the destination are resolved before any entry is planned.
/// The returned delta contains only the entries the instruction adds or
/// changes, so applying it to `tree` produces the copied result.
pub fn plan_copy(
    context: &BuildContext,
    tree: &Tree,
    workdir: &DosPath,
    sources: &[String],
    destination: &str,
    line: usize,
) -> Result<CopyPlan, BuildError> {
    if sources.is_empty() {
        return Err(instruction_error(line, "COPY requires at least one source"));
    }

    let mut resolved_sources = Vec::with_capacity(sources.len());
    for source in sources {
        let resolved = context
            .resolve_source(source)
            .map_err(|error| instruction_error(line, format!("source {source}: {error}")))?;
        resolved_sources.push((source.clone(), resolved));
    }

    let (destination_path, destination_has_separator) =
        resolve_copy_destination(workdir, destination).map_err(|error| {
            instruction_error(line, format!("destination {destination}: {error}"))
        })?;
    let notice_destination = !destination_has_separator
        && resolved_sources.len() == 1
        && matches!(resolved_sources[0].1.kind, SourceKind::File)
        && is_notice_path(&destination_path);
    if is_reserved_path(&destination_path) && !notice_destination {
        return Err(instruction_error(
            line,
            format!(
                "destination {} is under the reserved DOSKER directory",
                destination_path.to_dos_string()
            ),
        ));
    }
    if resolved_sources.len() > 1 && !destination_has_separator {
        return Err(instruction_error(
            line,
            format!(
                "destination {} must end with a separator when copying multiple sources",
                destination_path.to_dos_string()
            ),
        ));
    }

    let directory_destination = destination_has_separator
        || resolved_sources.len() > 1
        || resolved_sources
            .iter()
            .any(|(_, source)| source.kind == SourceKind::Directory)
        || matches!(tree.get(&destination_path), Some(TreeNode::Dir(_)));

    let mut planned_files = BTreeMap::new();
    let mut planned_dirs = BTreeSet::new();
    if directory_destination && !destination_path.is_root() {
        planned_dirs.insert(destination_path.clone());
    }

    for (source_text, source) in &resolved_sources {
        match source.kind {
            SourceKind::File => {
                let name = source_file_name(source_text, source, line)?;
                let target = if directory_destination {
                    destination_path.join(&name)
                } else {
                    destination_path.clone()
                };
                let files = context.read_source_files(source).map_err(|error| {
                    instruction_error(line, format!("source {source_text}: {error}"))
                })?;
                let bytes = files
                    .into_iter()
                    .next()
                    .map(|file| file.bytes)
                    .ok_or_else(|| {
                        instruction_error(
                            line,
                            format!("source {source_text}: the file could not be read"),
                        )
                    })?;
                insert_file(&mut planned_files, &planned_dirs, target, bytes, line)?;
            }
            SourceKind::Directory => {
                let entries = context.read_source_entries(source).map_err(|error| {
                    instruction_error(line, format!("source {source_text}: {error}"))
                })?;
                for entry in entries {
                    let relative = strip_source_prefix(entry.relative(), &source.relative)
                        .ok_or_else(|| {
                            instruction_error(
                                line,
                                format!(
                                    "source {source_text}: {} is not under the source",
                                    entry.relative()
                                ),
                            )
                        })?;
                    let mut target = destination_path.clone();
                    for component in relative.split('/') {
                        let name = normalize_source_component(component, source_text, line)?;
                        target = target.join(&name);
                    }
                    match entry {
                        SourceEntry::File(file) => {
                            insert_file(
                                &mut planned_files,
                                &planned_dirs,
                                target,
                                file.bytes,
                                line,
                            )?;
                        }
                        SourceEntry::Directory(_) => {
                            insert_dir(&planned_files, &mut planned_dirs, target, line)?;
                        }
                    }
                }
            }
        }
    }

    let planned_dir_paths: Vec<DosPath> = planned_dirs.iter().cloned().collect();
    for dir_path in &planned_dir_paths {
        add_ancestor_dirs(dir_path, &mut planned_dirs);
    }
    for file_path in planned_files.keys() {
        add_ancestor_dirs(file_path, &mut planned_dirs);
    }
    if let Some(conflict) = planned_dirs
        .iter()
        .find(|dir| planned_files.contains_key(*dir))
    {
        return Err(instruction_error(
            line,
            format!(
                "destination {} is both a file and a directory",
                conflict.to_dos_string()
            ),
        ));
    }
    let only_notice_files = planned_files.keys().all(is_notice_path);
    for target in planned_files.keys().chain(planned_dirs.iter()) {
        if is_reserved_path(target)
            && !(only_notice_files && is_notice_path(target))
            && !(only_notice_files && is_notice_dir(target))
        {
            return Err(instruction_error(
                line,
                format!(
                    "destination {} is under the reserved DOSKER directory",
                    target.to_dos_string()
                ),
            ));
        }
    }

    let input_hashes = {
        let sources: Vec<ResolvedSource> = resolved_sources
            .iter()
            .map(|(_, source)| source.clone())
            .collect();
        context
            .source_hashes(&sources)
            .map_err(|error| instruction_error(line, format!("source hashes: {error}")))?
    };

    let delta = build_delta(tree, &planned_dirs, &planned_files, line)?;

    let normalized_sources = resolved_sources
        .iter()
        .map(|(_, source)| {
            if source.relative.is_empty() {
                ".".to_owned()
            } else {
                source.relative.clone()
            }
        })
        .collect::<Vec<_>>();
    let destination_token = render_destination(&destination_path, directory_destination);
    let mut tokens = vec!["COPY".to_owned()];
    tokens.extend(normalized_sources.iter().cloned());
    tokens.push(destination_token);
    let normalized_instruction = render_tokens(&tokens);

    Ok(CopyPlan {
        delta,
        normalized_sources,
        normalized_destination: destination_path,
        normalized_instruction,
        input_hashes,
    })
}

fn source_file_name(
    source_text: &str,
    source: &ResolvedSource,
    line: usize,
) -> Result<DosName, BuildError> {
    let name = source
        .relative
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            instruction_error(line, format!("source {source_text}: the file has no name"))
        })?;
    normalize_source_component(name, source_text, line)
}

fn normalize_source_component(
    component: &str,
    source_text: &str,
    line: usize,
) -> Result<DosName, BuildError> {
    DosName::normalize(component).map_err(|_| {
        instruction_error(
            line,
            format!("source {source_text}: invalid DOS 8.3 name {component}"),
        )
    })
}

fn strip_source_prefix<'a>(relative: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(relative);
    }
    let rest = relative.strip_prefix(prefix)?;
    rest.strip_prefix('/')
}

fn insert_file(
    planned_files: &mut BTreeMap<DosPath, Vec<u8>>,
    planned_dirs: &BTreeSet<DosPath>,
    target: DosPath,
    bytes: Vec<u8>,
    line: usize,
) -> Result<(), BuildError> {
    if planned_files.contains_key(&target) {
        return Err(instruction_error(
            line,
            format!(
                "two sources copy to the same destination {}",
                target.to_dos_string()
            ),
        ));
    }
    if planned_dirs.contains(&target) {
        return Err(instruction_error(
            line,
            format!(
                "destination {} is both a file and a directory",
                target.to_dos_string()
            ),
        ));
    }
    planned_files.insert(target, bytes);
    Ok(())
}

fn insert_dir(
    planned_files: &BTreeMap<DosPath, Vec<u8>>,
    planned_dirs: &mut BTreeSet<DosPath>,
    target: DosPath,
    line: usize,
) -> Result<(), BuildError> {
    if planned_files.contains_key(&target) {
        return Err(instruction_error(
            line,
            format!(
                "destination {} is both a file and a directory",
                target.to_dos_string()
            ),
        ));
    }
    planned_dirs.insert(target);
    Ok(())
}

fn is_notice_path(path: &DosPath) -> bool {
    path.to_dos_string()
        .eq_ignore_ascii_case(crate::registry::NOTICE_PATH)
}

fn is_notice_dir(path: &DosPath) -> bool {
    path.to_dos_string().eq_ignore_ascii_case(r"C:\DOSKER")
}

fn add_ancestor_dirs(path: &DosPath, planned_dirs: &mut BTreeSet<DosPath>) {
    let mut parent = path.parent();
    while let Some(current) = parent {
        if current.is_root() {
            break;
        }
        planned_dirs.insert(current.clone());
        parent = current.parent();
    }
}

fn build_delta(
    tree: &Tree,
    planned_dirs: &BTreeSet<DosPath>,
    planned_files: &BTreeMap<DosPath, Vec<u8>>,
    line: usize,
) -> Result<LayerDelta, BuildError> {
    let mut delta = LayerDelta::new();
    for path in planned_dirs {
        match tree.get(path) {
            Some(TreeNode::Dir(_)) => {}
            Some(TreeNode::File(_)) => {
                return Err(instruction_error(
                    line,
                    format!(
                        "destination {} is a file, not a directory",
                        path.to_dos_string()
                    ),
                ));
            }
            None => delta.add_dir(TreeDir {
                path: path.clone(),
                attributes: Attributes::default(),
                timestamp: fixed_timestamp(),
            }),
        }
    }
    for (path, bytes) in planned_files {
        let attributes = Attributes {
            archive: true,
            ..Attributes::default()
        };
        match tree.get(path) {
            Some(TreeNode::File(existing))
                if existing.bytes == *bytes
                    && existing.attributes == attributes
                    && existing.timestamp == fixed_timestamp() => {}
            Some(TreeNode::File(_)) => delta.add_file(TreeFile {
                path: path.clone(),
                attributes,
                timestamp: fixed_timestamp(),
                bytes: bytes.clone(),
            }),
            Some(TreeNode::Dir(_)) => {
                return Err(instruction_error(
                    line,
                    format!(
                        "destination {} is a directory, not a file",
                        path.to_dos_string()
                    ),
                ));
            }
            None => delta.add_file(TreeFile {
                path: path.clone(),
                attributes,
                timestamp: fixed_timestamp(),
                bytes: bytes.clone(),
            }),
        }
    }
    Ok(delta)
}

fn render_destination(path: &DosPath, is_directory: bool) -> String {
    let mut text = path.to_dos_string();
    if is_directory && !text.ends_with('\\') {
        text.push('\\');
    }
    text
}

fn instruction_error(line: usize, reason: impl Into<String>) -> BuildError {
    BuildError::Instruction {
        line,
        instruction: "COPY".to_owned(),
        reason: reason.into(),
    }
}
