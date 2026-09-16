use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};

use crate::core::{
    error::{AppError, TargetConflictDetail},
    profiles,
    skill_store::{ProfileRecord, SkillStore},
    sync_metadata,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProfileDto {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub folders: Vec<String>,
    pub active: bool,
}

fn profile_dto(store: &SkillStore, profile: ProfileRecord) -> Result<ProfileDto, AppError> {
    let active = store.active_profile_id().map_err(AppError::db)?;
    Ok(ProfileDto {
        folders: store.get_profile_folders(&profile.id).map_err(AppError::db)?,
        active: active.as_deref() == Some(&profile.id),
        id: profile.id,
        name: profile.name,
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    })
}


fn emit_profile_change(app: &tauri::AppHandle) {
    let _ = app.emit("app-files-changed", ());
    crate::refresh_tray_menu(app).ok();
}

#[tauri::command]
pub async fn get_profiles(store: State<'_, Arc<SkillStore>>) -> Result<Vec<ProfileDto>, AppError> {
    store
        .get_all_profiles()
        .map_err(AppError::db)?
        .into_iter()
        .map(|profile| profile_dto(&store, profile))
        .collect()
}

#[tauri::command]
pub async fn get_active_profile(store: State<'_, Arc<SkillStore>>) -> Result<Option<ProfileDto>, AppError> {
    let Some(id) = store.active_profile_id().map_err(AppError::db)? else { return Ok(None); };
    store
        .get_profile(&id)
        .map_err(AppError::db)?
        .map(|profile| profile_dto(&store, profile))
        .transpose()
}

#[tauri::command]
pub async fn create_profile(
    app: tauri::AppHandle,
    name: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<ProfileDto, AppError> {
    let profile = sync_metadata::with_repo_lock("create profile", || {
        let profile = profiles::create_profile(&store, &name)?;
        sync_metadata::write_all_from_db_unlocked(&store)?;
        Ok(profile)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    profile_dto(&store, profile)
}

#[tauri::command]
pub async fn rename_profile(
    app: tauri::AppHandle,
    id: String,
    name: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    sync_metadata::with_repo_lock("rename profile", || {
        profiles::rename_profile(&store, &id, &name)?;
        sync_metadata::write_all_from_db_unlocked(&store)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_profile(
    app: tauri::AppHandle,
    id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    sync_metadata::with_repo_lock("delete profile", || {
        if store.active_profile_id()?.as_deref() == Some(&id) {
            profiles::clear_active_profile(&store)?;
        }
        profiles::delete_profile_files(&store, &id)?;
        sync_metadata::write_all_from_db_unlocked(&store)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_profile_document(
    id: String,
    folder_name: Option<String>,
) -> Result<String, AppError> {
    profiles::read_document(&id, folder_name.as_deref()).map_err(AppError::io)
}

#[tauri::command]
pub async fn save_profile_document(
    app: tauri::AppHandle,
    id: String,
    folder_name: Option<String>,
    content: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    sync_metadata::with_repo_lock("save profile document", || {
        profiles::save_document(&id, folder_name.as_deref(), &content)?;
        sync_metadata::write_all_from_db_unlocked(&store)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_profile_home_folders() -> Result<Vec<String>, AppError> {
    profiles::list_home_folders().map_err(AppError::io)
}

#[tauri::command]
pub async fn add_profile_folder(
    app: tauri::AppHandle,
    id: String,
    folder_name: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    sync_metadata::with_repo_lock("add profile folder", || {
        profiles::add_folder(&store, &id, &folder_name)?;
        sync_metadata::write_all_from_db_unlocked(&store)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    Ok(())
}

#[tauri::command]
pub async fn remove_profile_folder(
    app: tauri::AppHandle,
    id: String,
    folder_name: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    sync_metadata::with_repo_lock("remove profile folder", || {
        profiles::remove_folder(&store, &id, &folder_name)?;
        sync_metadata::write_all_from_db_unlocked(&store)
    })
    .map_err(AppError::io)?;
    emit_profile_change(&app);
    Ok(())
}

#[tauri::command]
pub async fn activate_profile(
    app: tauri::AppHandle,
    id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    if let Err(error) = profiles::activate_profile(&store, &id) {
        let message = error.to_string();
        if let Some(path) = message.strip_prefix("profile deployment ownership conflict: ") {
            return Err(AppError::target_conflict(
                "Profile activation would overwrite an externally modified document",
                vec![TargetConflictDetail { path: path.to_owned(), reason: "modified after profile deployment".to_string() }],
            ));
        }
        return Err(AppError::io(error));
    }
    emit_profile_change(&app);
    Ok(())
}
