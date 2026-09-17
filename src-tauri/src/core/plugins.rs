//! Plugin domain boundary.
//!
//! Plugins are source-backed packages of library skills. This module owns the
//! Cursor manifest boundary, package graph semantics, machine-local setup
//! adapters, and deployment reconciliation. Tauri commands and the CLI only
//! translate transport values into these domain operations.

use anyhow::{anyhow, bail, Context, Result};
use sha2::Digest;
use serde::{Deserialize, Serialize};
use crate::core::{
    central_repo, content_hash, git_fetcher, installer, scenario_service, skill_metadata,
    sync_engine, sync_metadata, tool_adapters,
    skill_store::{PluginManagedTargetRecord, PluginRecord, SkillRecord, SkillStore},
};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

/// Portable source provenance retained for a plugin package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSource {
    pub source_ref: String,
    pub source_ref_resolved: String,
    pub branch: Option<String>,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAuthor {
    pub name: String,
}

/// A skill declared by an imported Cursor plugin, in deterministic manifest
/// traversal order. `relative_path` is always relative to the plugin's skills
/// root with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredPluginSkill {
    pub relative_path: String,
    pub name: String,
    pub description: Option<String>,
}

/// Immutable result of inspecting one checked-out Cursor plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPluginPreview {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub homepage: Option<String>,
    pub author: Option<PluginAuthor>,
    pub source: PluginSource,
    pub skills: Vec<DeclaredPluginSkill>,
    pub ignored_agents: bool,
    pub ignored_rules: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorManifest {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    author: Option<CursorAuthor>,
    skills: String,
    #[serde(default)]
    agents: Option<String>,
    #[serde(default)]
    rules: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorAuthor {
    name: String,
}

/// Inspect a checked-out Cursor plugin without copying any files.
///
/// Every path is constrained to the checked-out root. This rejects path
/// traversal, absolute paths, and symlink escapes before a manifest-controlled
/// location is observed.
pub fn preview_cursor_plugin(root: &Path, source: PluginSource) -> Result<CursorPluginPreview> {
    let root = root
        .canonicalize()
        .with_context(|| format!("plugin root does not exist: {}", root.display()))?;
    if !root.is_dir() {
        bail!("plugin root is not a directory: {}", root.display());
    }

    let manifest_path = root.join(".cursor-plugin").join("plugin.json");
    let manifest_path = canonical_path_inside(&root, &manifest_path, "plugin manifest")?;
    if !manifest_path.is_file() {
        bail!("Cursor plugin manifest is not a file");
    }
    let manifest: CursorManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .context("invalid Cursor plugin manifest JSON")?;

    validate_manifest_name(&manifest.name)?;
    let skills_root = resolve_manifest_directory(&root, &manifest.skills, "skills")?;
    let skills = collect_declared_skills(&root, &skills_root)?;
    if skills.is_empty() {
        bail!("Cursor plugin skills directory contains no valid SKILL.md descendants");
    }

    Ok(CursorPluginPreview {
        display_name: manifest
            .display_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&manifest.name)
            .to_string(),
        name: manifest.name,
        description: manifest.description.filter(|value| !value.trim().is_empty()),
        version: manifest.version.filter(|value| !value.trim().is_empty()),
        homepage: manifest.homepage.filter(|value| !value.trim().is_empty()),
        author: manifest.author.map(|author| PluginAuthor { name: author.name }),
        source,
        skills,
        ignored_agents: manifest.agents.is_some(),
        ignored_rules: manifest.rules.is_some(),
    })
}

fn validate_manifest_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || name.starts_with(['-', '_', '.'])
    {
        bail!("Cursor plugin manifest has an invalid name");
    }
    Ok(())
}

fn resolve_manifest_directory(root: &Path, declared: &str, label: &str) -> Result<PathBuf> {
    let declared_path = Path::new(declared);
    if declared.trim().is_empty() || declared_path.is_absolute() {
        bail!("Cursor plugin {label} path must be a relative directory inside the plugin root");
    }
    if declared_path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        bail!("Cursor plugin {label} path must not traverse outside the plugin root");
    }
    let candidate = root.join(declared_path);
    let resolved = canonical_path_inside(root, &candidate, label)?;
    if !resolved.is_dir() {
        bail!("Cursor plugin {label} path is not a directory");
    }
    Ok(resolved)
}

