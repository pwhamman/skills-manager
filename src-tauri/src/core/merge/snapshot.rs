//! Read one commit's tree into the logical objects the merge operates on
//! (design §1/§2): skills (metadata + content-tree fingerprint), scenarios,
//! memberships, residual files, and the schema/protocol markers.

use anyhow::{Context, Result, bail};
use git2::{ObjectType, Oid, Repository, Tree};
use std::collections::{BTreeMap, BTreeSet};

use super::protocol::ProtocolFile;
use crate::core::{
    git_backup::BackupExclusion,
    profiles::{self, ProfileMetaFile},
    sync_metadata::SkillMetaFile,
};

pub const METADATA_DIR: &str = ".skills-manager";
/// Marker files that make a directory a valid skill dir (mirrors
/// `skill_metadata::SKILL_DIR_MARKERS` for tree-level checks).
pub const SKILL_DIR_MARKERS: &[&str] = &["SKILL.md", "skill.md"];
/// Maximum depth (in path components) of a skill content directory.
pub const MAX_SKILL_DEPTH: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileEntry {
    pub oid: Oid,
    pub mode: i32,
}

#[derive(Debug, Clone)]
pub struct SkillObj {
    pub meta: SkillMetaFile,
    /// The raw `skills/{id}.json` blob this metadata was read from, so the
    /// planner can tell whether a rebuilt blob actually differs.
    pub meta_entry: FileEntry,
    /// Tree OID of the content directory at `meta.path`; `None` when the
    /// path is missing from the tree (a broken pairing the validator rejects
    /// in any *merged* tree, but which inputs may exhibit).
    pub content: Option<Oid>,
}

/// Component-level equality (§2): `content` / `path` / `attrs` are compared
/// independently.
pub fn attrs_eq(a: &SkillMetaFile, b: &SkillMetaFile) -> bool {
    a.enabled == b.enabled && a.tags == b.tags && a.source == b.source
}

pub fn skill_identical(a: &SkillObj, b: &SkillObj) -> bool {
    a.content == b.content && a.meta.path == b.meta.path && attrs_eq(&a.meta, &b.meta)
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub skills: BTreeMap<String, SkillObj>,
    /// scenario_id → scenarios/{id}.json blob
    pub scenarios: BTreeMap<String, FileEntry>,
    /// (scenario_id, skill_id) → scenario-skills/{sid}/{skid}.json blob
    pub memberships: BTreeMap<(String, String), FileEntry>,
    /// profile_id → .skills-manager/profiles/{id}.json blob.
    pub profiles: BTreeMap<String, FileEntry>,
    /// Portable canonical and home-folder profile documents, keyed by full repo path.
    pub profile_documents: BTreeMap<String, FileEntry>,
    /// skill_id → .skills-manager/backup-exclusions/{id}.json blob.
    pub backup_exclusions: BTreeMap<String, FileEntry>,
    /// Repo-relative path → blob, for every file outside claimed content
    /// dirs and outside the known metadata files (`.gitignore`, stray user
    /// files, unknown future `.skills-manager` entries).
    pub residual: BTreeMap<String, FileEntry>,
    pub schema: Option<(FileEntry, u64)>,
    pub protocol: Option<(FileEntry, ProtocolFile)>,
}

