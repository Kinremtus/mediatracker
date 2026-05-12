import { useState } from "react";
import { statusConfig, type MediaStatus, type MediaType, getProgressLabel } from "@/lib/media-types";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

interface UnifiedMediaCardProps {
  title: string;
  posterUrl?: string;
  mediaType?: MediaType;
  episodes?: number | null;
  progress?: number;
  status?: MediaStatus;
  score?: number | null;
  
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
  total: number | null;
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
    <div className="flex items-center gap-1 text-xs text-muted-foreground">
      <button
        type="button"
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={(e) => {
          e.stopPropagation();
          onChange(Math.max(0, current - 1));
        }}
      >
        −
      </button>

      {editing ? (
        <input
          autoFocus
          type="number"
          value={inputVal}
          onClick={(e) => e.stopPropagation()}
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
          className="w-8 text-center bg-white/10 rounded h-5 text-foreground [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        />
      ) : (
        <span
          className="min-w-[1.5ch] text-center text-foreground tabular-nums cursor-pointer hover:text-primary transition-colors"
          onClick={(e) => {
            e.stopPropagation();
            setEditing(true);
            setInputVal(String(current));
          }}
        >
          {current}
        </span>
      )}

      <button
        type="button"
        className="w-5 h-5 rounded flex items-center justify-center bg-white/10 hover:bg-white/20 transition-colors text-foreground select-none"
        onClick={(e) => {
          e.stopPropagation();
          onChange(Math.min(total || Infinity, current + 1));
        }}
      >
        +
      </button>

      <span className="ml-0.5 opacity-70">/ {total || "?"} {unit}</span>
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

      {/* Poster Container - FIXED ASPECT RATIO */}
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
          <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground italic p-4 text-center">
            Нет обложки
          </div>
        )}
        
        {score && score > 0 && (
          <div className="absolute bottom-2 right-2 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-bold text-yellow-500 backdrop-blur-sm border border-white/10">
            {(score / 10).toFixed(1)}
          </div>
        )}
      </div>

      {/* Info Content - Return to Natural Flow */}
      <div className="flex flex-col p-3 flex-1 gap-2">
        <h3 className="truncate text-sm font-medium text-foreground" title={title}>
          {title}
        </h3>

        {/* Progress row - consistent height but simpler style */}
        <div className="h-6 flex items-center">
          {variant === "tracking" && onProgressChange ? (
            <ProgressControl
              current={progress}
              total={episodes ?? null}
              unit={unit}
              onChange={onProgressChange}
            />
          ) : (
            <div className="text-xs text-muted-foreground flex items-center gap-1">
              {episodes ? (
                <>
                  <span className="font-medium text-foreground/80">{episodes}</span>
                  <span className="opacity-70">{unit}</span>
                </>
              ) : (
                <span className="opacity-30 italic">Без данных</span>
              )}
            </div>
          )}
        </div>

        {/* Actions row */}
        <div className="relative mt-auto">
          {variant === "tracking" && onStatusChange ? (
            <>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  setStatusOpen(!statusOpen);
                }}
                disabled={isLoading}
                className="text-xs transition-opacity hover:opacity-80 flex items-center gap-1 font-medium"
                style={{
                  color: uiStatus === "watching" ? "var(--watching)" :
                         uiStatus === "completed" ? "var(--completed)" :
                         uiStatus === "dropped" ? "var(--dropped)" :
                         "var(--planned)",
                }}
              >
                {isLoading ? "..." : config.label}
                <span className="text-[10px] opacity-50">▼</span>
              </button>

              {statusOpen && (
                <>
                  <div className="fixed inset-0 z-40" onClick={() => setStatusOpen(false)} />
                  <div className="absolute bottom-full left-0 z-50 mb-1 w-36 overflow-hidden rounded-lg border border-border bg-popover shadow-lg">
                    {(Object.entries(statusConfig) as [MediaStatus, any][]).map(([statusKey, cfg]) => (
                      <button
                        key={statusKey}
                        onClick={(e) => {
                          e.stopPropagation();
                          onStatusChange(statusKey);
                          setStatusOpen(false);
                        }}
                        className={cn(
                          "w-full px-3 py-2 text-left text-xs text-foreground transition-colors hover:bg-secondary",
                          uiStatus === statusKey ? "bg-secondary/50" : ""
                        )}
                      >
                        {cfg.label}
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
              onClick={(e) => {
                e.stopPropagation();
                onAdd();
              }}
              className={cn(
                "w-full rounded-lg text-xs h-8",
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