fn canonical_path_inside(root: &Path, candidate: &Path, label: &str) -> Result<PathBuf> {
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("Cursor plugin {label} does not exist"))?;
    if !resolved.starts_with(root) {
        bail!("Cursor plugin {label} resolves outside the plugin root");
    }
    Ok(resolved)
}

fn collect_declared_skills(root: &Path, skills_root: &Path) -> Result<Vec<DeclaredPluginSkill>> {
    let mut skills = Vec::new();
    for entry in WalkDir::new(skills_root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_dir() || !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        let skill_dir = entry.path().canonicalize()?;
        if !skill_dir.starts_with(root) {
            bail!("Cursor plugin skill directory resolves outside the plugin root");
        }
        let metadata = crate::core::skill_metadata::parse_skill_md(&skill_dir);
        let relative_path = skill_dir
            .strip_prefix(skills_root)
            .map_err(|_| anyhow!("skill is outside declared skills root"))?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let name = metadata.name.filter(|name| !name.trim().is_empty()).unwrap_or_else(|| {
            skill_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| relative_path.clone())
        });
        skills.push(DeclaredPluginSkill {
            relative_path,
            name,
            description: metadata.description.filter(|value| !value.trim().is_empty()),
        });
    }
    skills.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(skills)
}
/// Resolve active roots to a de-duplicated dependency-first closure. Missing
/// references, self edges, and cycles are invalid portable plugin state.
pub fn dependency_first_closure(
    roots: &[String],
    dependencies: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    fn visit(
        id: &str,
        dependencies: &std::collections::BTreeMap<String, Vec<String>>,
        visiting: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(id) {
            return Ok(());
        }
        if !dependencies.contains_key(id) {
            bail!("plugin dependency references missing plugin: {id}");
        }
        if !visiting.insert(id.to_string()) {
            bail!("plugin dependency graph contains a cycle at {id}");
        }
        let mut children = dependencies[id].clone();
        children.sort();
        children.dedup();
        for dependency in children {
            if dependency == id {
                bail!("plugin dependency graph contains a self dependency: {id}");
            }
            visit(&dependency, dependencies, visiting, visited, ordered)?;
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        ordered.push(id.to_string());
        Ok(())
    }

    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
    for root in roots {
        visit(root, dependencies, &mut visiting, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}


/// Persist an imported Cursor package and every manifest-declared skill. This
/// is intentionally the only package importer: callers never scan arbitrary
/// repository directories or add undeclared skills.
pub fn import_cursor_plugin(
    store: &SkillStore,
    repo_url: &str,
    temp_dir: &Path,
    source: PluginSource,
) -> Result<PluginRecord> {
    let temp_dir = validate_plugin_clone(temp_dir)?;
    let parsed = git_fetcher::parse_git_source_resolved(repo_url, None);
    let root = resolve_plugin_root(&temp_dir, parsed.subpath.as_deref())?;
    let preview = preview_cursor_plugin(&root, source)?;
    let skills_root = resolve_manifest_directory(
        &root,
        &serde_json::from_slice::<CursorManifest>(&std::fs::read(root.join(".cursor-plugin/plugin.json"))?)?
            .skills,
        "skills",
    )?;
    let revision = git_fetcher::get_head_revision(&temp_dir)?;
    let now = chrono::Utc::now().timestamp_millis();
    let plugin_id = uuid::Uuid::new_v4().to_string();
    let plugin = PluginRecord {
        id: plugin_id.clone(),
        slug: unique_plugin_slug(store, &preview.name)?,
        kind: "cursor".to_string(),
        name: preview.name,
        display_name: preview.display_name,
        description: preview.description,
        version: preview.version,
        source_ref: Some(repo_url.to_string()),
        source_ref_resolved: Some(parsed.clone_url.clone()),
        source_branch: parsed.branch.clone(),
        source_revision: Some(revision.clone()),
        author: preview.author.map(|author| author.name),
        homepage: preview.homepage,
        active: false,
        created_at: now,
        updated_at: now,
    };

    let mut skill_ids = Vec::new();
    for declared in preview.skills {
        let source_dir = skills_root.join(&declared.relative_path);
        let subpath = git_fetcher::relative_subpath(&temp_dir, &source_dir);
        let skill_id = if let Some(skill) = reusable_plugin_skill(
            store,
            &parsed.clone_url,
            parsed.branch.as_deref(),
            subpath.as_deref(),
            &declared.name,
        )? {
            skill.id
        } else {
            let installed = installer::install_from_git_dir(&source_dir, Some(&declared.name))?;
            // `install_from_git_dir` returns an existing library directory only
            // when its content hash matches the declared source. Reuse that
            // record instead of attempting a second row for its unique path:
            // a pre-existing equivalent skill remains its own source record
            // while becoming a member of this package.
            if let Some(skill) = store.get_skill_by_central_path(
                &installed.central_path.to_string_lossy(),
            )? {
                skill.id
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                store.insert_skill(&SkillRecord {
                    id: id.clone(),
                    name: installed.name,
                    description: installed.description,
                    source_type: "git".to_string(),
                    source_ref: Some(repo_url.to_string()),
                    source_ref_resolved: Some(parsed.clone_url.clone()),
                    source_subpath: subpath,
                    source_branch: parsed.branch.clone(),
                    source_revision: Some(revision.clone()),
                    remote_revision: Some(revision.clone()),
                    central_path: installed.central_path.to_string_lossy().to_string(),
                    content_hash: Some(installed.content_hash),
                    enabled: true,
                    created_at: now,
                    updated_at: now,
                    status: "ok".to_string(),
                    update_status: "up_to_date".to_string(),
                    last_checked_at: Some(now),
                    last_check_error: None,
                })?;
                id
            }
        };
        skill_ids.push(skill_id);
    }
    store.upsert_plugin(&plugin)?;
    store.replace_plugin_skills(&plugin.id, &skill_ids)?;
    sync_metadata::write_all_from_db_unlocked(store)?;
    Ok(plugin)
}

fn reusable_plugin_skill(
    store: &SkillStore,
    source_ref_resolved: &str,
    source_branch: Option<&str>,
    source_subpath: Option<&str>,
    declared_name: &str,
) -> Result<Option<SkillRecord>> {
    let skills = store.get_all_skills()?;
    if let Some(skill) = skills.iter().find(|skill| {
        skill.source_ref_resolved.as_deref() == Some(source_ref_resolved)
            && skill.source_branch.as_deref() == source_branch
            && skill.source_subpath.as_deref() == source_subpath
    }) {
        return Ok(Some(skill.clone()));
    }

    let Some(identity) = canonical_skill_name(declared_name) else {
        return Ok(None);
    };
    let matches: Vec<_> = skills
        .into_iter()
        .filter(|skill| canonical_skill_name(&skill.name).as_deref() == Some(identity.as_str()))
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [skill] => Ok(Some(skill.clone())),
        _ => bail!(
            "cannot safely select a package skill for {declared_name}: {} library skills have that name",
            matches.len()
        ),
    }
}

fn canonical_skill_name(name: &str) -> Option<String> {
    skill_metadata::sanitize_skill_name(name).map(|name| name.to_lowercase())
}

/// Replace imported duplicate members with their pre-existing semantic skill
/// records. A package source is not permitted to overwrite a local variant:
/// the existing library skill is selected instead. Only unchanged, now-orphaned
/// package copies are removed.
pub fn reconcile_plugin_skill_members(store: &SkillStore, plugin_id: &str) -> Result<usize> {
    let plugin = store
        .get_plugin_by_id(plugin_id)?
        .ok_or_else(|| anyhow!("plugin not found"))?;
    if plugin.active {
        bail!("deactivate the plugin before reconciling its skills");
    }
    let member_ids = store.get_plugin_skill_ids(plugin_id)?;
    let all_skills = store.get_all_skills()?;
    let mut replacements = Vec::new();
    let mut members = Vec::with_capacity(member_ids.len());

    for member_id in &member_ids {
        let member = all_skills
            .iter()
            .find(|skill| skill.id == *member_id)
            .ok_or_else(|| anyhow!("plugin references an unknown skill"))?;
        let identity = canonical_skill_name(&skill_metadata::infer_skill_name(
            Path::new(&member.central_path),
        ));
        let replacement = identity.and_then(|identity| {
            let candidates: Vec<_> = all_skills
                .iter()
                .filter(|skill| {
                    skill.id != member.id
                        && skill.source_ref_resolved != plugin.source_ref_resolved
                        && canonical_skill_name(&skill.name).as_deref() == Some(identity.as_str())
                })
                .collect();
            (candidates.len() == 1).then(|| candidates[0])
        });
        if let Some(replacement) = replacement {
            replacements.push((member.clone(), replacement.id.clone()));
            members.push(replacement.id.clone());
        } else {
            members.push(member.id.clone());
        }
    }

    if replacements.is_empty() {
        return Ok(0);
    }
    store.replace_plugin_skills(plugin_id, &members)?;
    for (duplicate, _) in &replacements {
        remove_unchanged_orphaned_plugin_copy(store, duplicate, &plugin)?;
    }
    sync_metadata::write_all_from_db_unlocked(store)?;
    Ok(replacements.len())
}

fn remove_unchanged_orphaned_plugin_copy(
    store: &SkillStore,
    skill: &SkillRecord,
    plugin: &PluginRecord,
) -> Result<()> {
    if skill.source_ref_resolved != plugin.source_ref_resolved {
        return Ok(());
    }
    let path = PathBuf::from(&skill.central_path);
    let unchanged = skill.content_hash.as_deref().is_some_and(|expected| {
        content_hash::hash_directory(&path).ok().as_deref() == Some(expected)
    });
    if !unchanged || !store.delete_skill_if_unreferenced(&skill.id)? {
        return Ok(());
    }

    let root = central_repo::skills_dir().canonicalize()?;
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(&root) || resolved == root {
        bail!("refusing to remove plugin skill outside the library");
    }
    std::fs::remove_dir_all(resolved)?;
    Ok(())
}

fn validate_plugin_clone(temp_dir: &Path) -> Result<PathBuf> {
    let resolved = temp_dir
        .canonicalize()
        .with_context(|| format!("plugin clone session does not exist: {}", temp_dir.display()))?;
    let system_temp = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());
    let valid_name = resolved
        .file_name()
        .map(|name| name.to_string_lossy().starts_with(git_fetcher::CLONE_TEMP_PREFIX))
        .unwrap_or(false);
    if !resolved.starts_with(system_temp) || !valid_name {
        bail!("invalid plugin clone session");
    }
    Ok(resolved)
}

fn resolve_plugin_root(temp_dir: &Path, subpath: Option<&str>) -> Result<PathBuf> {
    if let Some(subpath) = subpath {
        let candidate = temp_dir.join(subpath);
        if candidate.join(".cursor-plugin/plugin.json").is_file() {
            return canonical_path_inside(temp_dir, &candidate, "plugin root");
        }
    }
    if temp_dir.join(".cursor-plugin/plugin.json").is_file() {
        return Ok(temp_dir.to_path_buf());
    }
    bail!("Cursor plugin manifest not found in checked-out source")
}

fn unique_plugin_slug(store: &SkillStore, base: &str) -> Result<String> {
    let normalized = base
        .to_ascii_lowercase()
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        bail!("plugin name cannot produce a slug");
    }
    let existing: std::collections::BTreeSet<String> = store
        .get_all_plugins()?
        .into_iter()
        .map(|plugin| plugin.slug.to_ascii_lowercase())
        .collect();
    if !existing.contains(&normalized) {
        return Ok(normalized);
    }
    for suffix in 2.. {
        let candidate = format!("{normalized}-{suffix}");
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
    }
    unreachable!("unbounded suffix search always finds an unused plugin slug")
}

