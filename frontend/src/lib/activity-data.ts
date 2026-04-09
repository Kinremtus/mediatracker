import { type MediaType, type MediaStatus } from "./media-types";

export type ActivityAction = 
  | "added"
  | "completed"
  | "started"
  | "updated"
  | "dropped"
  | "paused"
  | "rated";

export interface ActivityItem {
  id: number;
  action: ActivityAction;
  mediaTitle: string;
  mediaType: MediaType;
  timestamp: Date;
  details?: string;
  rating?: number;
  progress?: { current: number; total: number };
}

export const activityActionLabels: Record<ActivityAction, string> = {
  added: "добавил(а) в список",
  completed: "завершил(а)",
  started: "начал(а) смотреть",
  updated: "обновил(а) прогресс",
  dropped: "дропнул(а)",
  paused: "поставил(а) на паузу",
  rated: "оценил(а)",
};

export const sampleActivityData: ActivityItem[] = [
  {
    id: 1,
    action: "completed",
    mediaTitle: "Атака титанов: Финал",
    mediaType: "anime",
    timestamp: new Date("2026-04-04T08:30:00"),
    rating: 10,
  },
  {
    id: 2,
    action: "updated",
    mediaTitle: "Магическая битва",
    mediaType: "anime",
    timestamp: new Date("2026-04-04T07:15:00"),
    progress: { current: 18, total: 24 },
  },
  {
    id: 3,
    action: "started",
    mediaTitle: "Дюна: Часть 2",
    mediaType: "movies",
    timestamp: new Date("2026-04-03T22:00:00"),
  },
  {
    id: 4,
    action: "rated",
    mediaTitle: "Сквозь снег",
    mediaType: "tv-shows",
    timestamp: new Date("2026-04-03T20:30:00"),
    rating: 8,
  },
  {
    id: 5,
    action: "added",
    mediaTitle: "Берсерк",
    mediaType: "manga",
    timestamp: new Date("2026-04-03T18:45:00"),
  },
  {
    id: 6,
    action: "updated",
    mediaTitle: "Solo Leveling",
    mediaType: "manhwa",
    timestamp: new Date("2026-04-03T16:20:00"),
    progress: { current: 120, total: 179 },
  },
  {
    id: 7,
    action: "completed",
    mediaTitle: "Elden Ring",
    mediaType: "games",
    timestamp: new Date("2026-04-03T14:00:00"),
    rating: 10,
  },
  {
    id: 8,
    action: "paused",
    mediaTitle: "Доктор Стрэндж",
    mediaType: "dramas",
    timestamp: new Date("2026-04-03T12:30:00"),
  },
  {
    id: 9,
    action: "started",
    mediaTitle: "Проект Хейл Мэри",
    mediaType: "books",
    timestamp: new Date("2026-04-02T21:00:00"),
  },
  {
    id: 10,
    action: "dropped",
    mediaTitle: "Властелин Духов",
    mediaType: "manhua",
    timestamp: new Date("2026-04-02T19:30:00"),
  },
  {
    id: 11,
    action: "completed",
    mediaTitle: "Аркейн",
    mediaType: "cartoons",
    timestamp: new Date("2026-04-02T15:00:00"),
    rating: 10,
  },
  {
    id: 12,
    action: "added",
    mediaTitle: "Sword Art Online Progressive",
    mediaType: "novels",
    timestamp: new Date("2026-04-02T12:00:00"),
  },
];

export function formatRelativeTime(date: Date): string {
  const now = new Date();
  const diffInSeconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  
  if (diffInSeconds < 60) return "только что";
  if (diffInSeconds < 3600) return `${Math.floor(diffInSeconds / 60)} мин. назад`;
  if (diffInSeconds < 86400) return `${Math.floor(diffInSeconds / 3600)} ч. назад`;
  if (diffInSeconds < 604800) return `${Math.floor(diffInSeconds / 86400)} дн. назад`;
  
  return date.toLocaleDateString("ru-RU", { day: "numeric", month: "short" });
}
