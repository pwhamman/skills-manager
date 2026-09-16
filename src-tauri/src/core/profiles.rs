use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use super::{central_repo, skill_store::{ProfileRecord, SkillStore}};

pub const PROFILE_DOCUMENT_NAME: &str = "AGENTS.md";
const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileMetaFile {
    pub schema_version: u32,
    pub profile_id: String,
    pub name: String,
    pub folders: Vec<String>,
}

pub fn profiles_root() -> PathBuf {
    central_repo::skills_dir().join("profiles")
}

pub fn profile_root(profile_id: &str) -> Result<PathBuf> {
    validate_profile_id(profile_id)?;
    Ok(profiles_root().join(profile_id))
}

pub fn canonical_document_path(profile_id: &str) -> Result<PathBuf> {
    Ok(profile_root(profile_id)?.join(PROFILE_DOCUMENT_NAME))
}

pub fn folder_document_path(profile_id: &str, folder_name: &str) -> Result<PathBuf> {
    validate_folder_name(folder_name)?;
    Ok(profile_root(profile_id)?.join("folders").join(folder_name).join(PROFILE_DOCUMENT_NAME))
}

pub fn metadata_path(profile_id: &str) -> Result<PathBuf> {
    validate_profile_id(profile_id)?;
    Ok(central_repo::skills_dir()
        .join(".skills-manager")
        .join("profiles")
        .join(format!("{profile_id}.json")))
}

pub fn validate_profile_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id).map(|_| ()).map_err(|_| anyhow!("invalid profile ID"))
}

pub fn validate_folder_name(folder_name: &str) -> Result<()> {
    if folder_name.is_empty() || folder_name.contains(['/', '\\']) {
        bail!("folder name must be a single home-directory component");
    }
    let mut components = Path::new(folder_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) if folder_name != "." && folder_name != ".." => Ok(()),
        _ => bail!("folder name must be a single home-directory component"),
    }
}

pub fn home_dir() -> Result<PathBuf> {
    central_repo::home_base_dir()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("could not determine home directory"))
}

pub fn home_folder_target(folder_name: &str) -> Result<PathBuf> {
    validate_folder_name(folder_name)?;
    let path = home_dir()?.join(folder_name);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("cannot inspect home folder {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("home child is not a non-symlinked directory: {}", path.display());
    }
    Ok(path.join(PROFILE_DOCUMENT_NAME))
}

pub fn list_home_folders() -> Result<Vec<String>> {
    let mut folders = Vec::new();
    for entry in fs::read_dir(home_dir()?)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_folder_name(&name).is_ok() {
            folders.push(name);
        }
    }
    folders.sort_by_key(|name| name.to_lowercase());
    Ok(folders)
}

pub fn create_profile(store: &SkillStore, name: &str) -> Result<ProfileRecord> {
    let name = validate_name(name)?;
    let profile = ProfileRecord {
        id: uuid::Uuid::now_v7().to_string(),
        name,
        created_at: chrono::Utc::now().timestamp_millis(),
        updated_at: chrono::Utc::now().timestamp_millis(),
    };
    let is_first = store.get_all_profiles()?.is_empty();
    let seed = if is_first { first_profile_seed()? } else { String::new() };
    fs::create_dir_all(profile_root(&profile.id)?)?;
    write_text_atomic(&canonical_document_path(&profile.id)?, &seed)?;
    if let Err(error) = store.insert_profile(&profile) {
        let _ = fs::remove_dir_all(profile_root(&profile.id)?);
        return Err(error);
    }
    Ok(profile)
}

pub fn rename_profile(store: &SkillStore, id: &str, name: &str) -> Result<()> {
    validate_profile_id(id)?;
    store.update_profile_name(id, &validate_name(name)?)
}

pub fn delete_profile_files(store: &SkillStore, id: &str) -> Result<()> {
    validate_profile_id(id)?;
    let root = profile_root(id)?;
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    let metadata = metadata_path(id)?;
    if metadata.exists() {
        fs::remove_file(metadata)?;
    }
    store.delete_profile(id)
}

pub fn read_document(profile_id: &str, folder_name: Option<&str>) -> Result<String> {
    let path = match folder_name {
        Some(folder) => folder_document_path(profile_id, folder)?,
        None => canonical_document_path(profile_id)?,
    };
    read_regular_utf8(&path)
}

pub fn save_document(profile_id: &str, folder_name: Option<&str>, content: &str) -> Result<()> {
    let path = match folder_name {
        Some(folder) => folder_document_path(profile_id, folder)?,
        None => canonical_document_path(profile_id)?,
    };
    write_text_atomic(&path, content)
}

pub fn add_folder(store: &SkillStore, profile_id: &str, folder_name: &str) -> Result<()> {
    validate_profile_id(profile_id)?;
    validate_folder_name(folder_name)?;
    let target = home_folder_target(folder_name)?;
    let source = folder_document_path(profile_id, folder_name)?;
    if target.exists() {
        write_text_atomic(&source, &read_regular_utf8(&target)?)?;
    } else {
        write_text_atomic(&source, "")?;
    }
    let mut folders = store.get_profile_folders(profile_id)?;
    if !folders.iter().any(|folder| folder == folder_name) {
        folders.push(folder_name.to_owned());
        store.replace_profile_folders(profile_id, &folders)?;
    }
    Ok(())
}

