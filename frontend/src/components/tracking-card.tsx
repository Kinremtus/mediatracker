import { useState } from "react";
import { statusConfig, type MediaStatus, type MediaType } from "@/lib/media-types";
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

import { UnifiedMediaCard } from "./unified-media-card";

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
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const uiStatus = mapStatusToUI(entry.status);

  async function handleStatusChange(status: string) {
    setLoading(true);
    try {
      await updateTracking(entry.id, { status: mapStatusToBackend(status) });
      onUpdate(entry.id, mapStatusToBackend(status));
    } catch (e) { console.error(e); }
    finally { setLoading(false); }
  }

  async function handleProgressChange(newProgress: number) {
    try {
      await updateTracking(entry.id, { progress: newProgress });
      onUpdate(entry.id, entry.status, newProgress);
    } catch (e) { console.error(e); }
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
    <UnifiedMediaCard
      variant="tracking"
      title={entry.media.title_russian || entry.media.title}
      posterUrl={entry.media.poster_url}
      mediaType={entry.media.media_type as MediaType}
      episodes={entry.media.episodes}
      progress={entry.progress}
      status={uiStatus}
      onPosterClick={onPosterClick}
      onDelete={handleDelete}
      onStatusChange={(s) => handleStatusChange(s)}
      onProgressChange={handleProgressChange}
      isDeleting={deleting}
      isLoading={loading}
    />
  );
}