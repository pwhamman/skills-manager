use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::core::{
    error::AppError,
    git_fetcher,
    install_cancel::InstallCancelRegistry,
    plugins::{self, CursorPluginPreview, PluginSource},
    skill_store::{PluginRecord, SkillStore},
    sync_metadata,
};


struct CancelRegistrationGuard {
    registry: Arc<InstallCancelRegistry>,
    key: String,
}

impl CancelRegistrationGuard {
    fn new(registry: Arc<InstallCancelRegistry>, key: String) -> Self {
        Self { registry, key }
    }
}

impl Drop for CancelRegistrationGuard {
    fn drop(&mut self) {
        self.registry.remove(&self.key);
    }
}
#[derive(Debug, Serialize)]
pub struct PluginDto {
    pub id: String,
    pub slug: String,
    pub kind: String,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub source_ref: Option<String>,
    pub source_ref_resolved: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub active: bool,
    pub skill_ids: Vec<String>,
    pub dependency_ids: Vec<String>,
    pub setup_state: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct PluginPreviewDto {
    pub temp_dir: String,
    pub plugin: CursorPluginPreview,
}

fn plugin_dto(store: &SkillStore, plugin: PluginRecord) -> Result<PluginDto, AppError> {
    let setup_state = store
        .get_plugin_setup_state(&plugin.id)
        .map_err(AppError::db)
        .and_then(|state| serde_json::from_str(&state).map_err(AppError::db))?;
    Ok(PluginDto {
        skill_ids: store.get_plugin_skill_ids(&plugin.id).map_err(AppError::db)?,
        dependency_ids: store.get_plugin_dependency_ids(&plugin.id).map_err(AppError::db)?,
        id: plugin.id,
        slug: plugin.slug,
        kind: plugin.kind,
        name: plugin.name,
        display_name: plugin.display_name,
        description: plugin.description,
        version: plugin.version,
        source_ref: plugin.source_ref,
        source_ref_resolved: plugin.source_ref_resolved,
        source_branch: plugin.source_branch,
        source_revision: plugin.source_revision,
        author: plugin.author,
        homepage: plugin.homepage,
        active: plugin.active,
        setup_state,
    })
}

#[tauri::command]
pub async fn get_plugins(store: State<'_, Arc<SkillStore>>) -> Result<Vec<PluginDto>, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .get_all_plugins()
            .map_err(AppError::db)?
            .into_iter()
            .map(|plugin| plugin_dto(&store, plugin))
            .collect()
    })
    .await?
}

#[tauri::command]
pub async fn preview_plugin_install(
    repo_url: String,
    store: State<'_, Arc<SkillStore>>,
    cancel_registry: State<'_, Arc<InstallCancelRegistry>>,
) -> Result<PluginPreviewDto, AppError> {
    let store = store.inner().clone();
    let registry = cancel_registry.inner().clone();
    let key = format!("plugin:{repo_url}");
    let cancel = registry.register(&key);
    let _guard = CancelRegistrationGuard::new(registry, key);
    tauri::async_runtime::spawn_blocking(move || {
        git_fetcher::validate_git_url(&repo_url).map_err(AppError::git)?;
        let parsed = git_fetcher::parse_git_source_resolved(&repo_url, store.proxy_url().as_deref());
        let temp_dir = git_fetcher::clone_repo_ref_scoped(
            &parsed.clone_url,
            parsed.branch.as_deref(),
            parsed.subpath.as_deref(),
            Some(&cancel),
            store.proxy_url().as_deref(),
            None,
        )
        .map_err(AppError::classify_git_error)?;
        let source = PluginSource {
            source_ref: repo_url,
            source_ref_resolved: parsed.clone_url,
            branch: parsed.branch.clone(),
            revision: git_fetcher::get_head_revision(&temp_dir).ok(),
        };
        let root = if let Some(subpath) = parsed.subpath.as_deref() {
            let candidate = temp_dir.join(subpath);
            if candidate.join(".cursor-plugin/plugin.json").is_file() { candidate } else { temp_dir.clone() }
        } else {
            temp_dir.clone()
        };
        let result = plugins::preview_cursor_plugin(&root, source);
        match result {
            Ok(plugin) => Ok(PluginPreviewDto {
                temp_dir: temp_dir.to_string_lossy().to_string(),
                plugin,
            }),
            Err(error) => {
                git_fetcher::cleanup_temp(&temp_dir);
                Err(AppError::invalid_input(error.to_string()))
            }
        }
    })
    .await?
}

#[tauri::command]
pub async fn confirm_plugin_install(
    repo_url: String,
    temp_dir: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<PluginDto, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let temp_path = PathBuf::from(&temp_dir);
        let parsed = git_fetcher::parse_git_source_resolved(&repo_url, store.proxy_url().as_deref());
        let source = PluginSource {
            source_ref: repo_url.clone(),
            source_ref_resolved: parsed.clone_url,
            branch: parsed.branch,
            revision: git_fetcher::get_head_revision(&temp_path).ok(),
        };
        let result = sync_metadata::with_repo_lock("confirm plugin install", || {
            plugins::import_cursor_plugin(&store, &repo_url, &temp_path, source)
        });
        git_fetcher::cleanup_temp(&temp_path);
        plugin_dto(&store, result.map_err(AppError::io)?)
    })
    .await?
}

#[tauri::command]
pub async fn cancel_plugin_preview(temp_dir: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(temp_dir);
        if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(git_fetcher::CLONE_TEMP_PREFIX)) {
            git_fetcher::cleanup_temp(&path);
        }
        Ok(())
    })
    .await?
}

