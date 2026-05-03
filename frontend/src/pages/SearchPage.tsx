import { useState, type FormEvent } from "react";
import { ArrowLeft, Search } from "lucide-react";

import {
  addToTracking,
  searchMedia,
  type SearchResult,
  type SearchType,
} from "@/api/tracking";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { UnifiedMediaCard } from "@/components/unified-media-card";
import { type MediaType } from "@/lib/media-types";

const SEARCH_TYPES: { value: SearchType; label: string; emoji: string }[] = [
  { value: "anime", label: "Аниме", emoji: "▶" },
  { value: "dramas", label: "Дорамы", emoji: "🎬" },
  { value: "games", label: "Игры", emoji: "🎮" },
  { value: "books", label: "Книги", emoji: "📖" },
  { value: "manga", label: "Манга", emoji: "📚" },
  { value: "manhwa", label: "Манхва", emoji: "📚" },
  { value: "manhua", label: "Маньхуа", emoji: "📚" },
  { value: "other-comics", label: "Другие комиксы", emoji: "📚" },
  { value: "cartoons", label: "Мультсериалы", emoji: "📺" },
  { value: "animated-movies", label: "Мультфильмы", emoji: "🎞" },
  { value: "novels", label: "Новеллы", emoji: "📝" },
  { value: "tv", label: "Сериалы", emoji: "📺" },
  { value: "movies", label: "Фильмы", emoji: "🎥" },
];

function getErrorMessage(error: unknown): string {
  const err = error as {
    response?: {
      data?: {
        detail?: unknown;
      };
    };
    message?: string;
  };

  const detail = err.response?.data?.detail;

  if (typeof detail === "string" && detail) {
    return detail;
  }

  if (Array.isArray(detail)) {
    return detail
      .map((item) =>
        typeof item?.msg === "string" ? item.msg : JSON.stringify(item),
      )
      .join("; ");
  }

  if (typeof err.message === "string" && err.message) {
    return err.message;
  }

  return "Не удалось выполнить запрос";
}

export default function SearchPage({
  onBack,
  initialType = "anime",
}: {
  onBack: () => void;
  initialType?: SearchType;
}) {
  const [query, setQuery] = useState("");
  const [searchType, setSearchType] = useState<SearchType>(initialType);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [added, setAdded] = useState(new Set<string>());
  const [error, setError] = useState("");

  const currentType = SEARCH_TYPES.find((t) => t.value === searchType);

  async function handleSearch(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();

    if (!query.trim()) {
      return;
    }

    setLoading(true);
    setError("");

    try {
      const data = await searchMedia(query, searchType);
      setResults(data);
    } catch (e) {
      setError(getErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleAdd(item: SearchResult) {
    const externalId = item.external_id;
    const type = (item.media_type || searchType) as SearchType;
    const itemKey = `${type}:${externalId}`;

    if (!externalId) {
      setError("У этого результата поиска нет external_id");
      return;
    }

    setAdding(itemKey);
    setError("");

    try {
      await addToTracking(externalId, type, "planned", item.provider);
      setAdded((prev) => new Set(prev).add(itemKey));
    } catch (e) {
      const msg = getErrorMessage(e);

      if (msg.includes("Уже в трекинге")) {
        setAdded((prev) => new Set(prev).add(itemKey));
      } else {
        setError(msg);
      }
    } finally {
      setAdding(null);
    }
  }

  return (
    <div className="min-h-screen bg-background text-foreground flex flex-col">
      <div className="border-b border-border px-6 py-4 flex items-center gap-3">
        <button
          onClick={onBack}
          className="flex items-center gap-2 text-muted-foreground hover:text-foreground transition-colors text-sm"
        >
          <ArrowLeft className="size-4" />
          Назад
        </button>

        <div className="h-4 w-px bg-border" />

        <h1 className="text-lg font-semibold">Поиск</h1>

        {currentType && (
          <span className="text-sm text-muted-foreground">
            — {currentType.label}
          </span>
        )}
      </div>

      <div className="flex-1 max-w-5xl mx-auto w-full px-6 py-8 flex flex-col gap-6">
        <div className="flex flex-wrap gap-2">
          {SEARCH_TYPES.map((t) => (
            <button
              key={t.value}
              onClick={() => {
                setSearchType(t.value);
                setResults([]);
                setQuery("");
                setError("");
              }}
              className={cn(
                "px-4 py-2 rounded-xl text-sm font-medium transition-all border",
                searchType === t.value
                  ? "bg-foreground text-background border-foreground"
                  : "bg-card text-muted-foreground border-border hover:border-muted-foreground hover:text-foreground",
              )}
            >
              {t.label}
            </button>
          ))}
        </div>

        <form onSubmit={handleSearch} className="flex gap-3">
          <div className="relative flex-1">
            <Search className="absolute left-4 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
            <Input
              placeholder={`Поиск ${currentType?.label.toLowerCase() ?? ""}...`}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="pl-11 h-12 bg-card border-border rounded-xl text-foreground placeholder:text-muted-foreground"
            />
          </div>

          <Button
            type="submit"
            disabled={loading}
            className="h-12 px-8 rounded-xl font-semibold"
          >
            {loading ? "Ищем..." : "Найти"}
          </Button>
        </form>

        {error && <p className="text-destructive text-sm">{error}</p>}

        {results.length > 0 && (
          <div className="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
            {results.map((item) => {
              const externalId = item.external_id;
              const type = (item.media_type || searchType) as SearchType;
              const itemKey = `${type}:${externalId}`;
              const title =
                item.title_russian ||
                item.title_english ||
                item.title_native ||
                item.title ||
                "Без названия";

              return (
                <UnifiedMediaCard
                  key={itemKey}
                  variant="search"
                  title={title}
                  posterUrl={item.poster_url}
                  mediaType={type as MediaType}
                  episodes={item.episodes}
                  onAdd={() => handleAdd(item)}
                  isAdding={adding === itemKey}
                  isAdded={added.has(itemKey)}
                />
              );
            })}
          </div>
        )}

        {results.length === 0 && !loading && !error && (
          <div className="flex-1 flex flex-col items-center justify-center py-20 text-center">
            <div className="text-6xl mb-4 opacity-20">🔍</div>
            <p className="text-muted-foreground">
              Введите название и нажмите «Найти»
            </p>
          </div>
        )}
      </div>
    </div>
  );
}