
import { cn } from "@/lib/utils";
import { 
  ActivityItem, 
  activityActionLabels, 
  formatRelativeTime 
} from "@/lib/activity-data";
import { mediaTypeConfig } from "@/lib/media-types";
import { 
  Plus, 
  CheckCircle2, 
  Play, 
  ArrowUp, 
  XCircle, 
  PauseCircle,
  Star 
} from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";

const actionIcons = {
  added: Plus,
  completed: CheckCircle2,
  started: Play,
  updated: ArrowUp,
  dropped: XCircle,
  paused: PauseCircle,
  rated: Star,
};

const actionColors = {
  added: "text-blue-400 bg-blue-400/10",
  completed: "text-completed bg-completed/10",
  started: "text-watching bg-watching/10",
  updated: "text-accent bg-accent/10",
  dropped: "text-dropped bg-dropped/10",
  paused: "text-on-hold bg-on-hold/10",
  rated: "text-yellow-400 bg-yellow-400/10",
};

interface ActivityFeedProps {
  activities: ActivityItem[];
  maxHeight?: string;
}

export function ActivityFeed({ activities, maxHeight = "320px" }: ActivityFeedProps) {
  return (
    <ScrollArea className="pr-3" style={{ height: maxHeight }}>
      <div className="space-y-1 pr-2">
        {activities.map((activity, index) => {
          const ActionIcon = actionIcons[activity.action];
          const MediaIcon = mediaTypeConfig[activity.mediaType].icon;
          const isLast = index === activities.length - 1;

          return (
            <div key={activity.id} className="relative flex gap-3 py-2">
              {/* Timeline line */}
              {!isLast && (
                <div className="absolute left-[13px] top-10 h-[calc(100%-12px)] w-px bg-border" />
              )}

              {/* Action icon */}
              <div
                className={cn(
                  "relative z-10 flex size-7 shrink-0 items-center justify-center rounded-full",
                  actionColors[activity.action]
                )}
              >
                <ActionIcon className="size-3.5" />
              </div>

              {/* Content */}
              <div className="min-w-0 flex-1 space-y-1">
                <p className="text-sm leading-snug">
                  <span className="text-muted-foreground">
                    {activityActionLabels[activity.action]}
                  </span>{" "}
                  <span className="font-medium text-foreground">
                    {activity.mediaTitle}
                  </span>
                </p>

                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <MediaIcon className="size-3" />
                  <span>{mediaTypeConfig[activity.mediaType].labelRu}</span>
                  
                  {activity.rating && (
                    <>
                      <span className="text-border">•</span>
                      <span className="flex items-center gap-0.5 text-yellow-400">
                        <Star className="size-3 fill-current" />
                        {activity.rating}/10
                      </span>
                    </>
                  )}
                  
                  {activity.progress && (
                    <>
                      <span className="text-border">•</span>
                      <span>
                        {activity.progress.current}/{activity.progress.total}
                      </span>
                    </>
                  )}

                  <span className="text-border">•</span>
                  <span>{formatRelativeTime(activity.timestamp)}</span>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </ScrollArea>
  );
}
