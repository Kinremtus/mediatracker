import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { searchMedia, addToTracking, type SearchType } from "@/api/tracking";
import { ArrowLeft, Search } from "lucide-react";
import { cn } from "@/lib/utils";

const SEARCH_TYPES: { value: SearchType; label: string; emoji: string }[] = [
  { value: "anime", label: "Аниме", emoji: "▶" },
  { value: "dramas", label: "Дорамы", emoji: "🎬" },
  { value: "games", label: "Игры", emoji: "🎮" },
  { value: "books", label: "Книги", emoji: "📖" },
  { value: "manga", label: "Манга", emoji: "📚" },
  { value: "manhwa", label: "Манхва", emoji: "📚" },
  { value: "manhua", label: "Маньхуа", emoji: "📚" },
  { value: "cartoons", label: "Мультсериалы", emoji: "📺" },
  { value: "animated-movies", label: "Мультфильмы", emoji: "🎞" },
  { value: "novels", label: "Новеллы", emoji: "📝" },
  { value: "tv", label: "Сериалы", emoji: "📺" },
  { value: "movies", label: "Фильмы", emoji: "🎥" },
];

export default function SearchPage({
  onBack,
  initialType = "anime",
}: {
  onBack: () => void;
  initialType?: SearchType;
}) {
  const [query, setQuery] = useState("");
  const [searchType, setSearchType] = useState<SearchType>(initialType);
  const [results, setResults] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [added, setAdded] = useState(new Set<string>());
  const [error, setError] = useState("");

  const currentType = SEARCH_TYPES.find((t) => t.value === searchType);

  async function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    if (!query.trim()) return;
    setLoading(true);
    setError("");
    try {
      const data = await searchMedia(query, searchType);
      setResults(data);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  async function handleAdd(item: any) {
      const id = String(
        item.anilist_id ?? item.tmdb_id ?? item.rawg_id ?? item.google_id
      );
      const externalId = Number(
        item.anilist_id ?? item.tmdb_id ?? item.rawg_id ?? item.google_id
      );
      const type = (item.media_type ?? searchType) as SearchType;
      setAdding(id);
      try {
        await addToTracking(externalId, type, "planned");
        setAdded((prev) => new Set(prev).add(id));
      } catch (e) {
        const msg = (e as Error).message;
        if (msg.includes("трекинге")) {
          setAdded((prev) => new Set(prev).add(id));
        } else {
          setError(msg);
        }
      } finally {
        setAdding(null);
      }
    }

  return (
    <div className="min-h-screen bg-background text-foreground flex flex-col">
      {/* Шапка */}
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

        {/* Категории */}
        <div className="flex flex-wrap gap-2">
          {SEARCH_TYPES.map((t) => (
            <button
              key={t.value}
              onClick={() => {
                setSearchType(t.value as SearchType);
                setResults([]);
                setQuery("");
              }}
              className={cn(
                "px-4 py-2 rounded-xl text-sm font-medium transition-all border",
                searchType === t.value
                  ? "bg-foreground text-background border-foreground"
                  : "bg-card text-muted-foreground border-border hover:border-muted-foreground hover:text-foreground"
              )}
            >
              {t.label}
            </button>
          ))}
        </div>

        {/* Строка поиска */}
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

        {error && (
          <p className="text-destructive text-sm">{error}</p>
        )}

        {/* Результаты */}
        {results.length > 0 && (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-4">
            {results.map((item) => {
              const id = String(
                item.anilist_id ?? item.tmdb_id ?? item.rawg_id ?? item.google_id
              );
              const title =
                item.title_russian ||
                item.title_english ||
                item.title_romaji ||
                item.title;

              return (
                <div
                  key={id}
                  className="group flex flex-col rounded-xl overflow-hidden bg-card border border-border hover:border-muted-foreground transition-all"
                >
                  {item.poster_url ? (
                    <img
                      src={item.poster_url}
                      alt={title}
                      referrerPolicy="no-referrer"
                      className="w-full aspect-[2/3] object-cover"
                    />
                  ) : (
                    <div className="w-full aspect-[2/3] bg-muted flex items-center justify-center text-muted-foreground text-xs text-center p-2">
                      Нет обложки
                    </div>
                  )}
                  <div className="p-3 flex flex-col gap-2 flex-1">
                    <p className="text-sm font-medium truncate text-foreground">
                      {title}
                    </p>
                    {item.episodes && (
                      <p className="text-xs text-muted-foreground">
                        {item.episodes} эп.
                      </p>
                    )}
                    <Button
                      size="sm"
                      disabled={adding === id || added.has(id)}
                      onClick={() => handleAdd(item)}
                      className={cn(
                        "mt-auto w-full rounded-lg text-xs h-8",
                        added.has(id) && "opacity-60"
                      )}
                      variant={added.has(id) ? "outline" : "default"}
                    >
                      {added.has(id)
                        ? "✓ Добавлено"
                        : adding === id
                        ? "..."
                        : "+ В список"}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* Пустое состояние до поиска */}
        {results.length === 0 && !loading && !error && (
          <div className="flex-1 flex flex-col items-center justify-center py-20 text-center">
            <div className="text-6xl mb-4 opacity-20">🔍</div>
            <p className="text-muted-foreground">
              Введи название и нажми «Найти»
            </p>
          </div>
        )}
      </div>
    </div>
  );
}