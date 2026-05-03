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
    <div className="flex h-6 items-center gap-1.5 text-xs text-muted-foreground">
      <button
        type="button"
        className="w-5 h-5 rounded flex items-center justify-center bg-secondary hover:bg-secondary/80 transition-colors text-foreground select-none"
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
          className="w-8 text-center bg-secondary rounded h-5 text-foreground [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        />
      ) : (
        <span
          className="min-w-[1.5ch] text-center text-foreground tabular-nums cursor-pointer hover:text-primary transition-colors font-medium"
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
        className="w-5 h-5 rounded flex items-center justify-center bg-secondary hover:bg-secondary/80 transition-colors text-foreground select-none"
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
    <div className="group relative flex flex-col overflow-hidden rounded-xl border border-border bg-card transition-all hover:border-muted-foreground/30 h-full shadow-sm">
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
            className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground italic p-4 text-center">
            Нет обложки
          </div>
        )}
        
        {score && score > 0 && (
          <div className="absolute bottom-2 right-2 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-bold text-yellow-500 backdrop-blur-sm shadow-sm border border-white/10">
            {(score / 10).toFixed(1)}
          </div>
        )}
      </div>

      {/* Info Content */}
      <div className="flex flex-col p-3 flex-1 min-h-0 gap-3">
        <div className="flex flex-col gap-1.5">
          <h3 className="line-clamp-2 text-sm font-semibold text-foreground leading-snug h-10 overflow-hidden" title={title}>
            {title}
          </h3>

          {/* Progress row - Rigid height */}
          <div className="h-6 flex items-center overflow-hidden">
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
        </div>

        {/* Actions row - Standardized height */}
        <div className="relative mt-auto h-9">
          {variant === "tracking" && onStatusChange ? (
            <>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  setStatusOpen(!statusOpen);
                }}
                disabled={isLoading}
                className="w-full h-full rounded-lg text-[11px] font-bold transition-all flex items-center justify-between px-3 bg-secondary/80 hover:bg-secondary border border-transparent hover:border-border shadow-sm active:scale-[0.98]"
                style={{
                  color: uiStatus === "watching" ? "var(--watching)" :
                         uiStatus === "completed" ? "var(--completed)" :
                         uiStatus === "dropped" ? "var(--dropped)" :
                         "var(--planned)",
                }}
              >
                <span className="truncate mr-2">{isLoading ? "..." : config.label.toUpperCase()}</span>
                <span className="text-[9px] opacity-40">▼</span>
              </button>

              {statusOpen && (
                <>
                  <div className="fixed inset-0 z-40" onClick={() => setStatusOpen(false)} />
                  <div className="absolute bottom-full left-0 z-50 mb-2 w-full overflow-hidden rounded-xl border border-border bg-popover shadow-2xl animate-in fade-in slide-in-from-bottom-2 duration-200">
                    {(Object.entries(statusConfig) as [MediaStatus, any][]).map(([statusKey, cfg]) => (
                      <button
                        key={statusKey}
                        onClick={(e) => {
                          e.stopPropagation();
                          onStatusChange(statusKey);
                          setStatusOpen(false);
                        }}
                        className={cn(
                          "w-full px-3 py-2.5 text-left text-[11px] font-medium transition-colors hover:bg-secondary flex items-center justify-between",
                          uiStatus === statusKey ? "bg-secondary/50 text-foreground" : "text-muted-foreground hover:text-foreground"
                        )}
                      >
                        {cfg.label}
                        {uiStatus === statusKey && <div className="h-1.5 w-1.5 rounded-full bg-primary shadow-[0_0_8px_var(--primary)]" />}
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
                "w-full h-full rounded-lg text-[11px] font-bold shadow-sm transition-transform active:scale-[0.98]",
                isAdded ? "bg-secondary text-muted-foreground border-border hover:bg-secondary/80" : "bg-primary text-primary-foreground",
              )}
              variant={isAdded ? "outline" : "default"}
            >
              {isAdded ? "✓ В СПИСКЕ" : isAdding ? "..." : "+ В СПИСОК"}
            </Button>
          ) : (
            <div className="w-full h-full rounded-lg bg-muted/20 border border-dashed border-border flex items-center justify-center">
               <span className="text-[10px] text-muted-foreground/30 uppercase tracking-wider">Нет действий</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
