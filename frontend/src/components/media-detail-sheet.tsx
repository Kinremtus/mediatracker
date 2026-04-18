import { useEffect, useState } from "react";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { type TrackingEntry, getMediaDetails, updateTracking } from "@/api/tracking";
import { mapStatusToUI, mapStatusToBackend } from "./tracking-card";
import { statusConfig } from "@/lib/media-types";

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
  const [updatingStatus, setUpdatingStatus] = useState(false);

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
  const originalTitle = entry.media.title_russian ? entry.media.title : null;

  const description = details?.description || null;
  const totalEpisodes = details?.episodes ?? entry.media.episodes ?? null;
  const seasons = details?.seasons ?? null;
  const score = details?.score ?? null;
  const releaseStatus = details?.status ?? null;

  async function handleStatusChange(nextUiStatus: string) {
    if (!entry) return;

    const backendStatus = mapStatusToBackend(nextUiStatus);

    setUpdatingStatus(true);
    try {
      await updateTracking(entry.id, { status: backendStatus });
      onUpdate(entry.id, backendStatus);
    } catch (e) {
      console.error(e);
    } finally {
      setUpdatingStatus(false);
    }
  }

  return (
    <Sheet open={open} onOpenChange={(v) => !v && onClose()}>
      <SheetContent
        side="right"
        className="flex w-full flex-col overflow-y-auto border-border bg-card p-0 sm:max-w-md"
      >
        {/* Header */}
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
                }}
              >
                {config.label}
              </span>

              {totalEpisodes && (
                <span className="text-sm text-muted-foreground">
                  {entry.progress} / {totalEpisodes} эп.
                </span>
              )}
            </div>
          </div>
        </div>

        {/* Body */}
        <div className="flex flex-col gap-5 p-5">
          {/* Status buttons */}
          <div>
            <p className="mb-2 text-xs text-muted-foreground">Статус</p>
            <div className="flex flex-wrap gap-2">
              {Object.entries(statusConfig).map(([key, cfg]) => {
                const isActive = uiStatus === key;

                return (
                  <button
                    key={key}
                    onClick={() => handleStatusChange(key)}
                    disabled={updatingStatus}
                    className={`rounded-full border px-3 py-1 text-xs transition-all ${
                      isActive
                        ? "border-foreground bg-foreground text-background"
                        : "border-border bg-card text-muted-foreground hover:border-muted-foreground"
                    } ${updatingStatus ? "opacity-60" : ""}`}
                  >
                    {cfg.label}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Meta cards */}
          {(seasons || totalEpisodes || score || releaseStatus) && (
            <div className="grid grid-cols-2 gap-3">
              {seasons && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">Сезонов</p>
                  <p className="text-sm font-medium text-foreground">{seasons}</p>
                </div>
              )}

              {totalEpisodes && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">Серий</p>
                  <p className="text-sm font-medium text-foreground">
                    {totalEpisodes}
                  </p>
                </div>
              )}

              {score && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">Оценка</p>
                  <p className="text-sm font-medium text-foreground">
                    {score / 10}/10
                  </p>
                </div>
              )}

              {releaseStatus && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">Статус релиза</p>
                  <p className="text-sm font-medium text-foreground">
                    {releaseStatus === "FINISHED"
                      ? "Завершён"
                      : releaseStatus === "RELEASING"
                      ? "Онгоинг"
                      : releaseStatus === "ONGOING"
                      ? "Онгоинг"
                      : releaseStatus}
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Description */}
          <div>
            <p className="mb-2 text-xs text-muted-foreground">Описание</p>
            {loadingDetails ? (
              <p className="text-sm text-muted-foreground">Загрузка...</p>
            ) : description ? (
              <p className="text-sm leading-relaxed text-foreground">
                {description}
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">Нет описания</p>
            )}
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}