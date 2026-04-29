import {
  Play,
  CheckCircle2,
  XCircle,
  Clock,
  Film,
  Tv,
  Book,
  Gamepad2,
  Clapperboard,
  BookOpen,
  type LucideIcon,
} from "lucide-react";

// Удалили "on-hold" из типов
export type MediaStatus =
  | "watching"
  | "completed"
  | "dropped"
  | "plan-to-watch";

export type MediaType =
  | "anime"
  | "movies"
  | "tv-shows"
  | "books"
  | "manga"
  | "manhwa"
  | "manhua"
  | "games"
  | "dramas"
  | "cartoons"
  | "animated-movies"
  | "novels"
  | "other-comics";

export interface MediaItem {
  id: number;
  title: string;
  poster: string;
  status: MediaStatus;
  currentProgress: number;
  totalProgress: number;
  score?: number;
  type: MediaType;
}

export const statusConfig: Record<
  MediaStatus,
  { label: string; icon: LucideIcon; className: string }
> = {
  watching: {
    label: "Смотрю",
    icon: Play,
    // Используем классы, созданные в index.css
    className: "bg-watching/10 text-watching border-watching/30",
  },
  completed: {
    label: "Просмотрено",
    icon: CheckCircle2,
    className: "bg-completed/10 text-completed border-completed/30",
  },
  dropped: {
    label: "Дропнул",
    icon: XCircle,
    className: "bg-dropped/10 text-dropped border-dropped/30",
  },
  "plan-to-watch": {
    label: "Запланировано",
    icon: Clock,
    className: "bg-planned/10 text-planned border-planned/30",
  },
};

export const progressColors: Record<MediaStatus, string> = {
  watching: "bg-watching",
  completed: "bg-completed",
  dropped: "bg-dropped",
  "plan-to-watch": "bg-planned",
};


export const mediaTypeConfig: Record<
  MediaType,
  { label: string; labelRu: string; icon: LucideIcon; progressUnit: string }
> = {
  anime: {
    label: "Anime",
    labelRu: "Аниме",
    icon: Play,
    progressUnit: "ep",
  },
  movies: {
    label: "Movies",
    labelRu: "Фильмы",
    icon: Film,
    progressUnit: "min",
  },
  "tv-shows": {
    label: "TV Shows",
    labelRu: "Сериалы",
    icon: Tv,
    progressUnit: "ep",
  },
  books: {
    label: "Books",
    labelRu: "Книги",
    icon: Book,
    progressUnit: "pg",
  },
  manga: {
    label: "Manga",
    labelRu: "Манга",
    icon: BookOpen,
    progressUnit: "ch",
  },
  manhwa: {
    label: "Manhwa",
    labelRu: "Манхва",
    icon: BookOpen,
    progressUnit: "ch",
  },
  manhua: {
    label: "Manhua",
    labelRu: "Маньхуа",
    icon: BookOpen,
    progressUnit: "ch",
  },
  games: {
    label: "Games",
    labelRu: "Игры",
    icon: Gamepad2,
    progressUnit: "hr",
  },
  dramas: {
    label: "Dramas",
    labelRu: "Дорамы",
    icon: Clapperboard,
    progressUnit: "ep",
  },
  cartoons: {
    label: "Cartoons",
    labelRu: "Мультсериалы",
    icon: Tv,
    progressUnit: "ep",
  },
  "animated-movies": {
    label: "Animated Movies",
    labelRu: "Мультфильмы",
    icon: Film,
    progressUnit: "min",
  },
  novels: {
    label: "Light Novels",
    labelRu: "Новеллы",
    icon: BookOpen,
    progressUnit: "ch",
  },
  "other-comics": {
    label: "Other Comics",
    labelRu: "Другие комиксы",
    icon: BookOpen,
    progressUnit: "ch",
  },
};

export const mediaTypes: MediaType[] = [
  "anime",           // Аниме
  "dramas",          // Дорамы
  "games",           // Игры
  "books",           // Книги
  "manga",           // Манга
  "manhwa",          // Манхва
  "manhua",          // Маньхуа
  "other-comics",    // Другие комиксы
  "cartoons",        // Мультсериалы
  "animated-movies", // Мультфильмы
  "novels",          // Новеллы
  "tv-shows",        // Сериалы
  "movies",          // Фильмы
];


export function getProgressLabel(type: MediaType): string {
  const unit = mediaTypeConfig[type].progressUnit;
  switch (unit) {
    case "ep":
      return "эп.";
    case "ch":
      return "гл.";
    case "pg":
      return "стр.";
    case "hr":
      return "ч.";
    case "min":
      return "мин.";
    default:
      return "";
  }
}
