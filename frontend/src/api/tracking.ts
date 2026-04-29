import { api } from "./client";

export type SearchType =
  | "anime"
  | "manga"
  | "manhwa"
  | "manhua"
  | "novels"
  | "movies"
  | "tv"
  | "dramas"
  | "cartoons"
  | "animated-movies"
  | "games"
  | "books"
  | "other-comics";

export interface MediaItem {
  id: number;
  external_id: string | null;
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
  created_at: string;
  media: MediaItem;
}

export interface SearchResult {
  external_id: string;
  title: string;
  title_english: string | null;
  title_native: string | null;
  title_russian: string | null;
  poster_url: string | null;
  media_type: SearchType;
  episodes: number | null;
  seasons: number | null;
  status: string | null;
  score: number | null;
  description: string | null;
  provider: string | null;
}

export interface UpdateTrackingPayload {
  status?: string;
  rating?: number | null;
  progress?: number;
}

const SEARCH_ENDPOINTS: Record<SearchType, string> = {
  anime: "/search/anime",
  manga: "/search/manga",
  manhwa: "/search/manhwa",
  manhua: "/search/manhua",
  novels: "/search/novels",
  movies: "/search/movies",
  tv: "/search/tv",
  dramas: "/search/dramas",
  cartoons: "/search/cartoons",
  "animated-movies": "/search/animated-movies",
  games: "/search/games",
  books: "/search/books",
  "other-comics": "/search/other-comics",
};

export async function getTracking(
  status?: string,
): Promise<TrackingEntry[]> {
  const params = status
    ? `?status=${encodeURIComponent(status)}`
    : "";

  return api.get<TrackingEntry[]>(`/tracking${params}`);
}

export async function searchMedia(
  query: string,
  type: SearchType,
): Promise<SearchResult[]> {
  return api.get<SearchResult[]>(
    `${SEARCH_ENDPOINTS[type]}?q=${encodeURIComponent(query)}`,
  );
}

export async function addToTracking(
  externalId: string,
  mediaType: SearchType,
  status = "planned",
  provider?: string,
): Promise<TrackingEntry> {
  return api.post<TrackingEntry>("/tracking/from-search", {
    external_id: externalId,
    media_type: mediaType,
    status,
    provider,
  });
}

export async function updateTracking(
  id: number,
  data: UpdateTrackingPayload,
): Promise<TrackingEntry> {
  return api.put<TrackingEntry>(`/tracking/${id}`, data);
}

export async function deleteTracking(id: number): Promise<void> {
  return api.delete(`/tracking/${id}`);
}

export async function getMediaDetails(
  mediaType: SearchType,
  externalId: string,
): Promise<SearchResult> {
  const params =
    `media_type=${encodeURIComponent(mediaType)}` +
    `&external_id=${encodeURIComponent(externalId)}`;

  return api.get<SearchResult>(`/search/details?${params}`);
}