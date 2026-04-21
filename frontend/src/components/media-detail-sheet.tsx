import { useEffect, useState } from "react";

import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  getMediaDetails,
  type SearchResult,
  type SearchType,
  type TrackingEntry,
  updateTracking,
} from "@/api/tracking";
import { statusConfig } from "@/lib/media-types";
import { mapStatusToBackend, mapStatusToUI } from "./tracking-card";

interface MediaDetailSheetProps {
  entry: TrackingEntry | null;
  open: boolean;
  onClose: () => void;
  onUpdate: (id: number, status: string, progress?: number) => void;
}

function formatReleaseStatus(status: string | null): string {
  if (!status) {
    return "Неизвестно";
  }

  switch (status) {
    case "FINISHED":
    case "COMPLETED":
    case "ENDED":
      return "Завершён";
    case "RELEASING":
    case "ONGOING":
      return "Онгоинг";
    case "NOT_YET_RELEASED":
      return "Ещё не вышел";
    case "CANCELLED":
      return "Отменён";
    case "HIATUS":
      return "Пауза";
    default:
      return status;
  }
}

function formatScore(score: number | null): string | null {
  if (score === null || score === undefined) {
    return null;
  }

  return `${(score / 10).toFixed(1)}/10`;
}

export function MediaDetailSheet({
  entry,
  open,
  onClose,
  onUpdate,
}: MediaDetailSheetProps) {
  const [details, setDetails] = useState<SearchResult | null>(null);
  const [loadingDetails, setLoadingDetails] = useState(false);
  const [detailsError, setDetailsError] = useState("");
  const [updatingStatus, setUpdatingStatus] = useState(false);

 useEffect(() => {
  if (!entry || !open) {
    return;
  }

  const currentEntry = entry;
  const maybeExternalId = currentEntry.media.external_id;
  const mediaType = currentEntry.media.media_type as SearchType;

  if (typeof maybeExternalId !== "string" || maybeExternalId.trim() === "") {
    setDetails(null);
    setDetailsError("У этого тайтла нет external_id");
    setLoadingDetails(false);
    return;
  }

  let cancelled = false;

  async function loadDetails(
    currentMediaType: SearchType,
    externalId: string,
  ) {
    setDetails(null);
    setDetailsError("");
    setLoadingDetails(true);

    try {
      const data = (await getMediaDetails(
        currentMediaType,
        externalId,
      )) as SearchResult;

      if (!cancelled) {
        setDetails(data);
      }
    } catch (error) {
      console.error(error);

      if (!cancelled) {
        setDetailsError("Не удалось загрузить детали");
      }
    } finally {
      if (!cancelled) {
        setLoadingDetails(false);
      }
    }
  }

  void loadDetails(mediaType, maybeExternalId);

  return () => {
    cancelled = true;
  };
}, [entry, open]);

  if (!entry) {
    return null;
  }

  const uiStatus = mapStatusToUI(
    entry.status,
  ) as keyof typeof statusConfig;
  const config = statusConfig[uiStatus];

  const title =
    entry.media.title_russian ||
    entry.media.title_english ||
    entry.media.title_native ||
    entry.media.title ||
    "Без названия";

  const originalTitle = entry.media.title_russian
    ? entry.media.title_english ||
      entry.media.title_native ||
      entry.media.title
    : null;

  const description = details?.description ?? null;
  const totalEpisodes = details?.episodes ?? entry.media.episodes ?? null;
  const seasons = details?.seasons ?? null;
  const score = details?.score ?? null;
  const releaseStatus = details?.status ?? null;

  const hasMeta =
    seasons !== null ||
    totalEpisodes !== null ||
    score !== null ||
    releaseStatus !== null;

  async function handleStatusChange(
    nextUiStatus: keyof typeof statusConfig,
  ) {
    if (!entry) {
      return;
    }

    const backendStatus = mapStatusToBackend(nextUiStatus);

    setUpdatingStatus(true);

    try {
      await updateTracking(entry.id, { status: backendStatus });
      onUpdate(entry.id, backendStatus);
    } catch (error) {
      console.error(error);
    } finally {
      setUpdatingStatus(false);
    }
  }

  return (
    <Sheet open={open} onOpenChange={(value) => !value && onClose()}>
      <SheetContent
        side="right"
        className="flex w-full flex-col overflow-y-auto border-border bg-card p-0 sm:max-w-md"
      >
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
                <p className="text-sm text-muted-foreground">
                  {originalTitle}
                </p>
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

              {totalEpisodes !== null && (
                <span className="text-sm text-muted-foreground">
                  {entry.progress} / {totalEpisodes} эп.
                </span>
              )}
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-5 p-5">
          <div>
            <p className="mb-2 text-xs text-muted-foreground">Статус</p>

            <div className="flex flex-wrap gap-2">
              {Object.entries(statusConfig).map(([key, cfg]) => {
                const typedKey = key as keyof typeof statusConfig;
                const isActive = uiStatus === typedKey;

                return (
                  <button
                    key={key}
                    onClick={() => handleStatusChange(typedKey)}
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

          {hasMeta && (
            <div className="grid grid-cols-2 gap-3">
              {seasons !== null && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">
                    Сезонов
                  </p>
                  <p className="text-sm font-medium text-foreground">
                    {seasons}
                  </p>
                </div>
              )}

              {totalEpisodes !== null && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">Серий</p>
                  <p className="text-sm font-medium text-foreground">
                    {totalEpisodes}
                  </p>
                </div>
              )}

              {score !== null && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">
                    Оценка
                  </p>
                  <p className="text-sm font-medium text-foreground">
                    {formatScore(score)}
                  </p>
                </div>
              )}

              {releaseStatus !== null && (
                <div className="rounded-lg border border-border bg-background p-3">
                  <p className="mb-1 text-xs text-muted-foreground">
                    Статус релиза
                  </p>
                  <p className="text-sm font-medium text-foreground">
                    {formatReleaseStatus(releaseStatus)}
                  </p>
                </div>
              )}
            </div>
          )}

          <div>
            <p className="mb-2 text-xs text-muted-foreground">Описание</p>

            {loadingDetails ? (
              <p className="text-sm text-muted-foreground">Загрузка...</p>
            ) : detailsError ? (
              <p className="text-sm text-muted-foreground">
                {detailsError}
              </p>
            ) : description ? (
              <p className="text-sm leading-relaxed text-foreground">
                {description}
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">
                Нет описания
              </p>
            )}
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}