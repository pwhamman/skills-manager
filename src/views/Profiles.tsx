import { useEffect, useState } from "react";
import { Check, ChevronDown, ChevronRight, FileText, FolderOpen, Pencil, Plus, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { Profile } from "../lib/tauri";

export function Profiles() {
  const { t } = useTranslation();
  const { profiles, refreshProfiles } = useApp();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [document, setDocument] = useState("");
  const [nameDraft, setNameDraft] = useState("");
  const [newName, setNewName] = useState("");
  const [homeFolders, setHomeFolders] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [folderMenuOpen, setFolderMenuOpen] = useState(false);
  const [profilesOpen, setProfilesOpen] = useState(true);
  const [creating, setCreating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Profile | null>(null);

  const selected = profiles.find((profile) => profile.id === selectedId) ?? profiles[0] ?? null;

  useEffect(() => {
    if (!selected) {
      setNameDraft("");
      setSelectedFolder(null);
      return;
    }
    setSelectedId(selected.id);
    setNameDraft(selected.name);
    setSelectedFolder(null);
  }, [selected?.id]);

  useEffect(() => {
    if (!selected) {
      setDocument("");
      return;
    }
    api.getProfileDocument(selected.id, selectedFolder)
      .then(setDocument)
      .catch(() => setDocument(""));
  }, [selected?.id, selectedFolder]);

  useEffect(() => {
    api.listProfileHomeFolders().then(setHomeFolders).catch(() => setHomeFolders([]));
  }, []);

  const createProfile = async () => {
    const name = newName.trim();
    if (!name) return;
    const profile = await api.createProfile(name);
    setNewName("");
    setCreating(false);
    setSelectedId(profile.id);
    setSelectedFolder(null);
    await refreshProfiles();
    toast.success(t("profiles.created"));
  };

  const renameProfile = async () => {
    if (!selected || !nameDraft.trim() || nameDraft.trim() === selected.name) return;
    await api.renameProfile(selected.id, nameDraft.trim());
    await refreshProfiles();
    toast.success(t("profiles.renamed"));
  };

  const saveDocument = async () => {
    if (!selected) return;
    setSaving(true);
    try {
      await api.saveProfileDocument(selected.id, selectedFolder, document);
      await refreshProfiles();
      toast.success(t("profiles.saved"));
    } finally {
      setSaving(false);
    }
  };

  const addFolder = async (folderName: string) => {
    if (!selected) return;
    await api.addProfileFolder(selected.id, folderName);
    setSelectedFolder(folderName);
    await refreshProfiles();
  };

  const removeFolder = async (folderName: string) => {
    if (!selected) return;
    await api.removeProfileFolder(selected.id, folderName);
    if (selectedFolder === folderName) setSelectedFolder(null);
    await refreshProfiles();
  };

  const activate = async () => {
    if (!selected) return;
    await api.activateProfile(selected.id);
    await refreshProfiles();
    toast.success(t("profiles.activated"));
  };

  return (
    <div className="app-page">
      <div className="app-page-header">
        <h1 className="app-page-title">{t("profiles.title")}</h1>
        <p className="app-page-subtitle">{t("profiles.description")}</p>
      </div>

      <div className="grid gap-4 xl:grid-cols-[260px_minmax(0,1fr)]">
        <aside className="h-fit">
          <div className="mb-1.5 px-2.5">
            <button
              type="button"
              onClick={() => setProfilesOpen((open) => !open)}
              className="flex min-w-0 items-center gap-1 text-left outline-none"
            >
              {profilesOpen
                ? <ChevronDown className="h-3 w-3 shrink-0 text-faint" />
                : <ChevronRight className="h-3 w-3 shrink-0 text-faint" />}
              <span className="truncate text-[12px] font-semibold tracking-[0.01em] text-muted">
                {t("sidebar.profiles")}
              </span>
            </button>
          </div>
          {profilesOpen && (
            <>
              <div className="space-y-0.5">
                {profiles.map((profile) => {
                  const isSelected = selected?.id === profile.id;
                  return (
                    <button
                      key={profile.id}
                      type="button"
                      onClick={() => {
                        setCreating(false);
                        setSelectedId(profile.id);
                        setSelectedFolder(null);
                      }}
                      className={`flex w-full items-center gap-2 rounded-md px-2.5 py-[7px] text-left text-sm leading-5 transition-colors ${isSelected ? "bg-surface-active font-medium text-primary" : "text-tertiary hover:bg-surface-hover hover:text-secondary"}`}
                    >
                      <span className={`flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded border ${isSelected ? "border-accent/30 bg-accent/10 text-accent" : "border-border bg-surface text-muted"}`}>
                        <FileText className="h-3 w-3" />
                      </span>
                      <span className="min-w-0 flex-1 truncate">{profile.name}</span>
                      {profile.active && <Check className="h-4 w-4 shrink-0 text-accent" aria-label={t("profiles.active")} />}
                    </button>
                  );
                })}
                {profiles.length === 0 && <p className="px-2.5 py-2 text-[13px] text-muted">{t("profiles.noProfiles")}</p>}
              </div>
              {creating ? (
                <form
                  className="mt-1 flex gap-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void createProfile();
                  }}
                >
                  <input
                    autoFocus
                    value={newName}
                    onChange={(event) => setNewName(event.target.value)}
                    onBlur={() => { if (!newName.trim()) setCreating(false); }}
                    placeholder={t("profiles.namePlaceholder")}
                    className="app-input h-8 min-w-0 flex-1 px-2.5"
                  />
                  <button type="submit" className="app-button-primary h-8 w-8 shrink-0 p-0" aria-label={t("profiles.new")}>
                    <Plus className="h-3.5 w-3.5" />
                  </button>
                </form>
              ) : (
                <button
                  type="button"
                  onClick={() => setCreating(true)}
                  className="mt-1 flex w-full items-center gap-2 rounded-md px-2.5 py-[7px] text-sm text-muted transition-colors outline-none hover:bg-surface-hover hover:text-secondary"
                >
                  <Plus className="h-3.5 w-3.5" />
                  {t("profiles.new")}
                </button>
              )}
            </>
          )}
        </aside>

        {selected && (
          <section className="app-panel min-w-0 overflow-hidden">
            <div className="flex flex-wrap items-center gap-2 border-b border-border-subtle px-4 py-3.5">
              <input
                value={nameDraft}
                onChange={(event) => setNameDraft(event.target.value)}
                onBlur={() => void renameProfile()}
                className="h-8 min-w-[160px] flex-1 rounded-lg border border-transparent bg-transparent px-2.5 text-[14px] font-semibold text-secondary outline-none transition-colors focus:border-border focus:bg-background"
                aria-label={t("profiles.namePlaceholder")}
              />
              <button
                type="button"
                onClick={() => void renameProfile()}
                className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-secondary"
                aria-label={t("common.rename")}
              >
                <Pencil className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => void activate()}
                disabled={selected.active}
                className="app-button-primary h-8 px-3 disabled:cursor-default"
              >
                {selected.active ? t("profiles.active") : t("profiles.activate")}
              </button>
              <button
                type="button"
                onClick={() => setDeleteTarget(selected)}
                className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted transition-colors hover:bg-danger-bg hover:text-danger"
                aria-label={t("common.delete")}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>

            <div className="grid gap-4 p-4 xl:grid-cols-[220px_minmax(0,1fr)]">
              <div className="space-y-3">
                <p className="app-section-title">{t("profiles.folders")}</p>
                <button
                  type="button"
                  onClick={() => setSelectedFolder(null)}
                  className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] font-medium transition-colors ${selectedFolder === null ? "bg-surface-active text-secondary" : "text-muted hover:bg-surface-hover hover:text-secondary"}`}
                >
                  <FileText className="h-4 w-4" />
                  {t("profiles.canonical")}
                </button>
                <p className="text-[12px] leading-5 text-muted">{t("profiles.folderHint")}</p>
                <div className="relative">
                  <button
                    type="button"
                    onClick={() => setFolderMenuOpen((open) => !open)}
                    disabled={homeFolders.every((folder) => selected.folders.includes(folder))}
                    className="app-button-secondary h-10 w-full justify-between bg-background px-3 disabled:cursor-not-allowed"
                    aria-expanded={folderMenuOpen}
                    aria-haspopup="listbox"
                  >
                    {t("profiles.addFolder")}
                    <ChevronDown className="h-3.5 w-3.5" />
                  </button>
                  {folderMenuOpen && (
                    <div role="listbox" className="absolute top-full z-30 mt-1.5 max-h-52 w-full overflow-y-auto rounded-lg border border-border bg-surface p-1 shadow-lg">
                      {homeFolders.filter((folder) => !selected.folders.includes(folder)).map((folder) => (
                        <button
                          key={folder}
                          type="button"
                          role="option"
                          onClick={() => {
                            setFolderMenuOpen(false);
                            void addFolder(folder);
                          }}
                          className="flex w-full items-center rounded-md px-2.5 py-2 text-left text-[13px] text-secondary transition-colors hover:bg-surface-hover"
                        >
                          ~/{folder}/AGENTS.md
                        </button>
                      ))}
                    </div>
                  )}
                </div>
                {selected.folders.map((folder) => (
                  <div key={folder} className={`flex items-center gap-1 rounded-lg ${selectedFolder === folder ? "bg-surface-active" : "hover:bg-surface-hover"}`}>
                    <button
                      type="button"
                      onClick={() => setSelectedFolder(folder)}
                      className="min-w-0 flex-1 truncate px-2.5 py-2 text-left text-[13px] text-muted transition-colors hover:text-secondary"
                    >
                      <FolderOpen className="mr-2 inline h-4 w-4" />~/{folder}/AGENTS.md
                    </button>
                    <button
                      type="button"
                      onClick={() => void removeFolder(folder)}
                      className="mr-1 inline-flex h-7 w-7 items-center justify-center rounded-lg text-faint transition-colors hover:bg-danger-bg hover:text-danger"
                      aria-label={t("profiles.removeFolder")}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}
              </div>
              <div className="min-w-0 space-y-3">
                <p className="text-[13px] font-medium text-secondary">
                  {selectedFolder ? `~/${selectedFolder}/AGENTS.md` : t("profiles.canonical")}
                </p>
                <textarea
                  value={document}
                  onChange={(event) => setDocument(event.target.value)}
                  spellCheck={false}
                  className="h-[460px] w-full resize-y rounded-lg border border-border-subtle bg-background p-3 font-mono text-[13px] leading-6 text-secondary caret-accent outline-none transition-colors placeholder:text-faint focus:border-border"
                  aria-label={selectedFolder ? `~/${selectedFolder}/AGENTS.md` : t("profiles.canonical")}
                />
                <div className="flex justify-end">
                  <button type="button" onClick={() => void saveDocument()} disabled={saving} className="app-button-primary h-8 px-3">
                    <Save className="h-3.5 w-3.5" />{t("profiles.save")}
                  </button>
                </div>
              </div>
            </div>
          </section>
        )}
      </div>
      <ConfirmDialog
        open={deleteTarget !== null}
        message={t("profiles.deleteConfirm", { name: deleteTarget?.name ?? "" })}
        onClose={() => setDeleteTarget(null)}
        onConfirm={async () => {
          if (!deleteTarget) return;
          await api.deleteProfile(deleteTarget.id);
          setSelectedId(null);
          setDeleteTarget(null);
          await refreshProfiles();
          toast.success(t("profiles.deleted"));
        }}
      />
    </div>
  );
}