/// Activate one portable root and reconcile the union of every active closure
/// to enabled, installed coding adapters. All target preflights complete before
/// the root state or any target is changed.
pub fn activate_plugin(store: &SkillStore, plugin_id: &str) -> Result<()> {
    let mut roots = store.get_active_plugin_ids()?;
    if !roots.iter().any(|root| root == plugin_id) {
        roots.push(plugin_id.to_string());
    }
    let before = store
        .get_all_targets()?
        .into_iter()
        .map(|target| (target.skill_id, target.tool))
        .collect::<std::collections::HashSet<_>>();
    reconcile_roots(store, &roots, Some(plugin_id), &before)?;
    store.set_plugin_active(plugin_id, true)?;
    sync_metadata::write_all_from_db_unlocked(store)?;
    Ok(())
}

/// Remove exactly one active root. Files are deleted only when no remaining
/// root needs their skill and a machine-local plugin ownership row proves this
/// manager created the target.
pub fn deactivate_plugin(store: &SkillStore, plugin_id: &str) -> Result<()> {
    let roots: Vec<String> = store
        .get_active_plugin_ids()?
        .into_iter()
        .filter(|root| root != plugin_id)
        .collect();
    let remaining = required_skill_ids(store, &roots)?;
    store.set_plugin_active(plugin_id, false)?;
    let all_targets = store.get_all_targets()?;
    for managed in store.list_plugin_managed_targets()? {
        if remaining.contains(&managed.skill_id) {
            continue;
        }
        if let Some(target) = all_targets
            .iter()
            .find(|target| target.skill_id == managed.skill_id && target.tool == managed.tool)
        {
            let shared_path = all_targets.iter().any(|other| {
                other.target_path == target.target_path
                    && !(other.skill_id == target.skill_id && other.tool == target.tool)
            });
            if !shared_path {
                let _ = sync_engine::remove_recorded_target(
                    Path::new(&target.target_path),
                    &target.mode,
                );
            }
            store.delete_target(&target.skill_id, &target.tool)?;
        }
        store.delete_plugin_managed_target(&managed.plugin_id, &managed.skill_id, &managed.tool)?;
    }
    sync_metadata::write_all_from_db_unlocked(store)?;
    Ok(())
}

