import { useState } from "react";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

interface Props {
  open: boolean;
  onClose: () => void;
  onCreate: (name: string) => Promise<void>;
}

export function CreateProfileDialog({ open, onClose, onCreate }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [loading, setLoading] = useState(false);

  if (!open) return null;

  const handleCreate = async () => {
    if (!name.trim()) return;
    setLoading(true);
    try {
      await onCreate(name.trim());
      setName("");
      onClose();
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/70 backdrop-blur-sm" onClick={onClose} />
      <div className="relative w-full max-w-[400px] rounded-xl border border-border bg-surface p-5 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-[13px] font-semibold text-primary">{t("profiles.create")}</h2>
          <button onClick={onClose} className="rounded p-1 text-muted transition-colors outline-none hover:text-secondary">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div>
          <label className="mb-1 block text-[13px] font-medium text-tertiary">{t("profiles.name")}</label>
          <input
            type="text"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder={t("profiles.namePlaceholder")}
            className="w-full rounded-lg border border-border-subtle bg-background px-3 py-2 text-[13px] text-secondary transition-all placeholder:text-faint focus:border-border focus:outline-none"
            autoFocus
            onKeyDown={(event) => event.key === "Enter" && void handleCreate()}
          />
          <div className="flex justify-end gap-2 pt-4">
            <button
              onClick={onClose}
              className="rounded-lg px-3 py-1.5 text-[13px] font-medium text-tertiary transition-colors outline-none hover:bg-surface-hover hover:text-secondary"
            >
              {t("common.cancel")}
            </button>
            <button
              onClick={() => void handleCreate()}
              disabled={!name.trim() || loading}
              className="rounded-lg border border-accent-border bg-accent-dark px-3 py-1.5 text-[13px] font-medium text-white transition-colors outline-none hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading ? t("common.loading") : t("common.create")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
