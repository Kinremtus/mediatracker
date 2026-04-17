import { useEffect, useState } from "react";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { type TrackingEntry } from "@/api/tracking";
import { getMediaDetails } from "@/api/tracking";
import { mapStatusToUI, mapStatusToBackend } from "./tracking-card";
import { statusConfig } from "@/lib/media-types";
import { updateTracking } from "@/api/tracking";

interface MediaDetailSheetProps {
  entry: TrackingEntry | null;
  open: boolean;
  onClose: () => void;
  onUpdate: (id: number, status: string, progress?: number) => void;
}

export function MediaDetailSheet({
  entry,
  open,
  onClose,
  onUpdate,
}: MediaDetailSheetProps) {
  const [details, setDetails] = useState<any>(null);
  const [loadingDetails, setLoadingDetails] = useState(false);

  useEffect(() => {
    if (!entry || !open) return;
    setDetails(null);
    setLoadingDetails(true);
    getMediaDetails(entry.media.media_type, entry.media.external_id ?? "")
      .then(setDetails)
      .catch(console.error)
      .finally(() => setLoadingDetails(false));
  }, [entry, open]);

  if (!entry) return null;

  const uiStatus = mapStatusToUI(entry.status);
  const config = statusConfig[uiStatus];

  const title = entry.media.title_russian || entry.media.title;
  const originalTitle =
    entry.media.title_russian ? entry.media.title : null;

  const description = details?.description;

  return (
    <Sheet open={open} onOpenChange={(v) => !v && onClose()}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-md p-0 bg-card border-border flex flex-col overflow-y-auto"
      >
        {/* Постер */}
        <div className="flex gap-4 border-b border-border p-5">
          <img
            src={entry.media.poster_url || "/placeholder.jpg"}
            alt={title}
            className="h-36 w-24 shrink-0 rounded-lg object-cover"
          />

          <div className="min-w-0 flex-1">
            <SheetHeader className="space-y-1 text-left">
              <SheetTitle className="text-xl font-bold leading-tight text-foreground">
                {title}
              </SheetTitle>

              {originalTitle && (
                <p className="text-sm text-muted-foreground">{originalTitle}</p>  
              )}
            </SheetHeader>
          </div>

          {/* Градиент снизу */}
          <div className="absolute inset-0 bg-gradient-to-t from-card via-card/40 to-transparent" />

          {/* Заголовок поверх постера */}
          <div className="absolute bottom-0 left-0 right-0 p-5">
            <SheetHeader className="text-left space-y-1">
              <SheetTitle className="text-xl font-bold text-foreground leading-tight">
                {title}
              </SheetTitle>
              {originalTitle && (
                <p className="text-sm text-muted-foreground">{originalTitle}</p>
              )}
            </SheetHeader>
          </div>
        </div>

        {/* Контент */}
        <div className="flex flex-col gap-5 p-5">

          {/* Статус + прогресс */}
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <span
              className="rounded-full border px-3 py-1 text-sm font-medium"
              style={{
                color: 
                  uiStatus === "watching"
                    ? "var(--watching)"
                    : uiStatus === "completed"
                    ? "var(--completed)"
                    : uiStatus === "dropped"
                    ? "var(--dropped)"
                    : "var(--planned)",
                borderColor:
                  uiStatus === "watching"
                    ? "var(--watching)"
                    : uiStatus === "completed"
                    ? "var(--completed)"
                    : uiStatus === "dropped"
                    ? "var(--dropped)"
                    : "var(--planned)",
                backgroundColor: "transparent",
              }}
            >
              {config.label}
            </span>

            {entry.media.episodes && (
              <span className="text-sm text-muted-foreground">
                {entry.progress} / {entry.media.episodes} эп.
              </span>
            )}
          </div>
          
          <div className="grid grid-cols-2 gap-3">
            {details?.seasons && (
              <div className="rounded-lg border border-border bg-background p-3">
                <p className="mb-1 text-xs text-muted-foreground">Сезонов</p>
                <p className="text-sm font-medium text-foreground">{details.seasons}</p>
              </div>
            )}

            {details?.episodes && (
              <div className="rounded-lg border border-border bg-background p-3">
                <p className="mb-1 text-xs text-muted-foreground">Серий</p>
                <p className="text-sm font-medium text-foreground">{details.episodes}</p>
              </div>
            )}

            {details?.score && (
              <div className="rounded-lg border border-border bg-background p-3">
                <p className="mb-1 text-xs text-muted-foreground">Оценка</p>
                <p className="text-sm font-medium text-foreground">
                  {details.score / 10}/10
                </p>
              </div>
            )}
          </div>

          <div>
            <p className="mb-2 text-xs text-muted-foreground">Описание</p>
            {loadingDetails ? (
              <p className="text-sm text-muted-foreground">Загрузка...</p>
            ) : details?.description ? (
              <p className="text-sm leading-relaxed text-foreground">
                {details.description}
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">Нет описания</p>
            )}
          </div>    

          {/* Смена статуса */}
          <div>
            <p className="text-xs text-muted-foreground mb-2">Статус</p>
            <div className="flex flex-wrap gap-2">
              {Object.entries(statusConfig).map(([key, cfg]) => {
                const isActive = uiStatus === key;
                return (
                  <button
                    key={key}
                    onClick={async () => {
                      await updateTracking(entry.id, {
                        status: mapStatusToBackend(key),
                      });
                      onUpdate(entry.id, mapStatusToBackend(key));
                    }}
                    className={`px-3 py-1 rounded-full text-xs border transition-all ${
                      isActive
                        ? "bg-foreground text-background border-foreground"
                        : "bg-card text-muted-foreground border-border hover:border-muted-foreground"
                    }`}
                  >
                    {cfg.label}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Описание */}
          <div>
            <p className="text-xs text-muted-foreground mb-2">Описание</p>
            {loadingDetails ? (
              <p className="text-sm text-muted-foreground">Загрузка...</p>
            ) : description ? (
              <p className="text-sm text-foreground leading-relaxed line-clamp-6">
                {description}
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">Нет описания</p>
            )}
          </div>

          {/* Доп. инфо */}
          {details && (
            <div className="grid grid-cols-2 gap-3">
              {details.episodes && (
                <div className="rounded-lg bg-background border border-border p-3">
                  <p className="text-xs text-muted-foreground mb-1">Эпизодов</p>
                  <p className="text-sm font-medium text-foreground">
                    {details.episodes}
                  </p>
                </div>
              )}
              {details.score && (
                <div className="rounded-lg bg-background border border-border p-3">
                  <p className="text-xs text-muted-foreground mb-1">Оценка</p>
                  <p className="text-sm font-medium text-foreground">
                    {details.score / 10}/10
                  </p>
                </div>
              )}
              {details.status && (
                <div className="rounded-lg bg-background border border-border p-3">
                  <p className="text-xs text-muted-foreground mb-1">Выход</p>
                  <p className="text-sm font-medium text-foreground">
                    {details.status === "FINISHED" ? "Завершён" :
                     details.status === "RELEASING" ? "Онгоинг" :
                     details.status}
                  </p>
                </div>
              )}
            </div>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}