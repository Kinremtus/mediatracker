import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { searchMedia, addToTracking } from "@/api/tracking";

type SearchType = "anime" | "movies" | "tv";

const SEARCH_TYPES: { value: SearchType; label: string }[] = [
  { value: "anime", label: "Аниме" },
  { value: "movies", label: "Фильмы" },
  { value: "tv", label: "Сериалы" },
];

export default function SearchPage({ onBack }: { onBack: () => void }) {
  const [query, setQuery] = useState("");
  const [searchType, setSearchType] = useState<SearchType>("anime");
  const [results, setResults] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState<number | null>(null);
  const [added, setAdded] = useState(new Set<number>());
  const [error, setError] = useState("");

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
    const id = item.anilist_id ?? item.tmdb_id;
    const type = item.media_type ?? searchType;
    setAdding(id);
    try {
      await addToTracking(id, type, "planned");
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
    <div className="min-h-screen bg-gray-950 text-white">
      <header className="border-b border-white/10 px-6 py-4 flex items-center gap-4">
        <button
          onClick={onBack}
          className="text-white/50 hover:text-white transition-colors text-sm"
        >
          ← Назад
        </button>
        <h1 className="text-xl font-bold">Поиск</h1>
      </header>

      <main className="max-w-5xl mx-auto px-6 py-8">
        {/* Переключатель типа */}
        <div className="flex gap-2 mb-4">
          {SEARCH_TYPES.map((t) => (
            <button
              key={t.value}
              onClick={() => {
                setSearchType(t.value);
                setResults([]);
              }}
              className={`px-4 py-1.5 rounded-full text-sm transition-colors ${
                searchType === t.value
                  ? "bg-white text-gray-950 font-semibold"
                  : "bg-white/10 text-white/70 hover:bg-white/20"
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>

        {/* Поиск */}
        <form onSubmit={handleSearch} className="flex gap-3 mb-8">
          <Input
            placeholder={`Поиск ${SEARCH_TYPES.find((t) => t.value === searchType)?.label.toLowerCase()}...`}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="bg-white/5 border-white/20 text-white placeholder:text-white/40 rounded-full px-5"
          />
          <Button
            type="submit"
            disabled={loading}
            className="rounded-full px-6 shrink-0"
          >
            {loading ? "Ищем..." : "Найти"}
          </Button>
        </form>

        {error && <p className="text-red-400 mb-4">{error}</p>}

        {/* Результаты */}
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
          {results.map((item) => {
            const id = item.anilist_id ?? item.tmdb_id;
            const title =
              item.title_russian || item.title_english || item.title_romaji || item.title;
            return (
              <div
                key={id}
                className="rounded-xl overflow-hidden bg-white/5 border border-white/10 flex flex-col"
              >
                {item.poster_url && (
                  <img
                    src={item.poster_url}
                    alt={title}
                    referrerPolicy="no-referrer"
                    className="w-full aspect-[2/3] object-cover"
                  />
                )}
                <div className="p-3 flex flex-col gap-2 flex-1">
                  <p className="text-sm font-medium truncate">{title}</p>
                  {item.episodes && (
                    <p className="text-xs text-white/40">{item.episodes} эп.</p>
                  )}
                  <Button
                    size="sm"
                    disabled={adding === id || added.has(id)}
                    onClick={() => handleAdd(item)}
                    className="mt-auto w-full rounded-full text-xs"
                    variant={added.has(id) ? "outline" : "default"}
                  >
                    {added.has(id) ? "✓ Добавлено" : adding === id ? "..." : "+ В список"}
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      </main>
    </div>
  );
}