pub fn read_snapshot(repo: &Repository, tree: &Tree) -> Result<Snapshot> {
    let mut snap = Snapshot::default();

    // ── metadata namespace ──
    if let Some(meta_entry) = tree.get_name(METADATA_DIR) {
        let meta_tree = repo
            .find_tree(meta_entry.id())
            .context(".skills-manager is not a directory")?;
        for entry in meta_tree.iter() {
            let name = entry.name().unwrap_or_default().to_string();
            match (name.as_str(), entry.kind()) {
                ("skills", Some(ObjectType::Tree)) => {
                    let skills_tree = repo.find_tree(entry.id())?;
                    for e in skills_tree.iter() {
                        let file = e.name().unwrap_or_default().to_string();
                        let Some(stem) = file.strip_suffix(".json") else {
                            record_residual(&mut snap, format!("{METADATA_DIR}/skills/{file}"), &e);
                            continue;
                        };
                        let blob = repo.find_blob(e.id()).with_context(|| {
                            format!("skill metadata {file} is not a blob")
                        })?;
                        let meta: SkillMetaFile = serde_json::from_slice(blob.content())
                            .with_context(|| format!("invalid skill metadata {file}"))?;
                        if meta.skill_id != stem {
                            bail!(
                                "skill metadata {file}: skill_id {} does not match file name",
                                meta.skill_id
                            );
                        }
                        let content = tree
                            .get_path(std::path::Path::new(&meta.path))
                            .ok()
                            .filter(|e| e.kind() == Some(ObjectType::Tree))
                            .map(|e| e.id());
                        snap.skills.insert(
                            stem.to_string(),
                            SkillObj {
                                meta,
                                meta_entry: FileEntry { oid: e.id(), mode: e.filemode() },
                                content,
                            },
                        );
                    }
                }
                ("scenarios", Some(ObjectType::Tree)) => {
                    let t = repo.find_tree(entry.id())?;
                    for e in t.iter() {
                        let file = e.name().unwrap_or_default().to_string();
                        match file.strip_suffix(".json") {
                            Some(stem) if e.kind() == Some(ObjectType::Blob) => {
                                snap.scenarios.insert(
                                    stem.to_string(),
                                    FileEntry { oid: e.id(), mode: e.filemode() },
                                );
                            }
                            _ => record_residual(
                                &mut snap,
                                format!("{METADATA_DIR}/scenarios/{file}"),
                                &e,
                            ),
                        }
                    }
                }
                ("scenario-skills", Some(ObjectType::Tree)) => {
                    let t = repo.find_tree(entry.id())?;
                    for dir in t.iter() {
                        let sid = dir.name().unwrap_or_default().to_string();
                        if dir.kind() != Some(ObjectType::Tree) {
                            record_residual(
                                &mut snap,
                                format!("{METADATA_DIR}/scenario-skills/{sid}"),
                                &dir,
                            );
                            continue;
                        }
                        let dt = repo.find_tree(dir.id())?;
                        for e in dt.iter() {
                            let file = e.name().unwrap_or_default().to_string();
                            match file.strip_suffix(".json") {
                                Some(stem) if e.kind() == Some(ObjectType::Blob) => {
                                    snap.memberships.insert(
                                        (sid.clone(), stem.to_string()),
                                        FileEntry { oid: e.id(), mode: e.filemode() },
                                    );
                                }
                                _ => record_residual(
                                    &mut snap,
                                    format!("{METADATA_DIR}/scenario-skills/{sid}/{file}"),
                                    &e,
                                ),
                            }
                        }
                    }
                }
                ("profiles", Some(ObjectType::Tree)) => {
                    let t = repo.find_tree(entry.id())?;
                    for e in t.iter() {
                        let file = e.name().unwrap_or_default().to_string();
                        let Some(id) = file.strip_suffix(".json") else {
                            record_residual(&mut snap, format!("{METADATA_DIR}/profiles/{file}"), &e);
                            continue;
                        };
                        if e.kind() != Some(ObjectType::Blob) {
                            record_residual(&mut snap, format!("{METADATA_DIR}/profiles/{file}"), &e);
                            continue;
                        }
                        let blob = repo.find_blob(e.id())?;
                        let meta: ProfileMetaFile = serde_json::from_slice(blob.content())
                            .with_context(|| format!("invalid profile metadata {file}"))?;
                        if meta.profile_id != id {
                            bail!("profile metadata {file}: profile_id does not match file name");
                        }
                        profiles::validate_profile_id(id)?;
                        for folder in &meta.folders {
                            profiles::validate_folder_name(folder)?;
                        }
                        let mut documents = vec![format!("profiles/{id}/AGENTS.md")];
                        documents.extend(meta.folders.iter().map(|folder| {
                            format!("profiles/{id}/folders/{folder}/AGENTS.md")
                        }));
                        for document in documents {
                            let document_entry = tree
                                .get_path(std::path::Path::new(&document))
                                .with_context(|| format!("profile document missing: {document}"))?;
                            if document_entry.kind() != Some(ObjectType::Blob) {
                                bail!("profile document is not a blob: {document}");
                            }
                            snap.profile_documents.insert(
                                document,
                                FileEntry { oid: document_entry.id(), mode: document_entry.filemode() },
                            );
                        }
                        snap.profiles.insert(
                            id.to_string(),
                            FileEntry { oid: e.id(), mode: e.filemode() },
                        );
                    }
                }
                ("backup-exclusions", Some(ObjectType::Tree)) => {
                    let t = repo.find_tree(entry.id())?;
                    for e in t.iter() {
                        let file = e.name().unwrap_or_default().to_string();
                        let Some(skill_id) = file.strip_suffix(".json") else {
                            record_residual(
                                &mut snap,
                                format!("{METADATA_DIR}/backup-exclusions/{file}"),
                                &e,
                            );
                            continue;
                        };
                        if e.kind() != Some(ObjectType::Blob) {
                            record_residual(
                                &mut snap,
                                format!("{METADATA_DIR}/backup-exclusions/{file}"),
                                &e,
                            );
                            continue;
                        }
                        let blob = repo.find_blob(e.id())?;
                        let exclusion: BackupExclusion = serde_json::from_slice(blob.content())
                            .with_context(|| format!("invalid backup exclusion {file}"))?;
                        if exclusion.skill_id != skill_id {
                            bail!(
                                "backup exclusion {file}: skill_id {} does not match file name",
                                exclusion.skill_id
                            );
                        }
                        snap.backup_exclusions.insert(
                            skill_id.to_string(),
                            FileEntry { oid: e.id(), mode: e.filemode() },
                        );
                    }
                }
                ("schema.json", Some(ObjectType::Blob)) => {
                    let blob = repo.find_blob(entry.id())?;
                    let version = serde_json::from_slice::<serde_json::Value>(blob.content())
                        .ok()
                        .and_then(|v| v.get("schema_version").and_then(|n| n.as_u64()))
                        .unwrap_or(0);
                    snap.schema = Some((
                        FileEntry { oid: entry.id(), mode: entry.filemode() },
                        version,
                    ));
                }
                ("protocol.json", Some(ObjectType::Blob)) => {
                    let blob = repo.find_blob(entry.id())?;
                    let parsed: ProtocolFile = serde_json::from_slice(blob.content())
                        .context("invalid protocol.json")?;
                    snap.protocol = Some((
                        FileEntry { oid: entry.id(), mode: entry.filemode() },
                        parsed,
                    ));
                }
                _ => {
                    record_residual(&mut snap, format!("{METADATA_DIR}/{name}"), &entry);
                }
            }
        }
    }

    let claimed: BTreeSet<String> = snap
        .skills
        .values()
        .map(|skill| skill.meta.path.clone())
        .chain(snap.profile_documents.keys().cloned())
        .collect();
    collect_residual(repo, tree, "", &claimed, &mut snap.residual)?;

    Ok(snap)
}

