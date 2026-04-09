import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  type MediaItem,
  statusConfig,
  progressColors,
  mediaTypeConfig,
} from "@/lib/media-types";

export function MediaCard({ item }: { item: MediaItem }) {
  const config = statusConfig[item.status];
  const StatusIcon = config.icon;
  const typeConfig = mediaTypeConfig[item.type];
  const progressPercentage =
    item.totalProgress > 0
      ? (item.currentProgress / item.totalProgress) * 100
      : 0;

  return (
    <div className="group flex items-center gap-4 rounded-lg border border-border bg-card p-3 transition-all hover:border-muted-foreground/30 hover:bg-secondary/50">
      {/* Poster */}
      <div className="relative h-20 w-14 shrink-0 overflow-hidden rounded-md bg-secondary">
        <img
          src={item.poster}
          alt={item.title}
          className="h-full w-full object-cover"
        />
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1">
        <div className="flex items-start justify-between gap-2">
          <h3 className="truncate text-sm font-medium text-foreground">
            {item.title}
          </h3>
          {item.score && (
            <span className="shrink-0 text-xs text-muted-foreground">
              {item.score.toFixed(1)}
            </span>
          )}
        </div>

        {/* Status Badge */}
        <Badge
          variant="outline"
          className={cn("mt-1.5 gap-1 text-[10px]", config.className)}
        >
          <StatusIcon className="size-3" />
          {config.label}
        </Badge>

        {/* Progress */}
        <div className="mt-2.5 flex items-center gap-2">
          <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-secondary">
            <div
              className={cn(
                "absolute left-0 top-0 h-full transition-all",
                progressColors[item.status]
              )}
              style={{ width: `${progressPercentage}%` }}
            />
          </div>
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {item.currentProgress}/{item.totalProgress} {typeConfig.progressUnit}
          </span>
        </div>
      </div>
    </div>
  );
}