import { useState } from "react";
import { statusConfig, type MediaStatus, type MediaType, mediaTypeConfig, getProgressLabel } from "@/lib/media-types";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

interface UnifiedMediaCardProps {
  title: string;
  posterUrl?: string;
  mediaType?: MediaType;
  episodes?: number;
  progress?: number;
  status?: MediaStatus;
  score?: number;
  
  // Actions
  onPosterClick?: () => void;
  onDelete?: () => void;
  onStatusChange?: (status: MediaStatus) => void;
  onProgressChange?: (progress: number) => void;
  onAdd?: () => void;
  
  // State
  isAdding?: boolean;
  isAdded?: boolean;
  isDeleting?: boolean;
  isLoading?: boolean;
  
  variant?: "tracking" | "search";
}

function ProgressControl({
  current,
  total,
  unit,
  onChange,
}: {
  current: number;
  total: number;
  unit: string;
  onChange: (val: number) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [inputVal, setInputVal] = useState(String(current));

  async function save(val: number) {
    const clamped = Math.max(0, Math.min(total || Infinity, val));
    if (clamped !== current) {
      onChange(clamped);
    }
  }

  return (
    <div className="flex h-6 items-center gap-1 text-xs text-muted-foreground">
      <button
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={() => onChange(Math.max(0, current - 1))}
      >
        −
      </button>

      {editing ? (
        <input
          autoFocus
          type="number"
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
              setInputVal(String(current));
            }
          }}
          className="w-8 text-center bg-white/10 rounded px-1 text-foreground [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none h-5"
        />
      ) : (
        <span
          className="min-w-[2ch] text-center text-foreground tabular-nums cursor-pointer hover:text-white transition-colors"
          onClick={() => {
            setEditing(true);
            setInputVal(String(current));
          }}
        >
          {current}
        </span>
      )}

      <button
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={() => onChange(Math.min(total || Infinity, current + 1))}
      >
        +
      </button>

      <span className="text-muted-foreground">/ {total || "?"} {unit}</span>
    </div>
  );
}

export function UnifiedMediaCard({
  title,
  posterUrl,
  mediaType,
  episodes,
  progress = 0,
  status,
  score,
  onPosterClick,
  onDelete,
  onStatusChange,
  onProgressChange,
  onAdd,
  isAdding,
  isAdded,
  isDeleting,
  isLoading,
  variant = "tracking",
}: UnifiedMediaCardProps) {
  const [statusOpen, setStatusOpen] = useState(false);
  
  const uiStatus = status || "plan-to-watch";
  const config = statusConfig[uiStatus];
  
  const unit = mediaType ? getProgressLabel(mediaType) : "эп.";

  return (
    <div className="group relative flex flex-col overflow-hidden rounded-xl border border-border bg-card transition-all hover:border-muted-foreground/30 h-full">
      {/* Delete button */}
      {variant === "tracking" && onDelete && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          disabled={isDeleting}
          className="absolute right-2 top-2 z-20 flex h-6 w-6 items-center justify-center rounded-full bg-black/60 text-xs text-white/70 opacity-0 transition-all hover:bg-black/80 hover:text-red-400 group-hover:opacity-100"
        >
          ✕
        </button>
      )}

      {/* Poster */}
      <div 
        className="relative aspect-[2/3] w-full overflow-hidden bg-muted cursor-pointer shrink-0"
        onClick={onPosterClick}
      >
        {posterUrl ? (
          <img
            src={posterUrl}
            alt={title}
            referrerPolicy="no-referrer"
            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground italic">
            Нет обложки
          </div>
        )}
        
        {score && (
          <div className="absolute bottom-2 right-2 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-bold text-yellow-500 backdrop-blur-sm shadow-sm">
            {score.toFixed(1)}
          </div>
        )}
      </div>

      {/* Info Content */}
      <div className="flex flex-col gap-2 p-3 flex-1 min-h-0">
        <h3 className="line-clamp-2 text-sm font-medium text-foreground leading-snug min-h-[2.5rem]" title={title}>
          {title}
        </h3>

        {/* Progress row */}
        <div className="h-6 flex items-center">
          {variant === "tracking" && onProgressChange ? (
            <ProgressControl
              current={progress}
              total={episodes || 0}
              unit={unit}
              onChange={onProgressChange}
            />
          ) : episodes ? (
            <span className="text-xs text-muted-foreground">
              {episodes} {unit}
            </span>
          ) : (
            <span className="text-xs text-muted-foreground/30 italic">Без прогресса</span>
          )}
        </div>

        {/* Actions row */}
        <div className="relative mt-auto pt-1 h-8">
          {variant === "tracking" && onStatusChange ? (
            <>
              <button
                onClick={() => setStatusOpen(!statusOpen)}
                disabled={isLoading}
                className="w-full h-full rounded-lg text-xs font-medium transition-all flex items-center justify-between px-3 bg-secondary/50 hover:bg-secondary"
                style={{
                  color: uiStatus === "watching" ? "var(--watching)" :
                         uiStatus === "completed" ? "var(--completed)" :
                         uiStatus === "dropped" ? "var(--dropped)" :
                         "var(--planned)",
                }}
              >
                <span>{isLoading ? "..." : config.label}</span>
                <span className="text-[10px] opacity-50">▼</span>
              </button>

              {statusOpen && (
                <>
                  <div className="fixed inset-0 z-40" onClick={() => setStatusOpen(false)} />
                  <div className="absolute bottom-full left-0 z-50 mb-1 w-full overflow-hidden rounded-lg border border-border bg-popover shadow-xl animate-in fade-in slide-in-from-bottom-2 duration-200">
                    {(Object.entries(statusConfig) as [MediaStatus, any][]).map(([statusKey, cfg]) => (
                      <button
                        key={statusKey}
                        onClick={() => {
                          onStatusChange(statusKey);
                          setStatusOpen(false);
                        }}
                        className="w-full px-3 py-2.5 text-left text-xs text-foreground transition-colors hover:bg-secondary flex items-center justify-between"
                      >
                        {cfg.label}
                        {uiStatus === statusKey && <span className="text-[10px] text-primary">●</span>}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </>
          ) : variant === "search" && onAdd ? (
            <Button
              size="sm"
              disabled={isAdding || isAdded}
              onClick={onAdd}
              className={cn(
                "w-full h-full rounded-lg text-xs font-semibold",
                isAdded && "opacity-60",
              )}
              variant={isAdded ? "outline" : "default"}
            >
              {isAdded ? "✓ Добавлено" : isAdding ? "..." : "+ В список"}
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