pub fn remove_folder(store: &SkillStore, profile_id: &str, folder_name: &str) -> Result<()> {
    validate_profile_id(profile_id)?;
    validate_folder_name(folder_name)?;
    let mut folders = store.get_profile_folders(profile_id)?;
    folders.retain(|folder| folder != folder_name);
    store.replace_profile_folders(profile_id, &folders)?;
    let source = folder_document_path(profile_id, folder_name)?;
    if source.exists() {
        fs::remove_file(source)?;
    }
    Ok(())
}

pub fn write_metadata(store: &SkillStore, profile: &ProfileRecord) -> Result<()> {
    let mut folders = store.get_profile_folders(&profile.id)?;
    folders.sort();
    let metadata = ProfileMetaFile {
        schema_version: PROFILE_SCHEMA_VERSION,
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        folders,
    };
    write_json_atomic(&metadata_path(&profile.id)?, &metadata)
}

pub fn read_metadata_files() -> Result<Vec<ProfileMetaFile>> {
    let root = central_repo::skills_dir().join(".skills-manager").join("profiles");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut profiles = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)?;
        let metadata: ProfileMetaFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid profile metadata {}", path.display()))?;
        validate_metadata(&metadata, &path)?;
        profiles.push(metadata);
    }
    profiles.sort_by_key(|profile| profile.name.to_lowercase());
    Ok(profiles)
}

pub fn validate_metadata(metadata: &ProfileMetaFile, metadata_file: &Path) -> Result<()> {
    if metadata.schema_version != PROFILE_SCHEMA_VERSION {
        bail!("unsupported profile metadata schema version");
    }
    validate_profile_id(&metadata.profile_id)?;
    validate_name(&metadata.name)?;
    if metadata_file.file_stem().and_then(|value| value.to_str()) != Some(metadata.profile_id.as_str()) {
        bail!("profile metadata filename does not match profile ID");
    }
    let mut unique = HashSet::new();
    for folder in &metadata.folders {
        validate_folder_name(folder)?;
        if !unique.insert(folder) {
            bail!("profile metadata contains duplicate folder name");
        }
    }
    read_regular_utf8(&canonical_document_path(&metadata.profile_id)?)?;
    for folder in &metadata.folders {
        read_regular_utf8(&folder_document_path(&metadata.profile_id, folder)?)?;
    }
    Ok(())
}

pub fn activate_profile(store: &SkillStore, profile_id: &str) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, profile_id);
        bail!("profile activation is supported only on macOS");
    }
    #[cfg(target_os = "macos")]
    {
        validate_profile_id(profile_id)?;
        let profile = store.get_profile(profile_id)?.ok_or_else(|| anyhow!("profile not found"))?;
        let canonical = canonical_document_path(&profile.id)?;
        read_regular_utf8(&canonical)?;
        let folders = store.get_profile_folders(&profile.id)?;
        let mut writes = Vec::new();
        for folder in folders {
            let target = home_folder_target(&folder)?;
            let content = read_regular_utf8(&folder_document_path(&profile.id, &folder)?)?;
            let target_key = target.to_string_lossy().to_string();
            if target.exists() {
                let existing = hash_file(&target)?;
                let recorded = store.get_profile_deployment(&target_key)?;
                if recorded.as_ref().map(|deployment| &deployment.1) != Some(&existing) {
                    bail!("profile deployment ownership conflict: {}", target.display());
                }
            }
            writes.push((target, content));
        }
        for (target, content) in &writes {
            write_text_atomic(target, content)?;
            store.set_profile_deployment(&target.to_string_lossy(), &profile.id, &hash_file(target)?)?;
        }
        for target in global_targets()? {
            replace_with_symlink(&target, &canonical)?;
        }
        store.set_active_profile_id(Some(&profile.id))?;
        Ok(())
    }
}

pub fn clear_active_profile_if_missing(store: &SkillStore) -> Result<()> {
    let Some(active) = store.active_profile_id()? else { return Ok(()); };
    if store.get_profile(&active)?.is_none() {
        clear_active_profile(store)?;
    }
    Ok(())
}

pub fn clear_active_profile(store: &SkillStore) -> Result<()> {
    cleanup_managed_global_symlinks()?;
    store.set_active_profile_id(None)
}

pub fn global_targets() -> Result<Vec<PathBuf>> {
    let home = home_dir()?;
    Ok(vec![
        home.join(".omp/agent/AGENTS.md"),
        home.join(".claude/CLAUDE.md"),
        home.join(".codex/AGENTS.md"),
        home.join(".pi/agent/AGENTS.md"),
    ])
}

fn cleanup_managed_global_symlinks() -> Result<()> {
    let root = profiles_root().canonicalize().ok();
    for target in global_targets()? {
        if !target.is_symlink() {
            continue;
        }
        let resolved = target.canonicalize().ok();
        if root.as_ref().zip(resolved.as_ref()).is_some_and(|(root, path)| path.starts_with(root)) {
            fs::remove_file(target)?;
        }
    }
    Ok(())
}

