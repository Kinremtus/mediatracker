import { useMemo } from "react";
import { 
  BarChart3, 
  Clock, 
  TrendingUp,
  Award,
  Target,
  Flame
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { ActivityGrid } from "@/components/activity-grid";
import { mediaTypeConfig, statusConfig, type MediaStatus, type MediaType } from "@/lib/media-types";
import { cn } from "@/lib/utils";
import type { TrackingEntry } from "@/api/tracking";

interface StatisticsSectionProps {
  tracking: TrackingEntry[];
}

const mapStatusToUI = (backendStatus: string): MediaStatus => {
  switch (backendStatus) {
    case "in_progress": return "watching";
    case "completed": return "completed";
    case "planned": return "plan-to-watch";
    case "dropped": return "dropped";
    default: return "plan-to-watch";
  }
};

export function StatisticsSection({ tracking }: StatisticsSectionProps) {
  const safeTracking = tracking ||[];

  const stats = useMemo(() => {
    const statusCounts: Record<MediaStatus, number> = {
      watching: 0,
      completed: 0,
      dropped: 0,
      "plan-to-watch": 0,
    };
    
    const typeCounts: Record<string, number> = {};

    safeTracking.forEach(entry => {
      const uiStatus = mapStatusToUI(entry.status);
      if (statusCounts[uiStatus] !== undefined) {
        statusCounts[uiStatus]++;
      }

      const type = (entry.media.media_type as MediaType) || "anime";
      typeCounts[type] = (typeCounts[type] || 0) + 1;
    });

    const mostActiveType = Object.entries(typeCounts)
      .sort(([,a], [,b]) => b - a)[0];

    const totalItems = safeTracking.length;
    const completedItems = statusCounts.completed;
    const overallProgress = totalItems > 0 ? Math.round((completedItems / totalItems) * 100) : 0;

    return { statusCounts, overallProgress, totalItems, mostActiveType };
  },[safeTracking]);

  // ИСПРАВЛЕНИЕ ОШИБКИ: Удалили "on-hold"
  const statuses: MediaStatus[] = ["watching", "completed", "dropped", "plan-to-watch"];

  return (
    <div className="space-y-6">
      const activityData = useMemo(() => {
        const counts: Record<string, number> = {};
        safeTracking.forEach(entry => {
          if (entry.created_at) {
            const day = entry.created_at.split("T")[0]; // "2026-04-09"
            counts[day] = (counts[day] || 0) + 1;
          }
        });
        return counts;
      },[safeTracking]);

      <ActivityGrid data={{}} />

     {/* Header Stats */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        
        {/* Карточка 1: Всего тайтлов (Пыльный океан) */}
        <Card className="border-border bg-card">
          <CardContent className="flex items-center gap-4 p-4">
            <div className="flex size-10 items-center justify-center rounded-lg bg-planned/10 text-planned">
              <BarChart3 className="size-5" />
            </div>
            <div>
              <p className="text-2xl font-bold text-foreground">{stats.totalItems}</p>
              <p className="text-xs text-muted-foreground">Всего тайтлов</p>
            </div>
          </CardContent>
        </Card>

        {/* Карточка 2: Доля завершенных (Шалфей) */}
        <Card className="border-border bg-card">
          <CardContent className="flex items-center gap-4 p-4">
            <div className="flex size-10 items-center justify-center rounded-lg bg-completed/10 text-completed">
              <Target className="size-5" />
            </div>
            <div>
              <p className="text-2xl font-bold text-foreground">{stats.overallProgress}%</p>
              <p className="text-xs text-muted-foreground">Доля завершенных</p>
            </div>
          </CardContent>
        </Card>

        {/* Карточка 3: Топ категория (Охристая глина) */}
        <Card className="border-border bg-card">
          <CardContent className="flex items-center gap-4 p-4">
            <div className="flex size-10 items-center justify-center rounded-lg bg-watching/10 text-watching">
              <Flame className="size-5" />
            </div>
            <div>
              <p className="truncate text-lg font-bold text-foreground">
                {stats.mostActiveType ? mediaTypeConfig[stats.mostActiveType[0] as MediaType]?.labelRu || stats.mostActiveType[0] : "—"}
              </p>
              <p className="text-xs text-muted-foreground">Топ категория</p>
            </div>
          </CardContent>
        </Card>

        {/* Карточка 4: Время (Графитовый туман) */}
        <Card className="border-border bg-card">
          <CardContent className="flex items-center gap-4 p-4">
            <div className="flex size-10 items-center justify-center rounded-lg bg-dropped/10 text-dropped">
              <Clock className="size-5" />
            </div>
            <div>
              <p className="text-2xl font-bold text-foreground">0ч</p>
              <p className="text-xs text-muted-foreground">Время (нужен бэк)</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <Card className="border-border bg-card">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-sm font-medium">
              <TrendingUp className="size-4 text-muted-foreground" />
              По статусу
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {statuses.map(status => {
              const config = statusConfig[status];
              const Icon = config.icon;
              const count = stats.statusCounts[status];
              const percentage = stats.totalItems > 0 ? Math.round((count / stats.totalItems) * 100) : 0;
              
              // ХИТРОСТЬ: Вытаскиваем класс цвета текста (например, "text-watching") из конфига
              const textColorClass = config.className.split(" ").find(c => c.startsWith("text-"));

              return (
                <div key={status} className="space-y-2">
                  <div className="flex items-center justify-between text-sm">
                    <div className="flex items-center gap-2">
                      {/* Применяем цвет к иконке */}
                      <Icon className={cn("size-4", textColorClass)} />
                      <span className="text-foreground">{config.label}</span>
                    </div>
                    <span className="text-muted-foreground">{count}</span>
                  </div>
                  {/* Передаем цвет в прогресс-бар через bg-current */}
                  <Progress 
                    value={percentage} 
                    className={cn("h-1.5 bg-secondary [&>div]:bg-current", textColorClass)}
                  />
                </div>
              );
            })}
          </CardContent>
        </Card>

        <Card className="border-border bg-card lg:col-span-2">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-sm font-medium">
              <Award className="size-4 text-muted-foreground" />
              Прогресс по произведениям (В разработке)
            </CardTitle>
          </CardHeader>
          <CardContent className="flex h-40 items-center justify-center text-sm text-muted-foreground">
            Бэкенд пока не отдает данные о количестве просмотренных серий/глав.
          </CardContent>
        </Card>
      </div>

      <Card className="border-border bg-card">
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-sm font-medium">
            <Clock className="size-4 text-muted-foreground" />
            Полная история активности (В разработке)
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex h-32 items-center justify-center text-sm text-muted-foreground">
            Бэкенд пока не логирует историю изменений статусов.
          </div>
        </CardContent>
      </Card>
    </div>
  );
}