/// Startup and post-merge reconciliation. Portable active roots are applied to
/// locally enabled adapters; local setup and ownership records never enter Git.
pub fn reconcile_active_plugins(store: &SkillStore) -> Result<()> {
    let roots = store.get_active_plugin_ids()?;
    if roots.is_empty() {
        return Ok(());
    }
    let before = store
        .get_all_targets()?
        .into_iter()
        .map(|target| (target.skill_id, target.tool))
        .collect::<std::collections::HashSet<_>>();
    for root in &roots {
        reconcile_roots(store, &roots, Some(root), &before)?;
    }
    Ok(())
}

fn reconcile_roots(
    store: &SkillStore,
    roots: &[String],
    owner: Option<&str>,
    before: &std::collections::HashSet<(String, String)>,
) -> Result<()> {
    let skills = required_skill_ids(store, roots)?;
    let tools: Vec<String> = tool_adapters::enabled_installed_adapters(store)
        .into_iter()
        .map(|adapter| adapter.key)
        .collect();
    scenario_service::apply_skills_to_tools(
        store,
        &skills,
        &tools,
        scenario_service::BatchApplyMode::Add,
    )
    .map_err(|error| anyhow!(error.to_string()))?;
    if let Some(owner) = owner {
        let owner_skills = required_skill_ids(store, &[owner.to_string()])?;
        let after = store
            .get_all_targets()?
            .into_iter()
            .map(|target| (target.skill_id, target.tool))
            .collect::<std::collections::HashSet<_>>();
        for skill_id in owner_skills {
            for tool in &tools {
                if after.contains(&(skill_id.clone(), tool.clone()))
                    && !before.contains(&(skill_id.clone(), tool.clone()))
                {
                    store.add_plugin_managed_target(&PluginManagedTargetRecord {
                        plugin_id: owner.to_string(),
                        skill_id: skill_id.clone(),
                        tool: tool.clone(),
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn required_skill_ids(store: &SkillStore, roots: &[String]) -> Result<Vec<String>> {
    let plugins = store.get_all_plugins()?;
    let dependencies = plugins
        .iter()
        .map(|plugin| Ok((plugin.id.clone(), store.get_plugin_dependency_ids(&plugin.id)?)))
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let ordered = dependency_first_closure(roots, &dependencies)?;
    let mut seen = std::collections::HashSet::new();
    let mut skill_ids = Vec::new();
    for plugin_id in ordered {
        for skill_id in store.get_plugin_skill_ids(&plugin_id)? {
            if seen.insert(skill_id.clone()) {
                skill_ids.push(skill_id);
            }
        }
    }
    Ok(skill_ids)
}

/// Machine-local documents maintained by Pstack integrations. They are never
/// serialized into portable plugin metadata.
pub fn pstack_document_paths() -> Vec<(String, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        ("agents-models".to_string(), home.join(".agents/pstack-models.md")),
        (
            "cursor-models".to_string(),
            home.join(".cursor/rules/pstack-models.mdc"),
        ),
    ]
}

pub fn capture_pstack_document(store: &SkillStore, plugin_id: &str, document_key: &str) -> Result<()> {
    let (_, path) = pstack_document_paths()
        .into_iter()
        .find(|(key, _)| key == document_key)
        .ok_or_else(|| anyhow!("unknown Pstack document: {document_key}"))?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let content_hash = sha2::Sha256::digest(content.as_bytes());
    store.upsert_plugin_local_document(&crate::core::skill_store::PluginLocalDocumentRecord {
        plugin_id: plugin_id.to_string(),
        document_key: document_key.to_string(),
        content,
        content_hash: format!("{content_hash:x}"),
        updated_at: chrono::Utc::now().timestamp_millis(),
    })
}

/// Restore only if the recorded document still matches the local target. A
/// changed target is an explicit conflict, never an overwrite.
pub fn restore_pstack_document(store: &SkillStore, plugin_id: &str, document_key: &str) -> Result<()> {
    let document = store
        .get_plugin_local_document(plugin_id, document_key)?
        .ok_or_else(|| anyhow!("no saved Pstack document: {document_key}"))?;
    let (_, path) = pstack_document_paths()
        .into_iter()
        .find(|(key, _)| key == document_key)
        .ok_or_else(|| anyhow!("unknown Pstack document: {document_key}"))?;
    if path.exists() {
        let current = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let current_hash = sha2::Sha256::digest(current.as_bytes());
        if format!("{current_hash:x}") != document.content_hash {
            bail!("Pstack document conflict at {}; local content changed", path.display());
        }
    } else if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, document.content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::central_repo;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn source() -> PluginSource {
        PluginSource {
            source_ref: "https://example.test/org/plugin".to_string(),
            source_ref_resolved: "https://example.test/org/plugin.git".to_string(),
            branch: Some("main".to_string()),
            revision: Some("abc123".to_string()),
        }
    }

    fn write_manifest(root: &Path, skills: &str) {
        fs::create_dir_all(root.join(".cursor-plugin")).unwrap();
        fs::write(
            root.join(".cursor-plugin/plugin.json"),
            format!(r#"{{"name":"example-plugin","displayName":"Example","skills":"{skills}","agents":"./agents"}}"#),
        )
        .unwrap();
    }

    fn write_skill(root: &Path, relative: &str) {
        let skill = root.join(relative);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: Example Skill\ndescription: Example\n---\n").unwrap();
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn import_reuses_existing_named_library_record_without_overwriting_it() {
        let _guard = central_repo::test_base_dir_lock();
        let temp = tempdir().unwrap();
        let base = temp.path().join("library");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        let skills_dir = central_repo::skills_dir();
        fs::create_dir_all(&skills_dir).unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();

        let clone = std::env::temp_dir()
            .join(format!("{}{}", git_fetcher::CLONE_TEMP_PREFIX, uuid::Uuid::new_v4()));
        fs::create_dir_all(&clone).unwrap();
        write_manifest(&clone, "./skills");
        write_skill(&clone, "skills/one");
        git(&clone, &["init", "-b", "main"]);
        git(&clone, &["add", "."]);
        git(
            &clone,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-m",
                "fixture",
            ],
        );

        let existing_path = skills_dir.join("example-skill");
        write_skill(&skills_dir, "example-skill");
        fs::write(
            existing_path.join("SKILL.md"),
            "---\nname: Example Skill\ndescription: Local improvements\n---\n",
        )
        .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        store
            .insert_skill(&SkillRecord {
                id: "existing-skill".to_string(),
                name: "Example Skill".to_string(),
                description: Some("Local improvements".to_string()),
                source_type: "local".to_string(),
                source_ref: None,
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: existing_path.to_string_lossy().to_string(),
                content_hash: Some(content_hash::hash_directory(&existing_path).unwrap()),
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "up_to_date".to_string(),
                last_checked_at: Some(now),
                last_check_error: None,
            })
            .unwrap();

        let plugin = import_cursor_plugin(&store, &source().source_ref, &clone, source()).unwrap();
        assert_eq!(
            store.get_plugin_skill_ids(&plugin.id).unwrap(),
            vec!["existing-skill"]
        );
        assert_eq!(store.get_all_skills().unwrap().len(), 1);
        assert!(!skills_dir.join("example-skill-2").exists());
        assert_eq!(
            fs::read_to_string(existing_path.join("SKILL.md")).unwrap(),
            "---\nname: Example Skill\ndescription: Local improvements\n---\n"
        );

        let duplicate_path = skills_dir.join("example-skill-2");
        write_skill(&skills_dir, "example-skill-2");
        let duplicate_hash = content_hash::hash_directory(&duplicate_path).unwrap();
        store
            .insert_skill(&SkillRecord {
                id: "imported-duplicate".to_string(),
                name: "example-skill-2".to_string(),
                description: Some("Example".to_string()),
                source_type: "git".to_string(),
                source_ref: Some(source().source_ref.clone()),
                source_ref_resolved: Some(source().source_ref_resolved.clone()),
                source_subpath: Some("pstack/skills/example-skill".to_string()),
                source_branch: Some("main".to_string()),
                source_revision: Some("abc123".to_string()),
                remote_revision: Some("abc123".to_string()),
                central_path: duplicate_path.to_string_lossy().to_string(),
                content_hash: Some(duplicate_hash),
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "up_to_date".to_string(),
                last_checked_at: Some(now),
                last_check_error: None,
            })
            .unwrap();
        let duplicate_plugin = PluginRecord {
            id: "duplicate-plugin".to_string(),
            slug: "duplicate-plugin".to_string(),
            kind: "cursor".to_string(),
            name: "duplicate-plugin".to_string(),
            display_name: "Duplicate Plugin".to_string(),
            description: None,
            version: None,
            source_ref: Some(source().source_ref.clone()),
            source_ref_resolved: Some(source().source_ref_resolved.clone()),
            source_branch: Some("main".to_string()),
            source_revision: Some("abc123".to_string()),
            author: None,
            homepage: None,
            active: false,
            created_at: now,
            updated_at: now,
        };
        store.upsert_plugin(&duplicate_plugin).unwrap();
        store
            .replace_plugin_skills(&duplicate_plugin.id, &["imported-duplicate".to_string()])
            .unwrap();
        assert_eq!(
            reconcile_plugin_skill_members(&store, &duplicate_plugin.id).unwrap(),
            1
        );
        assert_eq!(
            store.get_plugin_skill_ids(&duplicate_plugin.id).unwrap(),
            vec!["existing-skill"]
        );
        assert!(store.get_skill_by_id("imported-duplicate").unwrap().is_none());
        assert!(!duplicate_path.exists());

        central_repo::set_test_base_dir_override(None);
        fs::remove_dir_all(clone).unwrap();
    }
    #[test]
    fn cursor_manifest_accepts_only_declared_skill_subtree() {
        let temp = tempdir().unwrap();
        write_manifest(temp.path(), "./skills");
        write_skill(temp.path(), "skills/one");
        write_skill(temp.path(), "outside");

        let preview = preview_cursor_plugin(temp.path(), source()).unwrap();
        assert_eq!(preview.skills.len(), 1);
        assert_eq!(preview.skills[0].relative_path, "one");
        assert!(preview.ignored_agents);
        assert!(!preview.ignored_rules);
    }

    #[test]
    fn cursor_manifest_rejects_parent_traversal() {
        let temp = tempdir().unwrap();
        write_manifest(temp.path(), "../outside");
        let error = preview_cursor_plugin(temp.path(), source()).unwrap_err();
        assert!(error.to_string().contains("must not traverse"));
    }

    #[test]
    fn cursor_manifest_rejects_empty_skill_tree() {
        let temp = tempdir().unwrap();
        write_manifest(temp.path(), "./skills");
        fs::create_dir_all(temp.path().join("skills")).unwrap();
        let error = preview_cursor_plugin(temp.path(), source()).unwrap_err();
        assert!(error.to_string().contains("no valid SKILL.md"));
    }

    #[test]
    fn dependency_closure_is_dependency_first_and_deterministic() {
        let graph = std::collections::BTreeMap::from([
            ("app".to_string(), vec!["shared".to_string(), "base".to_string()]),
            ("base".to_string(), vec![]),
            ("shared".to_string(), vec!["base".to_string()]),
        ]);
        assert_eq!(
            dependency_first_closure(&["app".to_string()], &graph).unwrap(),
            vec!["base", "shared", "app"]
        );
    }

    #[test]
    fn dependency_closure_rejects_cycles_and_missing_dependencies() {
        let cycle = std::collections::BTreeMap::from([
            ("one".to_string(), vec!["two".to_string()]),
            ("two".to_string(), vec!["one".to_string()]),
        ]);
        assert!(dependency_first_closure(&["one".to_string()], &cycle)
            .unwrap_err()
            .to_string()
            .contains("cycle"));
        let missing = std::collections::BTreeMap::from([("one".to_string(), vec!["two".to_string()])]);
        assert!(dependency_first_closure(&["one".to_string()], &missing)
            .unwrap_err()
            .to_string()
            .contains("missing plugin"));
    }
}
