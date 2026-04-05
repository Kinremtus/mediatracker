import { api } from "./client";

export interface MediaItem {
  id: number;
  title: string;
  title_english: string | null;
  title_native: string | null;
  title_russian: string | null;
  media_type: string;
  poster_url: string | null;
  episodes: number | null;
}

export interface TrackingEntry {
  id: number;
  status: string;
  rating: number | null;
  progress: number;
  media: MediaItem;
}

export interface AnimeSearchResult {
  anilist_id: number;
  title_romaji: string;
  title_english: string | null;
  title_native: string | null;
  title_russian: string | null;
  poster_url: string;
  episodes: number | null;
  status: string;
  score: number | null;
}

export async function getTracking(status?: string): Promise<TrackingEntry[]> {
  const params = status ? `?status=${status}` : "";
  return api.get<TrackingEntry[]>(`/tracking${params}`);
}

export async function searchAnime(query: string): Promise<AnimeSearchResult[]> {
  return api.get<AnimeSearchResult[]>(
    `/search/anime?q=${encodeURIComponent(query)}`
  );
}

export async function addToTracking(
  anilistId: number,
  status = "planned"
): Promise<TrackingEntry> {
  return api.post<TrackingEntry>("/tracking/from-search", {
    anilist_id: anilistId,
    status,
  });
}

export async function updateTracking(
  id: number,
  data: { status?: string; rating?: number | null; progress?: number }
): Promise<TrackingEntry> {
  return api.put<TrackingEntry>(`/tracking/${id}`, data);
}

export async function deleteTracking(id: number): Promise<void> {
  return api.delete(`/tracking/${id}`);
}