#[tauri::command]
pub async fn create_manual_plugin(
    display_name: String,
    description: Option<String>,
    skill_ids: Vec<String>,
    dependency_ids: Vec<String>,
    setup_state: serde_json::Value,
    store: State<'_, Arc<SkillStore>>,
) -> Result<PluginDto, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if display_name.trim().is_empty() || skill_ids.is_empty() || !setup_state.is_object() {
            return Err(AppError::invalid_input("manual plugin needs a name, at least one skill, and an object setup state"));
        }
        if skill_ids.iter().collect::<HashSet<_>>().len() != skill_ids.len()
            || dependency_ids.iter().collect::<HashSet<_>>().len() != dependency_ids.len()
        {
            return Err(AppError::invalid_input("plugin members and dependencies must be unique"));
        }
        for skill_id in &skill_ids {
            if store.get_skill_by_id(skill_id).map_err(AppError::db)?.is_none() {
                return Err(AppError::not_found("plugin references an unknown skill"));
            }
        }
        let now = chrono::Utc::now().timestamp_millis();
        let plugin = PluginRecord {
            id: uuid::Uuid::new_v4().to_string(),
            slug: manual_slug(&store, &display_name).map_err(AppError::db)?,
            kind: "manual".to_string(),
            name: display_name.clone(),
            display_name,
            description,
            version: None,
            source_ref: None,
            source_ref_resolved: None,
            source_branch: None,
            source_revision: None,
            author: None,

            homepage: None,
            active: false,
            created_at: now,
            updated_at: now,
        };
        sync_metadata::with_repo_lock("create manual plugin", || {
            validate_dependencies(&store, &plugin.id, &dependency_ids)?;
            store.upsert_plugin(&plugin)?;
            store.replace_plugin_skills(&plugin.id, &skill_ids)?;
            store.replace_plugin_dependencies(&plugin.id, &dependency_ids)?;
            store.set_plugin_setup_state(&plugin.id, &setup_state.to_string())?;
            sync_metadata::write_all_from_db_unlocked(&store)
        })
        .map_err(AppError::io)?;
        plugin_dto(&store, plugin)
    })
    .await?
}

#[tauri::command]
pub async fn update_manual_plugin(
    plugin_id: String,
    display_name: String,
    description: Option<String>,
    skill_ids: Vec<String>,
    dependency_ids: Vec<String>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<PluginDto, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if display_name.trim().is_empty() || skill_ids.is_empty() {
            return Err(AppError::invalid_input("manual plugin needs a name and at least one skill"));
        }
        if skill_ids.iter().collect::<HashSet<_>>().len() != skill_ids.len()
            || dependency_ids.iter().collect::<HashSet<_>>().len() != dependency_ids.len()
        {
            return Err(AppError::invalid_input("plugin members and dependencies must be unique"));
        }
        let mut plugin = store
            .get_plugin_by_id(&plugin_id)
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("plugin not found"))?;
        if plugin.kind != "manual" {
            return Err(AppError::invalid_input("only manual plugins can be edited"));
        }
        for skill_id in &skill_ids {
            if store.get_skill_by_id(skill_id).map_err(AppError::db)?.is_none() {
                return Err(AppError::not_found("plugin references an unknown skill"));
            }
        }
        plugin.name = display_name.clone();
        plugin.display_name = display_name;
        plugin.description = description;
        plugin.updated_at = chrono::Utc::now().timestamp_millis();
        sync_metadata::with_repo_lock("update manual plugin", || {
            validate_dependencies(&store, &plugin.id, &dependency_ids)?;
            store.upsert_plugin(&plugin)?;
            store.replace_plugin_skills(&plugin.id, &skill_ids)?;
            store.replace_plugin_dependencies(&plugin.id, &dependency_ids)?;
            sync_metadata::write_all_from_db_unlocked(&store)
        })
        .map_err(AppError::io)?;
        plugin_dto(&store, plugin)
    })
    .await?
}

#[tauri::command]
pub async fn activate_plugin(
    plugin_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sync_metadata::with_repo_lock("activate plugin", || plugins::activate_plugin(&store, &plugin_id))
            .map_err(AppError::io)
    })
    .await?
}

#[tauri::command]
pub async fn deactivate_plugin(
    plugin_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sync_metadata::with_repo_lock("deactivate plugin", || plugins::deactivate_plugin(&store, &plugin_id))
            .map_err(AppError::io)
    })
    .await?
}

fn manual_slug(store: &SkillStore, name: &str) -> anyhow::Result<String> {
    let base = name
        .to_ascii_lowercase()
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if base.is_empty() { anyhow::bail!("plugin name cannot produce a slug"); }
    let slugs: HashSet<String> = store.get_all_plugins()?.into_iter().map(|plugin| plugin.slug).collect();
    if !slugs.contains(&base) { return Ok(base); }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !slugs.contains(&candidate) { return Ok(candidate); }
    }
    unreachable!()
}

fn validate_dependencies(store: &SkillStore, pending_id: &str, dependencies: &[String]) -> anyhow::Result<()> {
    let mut graph: BTreeMap<String, Vec<String>> = store
        .get_all_plugins()?
        .into_iter()
        .map(|plugin| Ok((plugin.id.clone(), store.get_plugin_dependency_ids(&plugin.id)?)))
        .collect::<anyhow::Result<_>>()?;
    graph.insert(pending_id.to_string(), dependencies.to_vec());
    let roots: Vec<String> = graph.keys().cloned().collect();
    plugins::dependency_first_closure(&roots, &graph)?;
    Ok(())
}
