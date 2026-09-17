import { useEffect, useMemo, useState } from "react";
import { Box, Check, Download, Loader2, PackagePlus, Play, Square, Trash2, X } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { Plugin, PluginInstallPreview } from "../lib/tauri";

export function Plugins() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const { plugins, managedSkills, refreshManagedSkills, refreshPlugins } = useApp();
  const [importUrl, setImportUrl] = useState("");
  const [preview, setPreview] = useState<PluginInstallPreview | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [manualName, setManualName] = useState("");
  const [manualDescription, setManualDescription] = useState("");
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const [deleteTarget, setDeleteTarget] = useState<Plugin | null>(null);


  useEffect(() => {
    if (searchParams.get("new") === "1") {
      setCreating(true);
      setSearchParams({}, { replace: true });
    }
  }, [searchParams, setSearchParams]);
  const skillNames = useMemo(
    () => new Map(managedSkills.map((skill) => [skill.id, skill.name])),
    [managedSkills],
  );

  const refresh = async () => {
    await Promise.all([refreshPlugins(), refreshManagedSkills()]);
  };

  const handlePreview = async () => {
    if (!importUrl.trim()) return;
    setBusy("preview");
    try {
      setPreview(await api.previewPluginInstall(importUrl.trim()));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(null);
    }
  };

  const handleCancelPreview = async () => {
    if (preview) await api.cancelPluginPreview(preview.temp_dir).catch(() => {});
    setPreview(null);
  };

  const handleConfirmImport = async () => {
    if (!preview) return;
    setBusy("confirm");
    try {
      await api.confirmPluginInstall(importUrl.trim(), preview.temp_dir);
      setPreview(null);
      setImportUrl("");
      await refresh();
      toast.success(t("plugins.imported"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(null);
    }
  };

  const handleCreate = async () => {
    if (!manualName.trim() || selectedSkillIds.length === 0) return;
    setBusy("create");
    try {
      await api.createManualPlugin(
        manualName.trim(),
        manualDescription.trim() || null,
        selectedSkillIds,
        [],
      );
      setManualName("");
      setManualDescription("");
      setSelectedSkillIds([]);
      setCreating(false);
      await refreshPlugins();
      toast.success(t("plugins.created"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(null);
    }
  };

  const toggleSkill = (skillId: string) => {
    setSelectedSkillIds((current) =>
      current.includes(skillId)
        ? current.filter((id) => id !== skillId)
        : [...current, skillId],
    );
  };

  const handleActivation = async (pluginId: string, active: boolean) => {
    setBusy(pluginId);
    try {
      if (active) await api.deactivatePlugin(pluginId);
      else await api.activatePlugin(pluginId);
      await refresh();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(null);
    }
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    setBusy(deleteTarget.id);
    try {
      await api.deletePlugin(deleteTarget.id);
      await refresh();
      toast.success(t("plugins.deleted"));
    } catch (error) {
      toast.error(String(error));
      throw error;
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mx-auto flex w-full max-w-[1000px] flex-col gap-5 pb-8">
      <header className="flex flex-wrap items-end justify-between gap-3 border-b border-border-subtle pb-4">
        <div>
          <p className="mb-1 text-[12px] font-semibold uppercase tracking-[0.08em] text-muted">{t("plugins.eyebrow")}</p>
          <h1 className="text-2xl font-semibold tracking-tight text-primary">{t("plugins.title")}</h1>
          <p className="mt-1 max-w-[65ch] text-sm leading-6 text-muted">{t("plugins.description")}</p>
        </div>
        <button
          type="button"
          onClick={() => setCreating((open) => !open)}
          className="inline-flex min-h-10 items-center gap-2 rounded-md border border-border bg-surface px-3 text-sm font-medium text-secondary transition-colors hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <PackagePlus className="h-4 w-4" />
          {t("plugins.new")}
        </button>
      </header>

      <section className="rounded-lg border border-border bg-surface p-4">
        <label htmlFor="plugin-import-url" className="text-sm font-medium text-secondary">{t("plugins.importLabel")}</label>
        <div className="mt-2 flex flex-col gap-2 sm:flex-row">
          <input
            id="plugin-import-url"
            value={importUrl}
            onChange={(event) => setImportUrl(event.target.value)}
            placeholder="https://github.com/cursor/plugins/tree/main/pstack"
            className="min-h-10 min-w-0 flex-1 rounded-md border border-border bg-surface px-3 text-base text-primary outline-none placeholder:text-faint focus:border-accent"
          />
          <button
            type="button"
            onClick={handlePreview}
            disabled={!importUrl.trim() || busy !== null}
            className="inline-flex min-h-10 items-center justify-center gap-2 rounded-md bg-accent px-4 text-sm font-semibold text-white transition-colors hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-50"
          >
            {busy === "preview" ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            {t("plugins.import")}
          </button>
        </div>
      </section>

      {preview && (
        <section className="rounded-lg border border-accent/40 bg-accent-bg p-4" aria-live="polite">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-base font-semibold text-primary">{preview.plugin.display_name}</h2>
              <p className="mt-1 text-sm text-muted">{preview.plugin.description || t("plugins.noDescription")}</p>
            </div>
            <button type="button" onClick={handleCancelPreview} className="rounded p-2 text-muted hover:bg-surface-hover hover:text-secondary" aria-label={t("common.cancel")}><X className="h-4 w-4" /></button>
          </div>
          <p className="mt-3 text-sm text-secondary">{t("plugins.importSkills", { count: preview.plugin.skills.length })}</p>
          <ul className="mt-2 grid gap-1 sm:grid-cols-2">
            {preview.plugin.skills.map((skill) => <li key={skill.relative_path} className="truncate text-sm text-muted">{skill.name}</li>)}
          </ul>
          {(preview.plugin.ignored_agents || preview.plugin.ignored_rules) && <p className="mt-3 text-sm text-amber-500">{t("plugins.ignoredComponents")}</p>}
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" onClick={handleCancelPreview} className="min-h-10 rounded-md border border-border px-3 text-sm font-medium text-secondary hover:bg-surface-hover">{t("common.cancel")}</button>
            <button type="button" onClick={handleConfirmImport} disabled={busy !== null} className="inline-flex min-h-10 items-center gap-2 rounded-md bg-accent px-3 text-sm font-semibold text-white disabled:opacity-50">
              {busy === "confirm" && <Loader2 className="h-4 w-4 animate-spin" />}<Check className="h-4 w-4" />{t("plugins.confirmImport")}
            </button>
          </div>
        </section>
      )}

      {creating && (
        <section className="rounded-lg border border-border bg-surface p-4">
          <h2 className="text-base font-semibold text-primary">{t("plugins.new")}</h2>
          <div className="mt-3 grid gap-3">
            <input value={manualName} onChange={(event) => setManualName(event.target.value)} placeholder={t("plugins.namePlaceholder")} className="min-h-10 rounded-md border border-border bg-surface px-3 text-base text-primary outline-none focus:border-accent" />
            <textarea value={manualDescription} onChange={(event) => setManualDescription(event.target.value)} placeholder={t("plugins.descriptionPlaceholder")} className="min-h-20 rounded-md border border-border bg-surface px-3 py-2 text-base text-primary outline-none focus:border-accent" />
            <div className="grid max-h-44 gap-1 overflow-y-auto rounded-md border border-border p-2 sm:grid-cols-2">
              {managedSkills.map((skill) => <label key={skill.id} className="flex min-h-9 items-center gap-2 rounded px-2 text-sm text-secondary hover:bg-surface-hover"><input type="checkbox" checked={selectedSkillIds.includes(skill.id)} onChange={() => toggleSkill(skill.id)} />{skill.name}</label>)}
            </div>
            <div className="flex justify-end gap-2"><button type="button" onClick={() => setCreating(false)} className="min-h-10 rounded-md border border-border px-3 text-sm font-medium text-secondary hover:bg-surface-hover">{t("common.cancel")}</button><button type="button" onClick={handleCreate} disabled={!manualName.trim() || selectedSkillIds.length === 0 || busy !== null} className="min-h-10 rounded-md bg-accent px-3 text-sm font-semibold text-white disabled:opacity-50">{t("common.create")}</button></div>
          </div>
        </section>
      )}

      <section className="grid gap-3">
        {plugins.map((plugin) => (
          <article key={plugin.id} className="rounded-lg border border-border bg-surface p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="flex min-w-0 gap-3"><span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border bg-surface-hover text-accent"><Box className="h-4 w-4" /></span><div className="min-w-0"><div className="flex items-center gap-2"><h2 className="truncate text-base font-semibold text-primary">{plugin.display_name}</h2>{plugin.active && <span className="rounded bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-500">{t("plugins.active")}</span>}</div><p className="mt-1 text-sm text-muted">{plugin.description || t("plugins.noDescription")}</p><p className="mt-2 text-xs text-faint">{t("plugins.members", { count: plugin.skill_ids.length })} · {t("plugins.dependencies", { count: plugin.dependency_ids.length })}{plugin.version ? ` · v${plugin.version}` : ""}</p></div></div>
              <div className="flex flex-wrap gap-2">
                <button type="button" onClick={() => handleActivation(plugin.id, plugin.active)} disabled={busy !== null} className="inline-flex min-h-10 items-center gap-2 rounded-md border border-border px-3 text-sm font-medium text-secondary hover:bg-surface-hover disabled:opacity-50">{busy === plugin.id ? <Loader2 className="h-4 w-4 animate-spin" /> : plugin.active ? <Square className="h-4 w-4" /> : <Play className="h-4 w-4" />}{plugin.active ? t("plugins.deactivate") : t("plugins.activate")}</button>
                <button type="button" onClick={() => setDeleteTarget(plugin)} disabled={busy !== null} className="inline-flex min-h-10 items-center gap-2 rounded-md border border-red-500/40 px-3 text-sm font-medium text-red-400 hover:bg-red-500/10 disabled:opacity-50"><Trash2 className="h-4 w-4" />{t("common.delete")}</button>
              </div>
            </div>
            <div className="mt-3 flex flex-wrap gap-1.5">{plugin.skill_ids.map((skillId) => <span key={skillId} className="rounded border border-border-subtle bg-surface-hover px-2 py-1 text-xs text-muted">{skillNames.get(skillId) || skillId}</span>)}</div>
          </article>
        ))}
        {plugins.length === 0 && <div className="rounded-lg border border-dashed border-border p-8 text-center text-sm text-muted">{t("plugins.empty")}</div>}
      </section>
      <ConfirmDialog
        open={deleteTarget !== null}
        title={t("plugins.deleteTitle")}
        message={t("plugins.deleteConfirm", { name: deleteTarget?.display_name || "" })}
        confirmLabel={t("common.delete")}
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleDelete}
      />
    </div>
  );
}