/// Record a metadata-namespace entry we don't own as residual so it still
/// merges (whole-file) instead of being silently dropped. Directories are
/// flattened to their files.
fn record_residual(snap: &mut Snapshot, path: String, entry: &git2::TreeEntry) {
    if entry.kind() == Some(ObjectType::Blob) {
        snap.residual
            .insert(path, FileEntry { oid: entry.id(), mode: entry.filemode() });
    }
    // Unknown subtrees under .skills-manager are intentionally not descended:
    // nothing writes them today, and treating them as opaque would need
    // whole-tree semantics we don't have. The validator does not reject them.
}

fn collect_residual(
    repo: &Repository,
    tree: &Tree,
    prefix: &str,
    claimed: &BTreeSet<String>,
    out: &mut BTreeMap<String, FileEntry>,
) -> Result<()> {
    for entry in tree.iter() {
        let name = entry.name().unwrap_or_default();
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if prefix.is_empty() && name == METADATA_DIR {
            continue; // handled by the metadata reader
        }
        match entry.kind() {
            Some(ObjectType::Tree) => {
                if claimed.contains(&path) {
                    continue; // skill content, merged as one subtree
                }
                let sub = repo.find_tree(entry.id())?;
                collect_residual(repo, &sub, &path, claimed, out)?;
            }
            Some(ObjectType::Blob) => {
                out.insert(path, FileEntry { oid: entry.id(), mode: entry.filemode() });
            }
            _ => {} // commits (submodules) etc. — not supported, ignored
        }
    }
    Ok(())
}

/// Whether a tree (a directory) is a valid skill dir: directly contains one
/// of the marker files as a blob.
pub fn tree_is_valid_skill_dir(tree: &Tree) -> bool {
    SKILL_DIR_MARKERS.iter().any(|marker| {
        tree.get_name(marker)
            .map(|e| e.kind() == Some(ObjectType::Blob))
            .unwrap_or(false)
    })
}