fn first_profile_seed() -> Result<String> {
    for target in global_targets()?.into_iter().filter(|path| path.file_name() == Some(std::ffi::OsStr::new(PROFILE_DOCUMENT_NAME))) {
        if target.is_symlink() && target.canonicalize().ok().is_some_and(|path| path.starts_with(profiles_root())) {
            continue;
        }
        if let Ok(content) = read_regular_utf8(&target) {
            return Ok(content);
        }
    }
    Ok(String::new())
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        bail!("profile name must be between 1 and 120 characters");
    }
    Ok(name.to_owned())
}

fn read_regular_utf8(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("profile document missing: {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("profile document must be a regular file: {}", path.display());
    }
    fs::read_to_string(path).with_context(|| format!("profile document is not UTF-8: {}", path.display()))
}

fn write_text_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("document path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp.{}", PROFILE_DOCUMENT_NAME, uuid::Uuid::now_v7()));
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn write_json_atomic(path: &Path, value: &ProfileMetaFile) -> Result<()> {
    let mut content = serde_json::to_string_pretty(value)?;
    content.push('\n');
    write_text_atomic(path, &content)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path)?);
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(target_os = "macos")]
fn replace_with_symlink(target: &Path, source: &Path) -> Result<()> {
    let parent = target.parent().ok_or_else(|| anyhow!("global target has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.link.{}", target.file_name().and_then(|name| name.to_str()).unwrap_or("AGENTS.md"), uuid::Uuid::now_v7()));
    std::os::unix::fs::symlink(source, &temporary)?;
    fs::rename(temporary, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::central_repo;
    use crate::core::skill_store::SkillStore;
    use tempfile::tempdir;

    #[test]
    #[cfg(target_os = "macos")]
    fn first_profile_seeds_and_activates_global_and_folder_documents() {
        let _guard = central_repo::test_base_dir_lock();
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let library = temp.path().join("library");
        fs::create_dir_all(home.join(".omp/agent")).unwrap();
        fs::write(home.join(".omp/agent/AGENTS.md"), "seeded global").unwrap();
        fs::create_dir_all(home.join("repo")).unwrap();

        central_repo::set_test_home_dir_override(Some(home.clone()));
        central_repo::set_test_base_dir_override(Some(library));
        let store = SkillStore::new(&temp.path().join("profiles.sqlite")).unwrap();

        let work = create_profile(&store, "Work").unwrap();
        assert_eq!(read_document(&work.id, None).unwrap(), "seeded global");
        add_folder(&store, &work.id, "repo").unwrap();
        save_document(&work.id, Some("repo"), "repo work").unwrap();
        activate_profile(&store, &work.id).unwrap();

        assert_eq!(fs::read_to_string(home.join("repo/AGENTS.md")).unwrap(), "repo work");
        for target in global_targets().unwrap() {
            assert!(target.is_symlink());
            assert_eq!(
                target.canonicalize().unwrap(),
                canonical_document_path(&work.id).unwrap().canonicalize().unwrap()
            );
        }

        fs::write(home.join("repo/AGENTS.md"), "external edit").unwrap();
        let personal = create_profile(&store, "Personal").unwrap();
        add_folder(&store, &personal.id, "repo").unwrap();
        let error = activate_profile(&store, &personal.id).unwrap_err();
        assert!(error.to_string().contains("ownership conflict"));
        assert_eq!(store.active_profile_id().unwrap(), Some(work.id));

        central_repo::set_test_base_dir_override(None);
        central_repo::set_test_home_dir_override(None);
    }

    #[test]
    fn profile_metadata_reindexes_with_folder_membership() {
        let _guard = central_repo::test_base_dir_lock();
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let library = temp.path().join("library");
        fs::create_dir_all(home.join("repo")).unwrap();
        central_repo::set_test_home_dir_override(Some(home));
        central_repo::set_test_base_dir_override(Some(library));

        let source_store = SkillStore::new(&temp.path().join("source.sqlite")).unwrap();
        let profile = create_profile(&source_store, "Work").unwrap();
        add_folder(&source_store, &profile.id, "repo").unwrap();
        save_document(&profile.id, Some("repo"), "repo work").unwrap();
        super::super::sync_metadata::write_all_from_db(&source_store).unwrap();

        let restored_store = SkillStore::new(&temp.path().join("restored.sqlite")).unwrap();
        super::super::sync_metadata::reindex_from_metadata(&restored_store).unwrap();
        let restored = restored_store.get_profile(&profile.id).unwrap().unwrap();
        assert_eq!(restored.name, "Work");
        assert_eq!(restored_store.get_profile_folders(&profile.id).unwrap(), vec!["repo".to_string()]);
        assert_eq!(read_document(&profile.id, Some("repo")).unwrap(), "repo work");

        central_repo::set_test_base_dir_override(None);
        central_repo::set_test_home_dir_override(None);
    }
}
