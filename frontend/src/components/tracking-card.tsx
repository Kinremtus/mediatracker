import { useState } from "react";
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

function ProgressControl({
  entry,
  onUpdate,
}: {
  entry: TrackingEntry;
  onUpdate: (id: number, status: string, progress?: number) => void;
}) {
  const [progress, setProgress] = useState(entry.progress);
  const [editing, setEditing] = useState(false);
  const [inputVal, setInputVal] = useState(String(entry.progress));

  const episodes = entry.media?.episodes;
  const total = episodes ?? 0;

  async function save(val: number) {
    const clamped = Math.max(0, Math.min(total, val));
    setProgress(clamped);
    setInputVal(String(clamped));
    if (clamped !== entry.progress) {
      await updateTracking(entry.id, { progress: clamped });
      onUpdate(entry.id, entry.status, clamped);
    }
  }

  async function adjust(delta: number) {
    const next = Math.max(0, Math.min(total, progress + delta));
    setProgress(next);
    setInputVal(String(next));
    await updateTracking(entry.id, { progress: next });
    onUpdate(entry.id, entry.status, next);
  }

  function handleWheel(e: React.WheelEvent) {
    e.preventDefault();
    adjust(e.deltaY < 0 ? 1 : -1);
  }

  return (
    <div
      className="flex items-center gap-1 text-xs text-muted-foreground"
      onWheel={handleWheel}
    >
      <button
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={() => adjust(-1)}
      >
        −
      </button>

      {editing ? (
        <input
          autoFocus
          type="number"
          min={0}
          max={total || undefined}
          value={inputVal}
          onChange={(e) => setInputVal(e.target.value)}
          onBlur={() => {
            setEditing(false);
            save(parseInt(inputVal) || 0);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              setEditing(false);
              save(parseInt(inputVal) || 0);
            }
            if (e.key === "Escape") {
              setEditing(false);
              setInputVal(String(progress));
            }
          }}
          className="w-8 text-center bg-white/10 rounded px-1 text-foreground [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        />
      ) : (
        <span
          className="min-w-[2ch] text-center text-foreground tabular-nums cursor-pointer hover:text-white transition-colors"
          onClick={() => {
            setEditing(true);
            setInputVal(String(progress));
          }}
          title="Нажми чтобы ввести вручную"
        >
          {progress}
        </span>
      )}

      <button
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={() => adjust(1)}
      >
        +
      </button>

      <span className="text-muted-foreground">/ {total || "?"} эп.</span>
    </div>
  );
}

export function TrackingCard({
  entry,
  onUpdate,
  onDelete,
  onPosterClick,
}: {
  entry: TrackingEntry;
  onUpdate: (id: number, status: string, progress?: number) => void;
  onDelete: (id: number) => void;
  onPosterClick?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const uiStatus = mapStatusToUI(entry.status);
  const config = statusConfig[uiStatus];

  async function handleStatusChange(status: string) {
    setLoading(true);
    setOpen(false);
    try {
      await updateTracking(entry.id, { status: mapStatusToBackend(status) });
      onUpdate(entry.id, mapStatusToBackend(status));
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
        className="aspect-[2/3] w-full object-cover cursor-pointer"
        onClick={onPosterClick}
      />

      <div className="flex flex-col gap-2 p-3">
        <p className="truncate text-sm font-medium text-foreground">
          {entry.media.title_russian || entry.media.title}
        </p>

        {episodes != null && (
          <ProgressControl entry={entry} onUpdate={onUpdate} />
        )}

        <div className="relative">
          <button
            onClick={() => setOpen(!open)}
            disabled={loading}
            className="text-xs transition-opacity hover:opacity-80"
            style={{
              color: uiStatus === "watching" ? "var(--watching)" :
                     uiStatus === "completed" ? "var(--completed)" :
                     uiStatus === "dropped" ? "var(--dropped)" :
                     "var(--planned)",
            }}
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