import { useEffect, useState } from "react";
import { Check, FileText, FolderOpen, Pencil, Plus, Save, Trash2 } from "lucide-react";
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
  const [deleteTarget, setDeleteTarget] = useState<Profile | null>(null);

  const selected = profiles.find((profile) => profile.id === selectedId) ?? profiles[0] ?? null;

  useEffect(() => {
    if (selected && selected.id !== selectedId) {
      setSelectedId(selected.id);
      setNameDraft(selected.name);
      setSelectedFolder(null);
    }
  }, [selected, selectedId]);

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
    <div className="h-full overflow-y-auto bg-bg px-8 py-7">
      <div className="mx-auto max-w-6xl space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-primary">{t("profiles.title")}</h1>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted">{t("profiles.description")}</p>
        </div>

        <div className="grid gap-5 lg:grid-cols-[260px_minmax(0,1fr)]">
          <aside className="rounded-lg border border-border bg-surface p-3">
            <div className="flex gap-2">
              <input
                value={newName}
                onChange={(event) => setNewName(event.target.value)}
                onKeyDown={(event) => { if (event.key === "Enter") void createProfile(); }}
                placeholder={t("profiles.namePlaceholder")}
                className="min-w-0 flex-1 rounded-md border border-border bg-bg px-2.5 py-2 text-sm text-primary outline-none focus:border-accent"
              />
              <button
                onClick={() => void createProfile()}
                className="rounded-md bg-accent px-2.5 text-white transition-colors hover:bg-accent/90"
                aria-label={t("profiles.new")}
              >
                <Plus className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-3 space-y-1">
              {profiles.map((profile) => (
                <button
                  key={profile.id}
                  onClick={() => { setSelectedId(profile.id); setSelectedFolder(null); }}
                  className={`flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors ${selected?.id === profile.id ? "bg-surface-active text-primary" : "text-tertiary hover:bg-surface-hover hover:text-secondary"}`}
                >
                  <FileText className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">{profile.name}</span>
                  {profile.active && <Check className="h-4 w-4 text-accent" aria-label={t("profiles.active")} />}
                </button>
              ))}
              {profiles.length === 0 && <p className="px-2.5 py-4 text-sm text-muted">{t("profiles.noProfiles")}</p>}
            </div>
          </aside>

          {selected && (
            <section className="min-w-0 rounded-lg border border-border bg-surface p-5">
              <div className="flex flex-wrap items-center gap-2 border-b border-border-subtle pb-4">
                <input
                  value={nameDraft}
                  onChange={(event) => setNameDraft(event.target.value)}
                  onBlur={() => void renameProfile()}
                  className="min-w-[160px] flex-1 bg-transparent text-lg font-semibold text-primary outline-none"
                  aria-label={t("profiles.namePlaceholder")}
                />
                <button onClick={() => void renameProfile()} className="rounded-md p-2 text-muted hover:bg-surface-hover hover:text-secondary" aria-label={t("common.rename")}>
                  <Pencil className="h-4 w-4" />
                </button>
                <button onClick={() => void activate()} disabled={selected.active} className="rounded-md bg-accent px-3 py-2 text-sm font-medium text-white disabled:cursor-default disabled:opacity-50">
                  {selected.active ? t("profiles.active") : t("profiles.activate")}
                </button>
                <button onClick={() => setDeleteTarget(selected)} className="rounded-md p-2 text-muted hover:bg-red-500/10 hover:text-red-500" aria-label={t("common.delete")}>
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>

              <div className="mt-5 grid gap-5 xl:grid-cols-[200px_minmax(0,1fr)]">
                <div className="space-y-3">
                  <p className="text-xs font-semibold uppercase tracking-wide text-muted">{t("profiles.folders")}</p>
                  <button
                    onClick={() => setSelectedFolder(null)}
                    className={`flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm ${selectedFolder === null ? "bg-surface-active text-primary" : "text-tertiary hover:bg-surface-hover"}`}
                  >
                    <FileText className="h-4 w-4" />
                    {t("profiles.canonical")}
                  </button>
                  <p className="text-xs leading-5 text-faint">{t("profiles.folderHint")}</p>
                  <select
                    value=""
                    onChange={(event) => { if (event.target.value) void addFolder(event.target.value); }}
                    className="w-full rounded-md border border-border bg-bg px-2.5 py-2 text-sm text-primary outline-none"
                    aria-label={t("profiles.addFolder")}
                  >
                    <option value="">{t("profiles.addFolder")}</option>
                    {homeFolders.filter((folder) => !selected.folders.includes(folder)).map((folder) => <option key={folder} value={folder}>{folder}</option>)}
                  </select>
                  {selected.folders.map((folder) => (
                    <div key={folder} className={`flex items-center gap-1 rounded-md ${selectedFolder === folder ? "bg-surface-active" : ""}`}>
                      <button onClick={() => setSelectedFolder(folder)} className="min-w-0 flex-1 truncate px-2.5 py-2 text-left text-sm text-tertiary hover:text-secondary">
                        <FolderOpen className="mr-2 inline h-4 w-4" />~/{folder}/AGENTS.md
                      </button>
                      <button onClick={() => void removeFolder(folder)} className="rounded p-1.5 text-faint hover:text-red-500" aria-label={t("profiles.removeFolder")}>
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
                <div className="min-w-0">
                  <textarea
                    value={document}
                    onChange={(event) => setDocument(event.target.value)}
                    spellCheck={false}
                    className="h-[480px] w-full resize-y rounded-md border border-border bg-bg p-3 font-mono text-sm leading-6 text-primary outline-none focus:border-accent"
                    aria-label={selectedFolder ? `~/${selectedFolder}/AGENTS.md` : t("profiles.canonical")}
                  />
                  <div className="mt-3 flex justify-end">
                    <button onClick={() => void saveDocument()} disabled={saving} className="inline-flex items-center gap-2 rounded-md bg-accent px-3 py-2 text-sm font-medium text-white disabled:opacity-50">
                      <Save className="h-4 w-4" />{t("profiles.save")}
                    </button>
                  </div>
                </div>
              </div>
            </section>
          )}
        </div>
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
