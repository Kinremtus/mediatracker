import { useState } from "react";
import { cn } from "@/lib/utils";
import { statusConfig, type MediaStatus } from "@/lib/media-types";
import { updateTracking, deleteTracking, type TrackingEntry } from "@/api/tracking";

export const mapStatusToUI = (backendStatus: string): MediaStatus => {
  switch (backendStatus) {
    case "in_progress": return "watching";
    case "completed": return "completed";
    case "planned": return "plan-to-watch";
    case "dropped": return "dropped";
    default: return "plan-to-watch";
  }
};

export const mapStatusToBackend = (uiStatus: string): string => {
  switch (uiStatus) {
    case "watching": return "in_progress";
    case "plan-to-watch": return "planned";
    case "completed": return "completed";
    case "dropped": return "dropped";
    default: return "planned";
  }
};

export function TrackingCard({
  entry,
  onUpdate,
  onDelete,
}: {
  entry: TrackingEntry;
  onUpdate: (id: number, status: string) => void;
  onDelete: (id: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const uiStatus = mapStatusToUI(entry.status);
  const config = statusConfig[uiStatus];
  const StatusIcon = config.icon;

  async function handleStatusChange(status: string) {
  setLoading(true);
  setOpen(false);
  try {
    await updateTracking(entry.id, { status: mapStatusToBackend(status) });
    onUpdate(entry.id, mapStatusToBackend(status)); // ← добавь эту строку
  } catch (e) { console.error(e); }
  finally { setLoading(false); }
}

  async function handleDelete() {
    setDeleting(true);
    try {
      await deleteTracking(entry.id);
      onDelete(entry.id);
    } catch (e) { console.error(e); }
    finally { setDeleting(false); }
  }

  return (
    <div className="group relative flex flex-col overflow-hidden rounded-xl border border-border bg-card transition-colors hover:border-muted-foreground/30">
      <button
        onClick={handleDelete}
        disabled={deleting}
        className="absolute right-2 top-2 z-10 flex h-6 w-6 items-center justify-center rounded-full bg-black/60 text-xs text-white/70 opacity-0 transition-all hover:bg-black/80 hover:text-red-400 group-hover:opacity-100"
      >
        ✕
      </button>

      <img
        src={entry.media.poster_url || "/placeholder.jpg"}
        alt={entry.media.title}
        className="aspect-[2/3] w-full object-cover"
      />
      
      <div className="flex flex-col gap-2 p-3">
        <p className="truncate text-sm font-medium text-foreground">
          {entry.media.title_russian || entry.media.title}
        </p>
        
        {entry.media.episodes && (
          <div className="flex items-center gap-1 text-xs text-muted-foreground">
            <input
              type="number"
              min={0}
              max={entry.media.episodes}
              defaultValue={entry.progress}
              style={{ width: `${String(entry.media.episodes).length + 1}ch` }}
              className="rounded bg-white/10 px-1 text-center text-xs text-foreground"
              onBlur={async (e) => {
                const val = Math.min(
                  parseInt(e.target.value) || 0,
                  entry.media.episodes!,
                );
                if (val !== entry.progress) {
                  await updateTracking(entry.id, { progress: val });
                  onUpdate(entry.id, entry.status);
                }
              }}
            />
            <span>/ {entry.media.episodes} эп.</span>
          </div>
        )}

        <div className="relative">
          <button
            onClick={() => setOpen(!open)}
            disabled={loading}
            // Используем color из config (он у нас теперь в Tailwind)
            className={cn("text-xs transition-opacity hover:opacity-80", config.className.split(" ")[1])}
          >
            {loading ? "..." : config.label + " ▾"}
          </button>
          
          {open && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
              <div className="absolute bottom-full left-0 z-50 mb-1 w-36 overflow-hidden rounded-lg border border-border bg-popover shadow-lg">
                {Object.entries(statusConfig).map(([statusKey, cfg]) => (
                  <button
                    key={statusKey}
                    onClick={() => handleStatusChange(statusKey)}
                    className="w-full px-3 py-2 text-left text-xs text-foreground transition-colors hover:bg-secondary"
                  >
                    {cfg.label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}