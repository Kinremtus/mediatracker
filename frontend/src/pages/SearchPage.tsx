import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { searchAnime, addToTracking } from "@/api/tracking";

export default function SearchPage({ onBack }) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState(null); // id аниме которое добавляем
  const [added, setAdded] = useState(new Set()); // уже добавленные
  const [error, setError] = useState("");

  async function handleSearch(e) {
    e.preventDefault();
    if (!query.trim()) return;

    setLoading(true);
    setError("");
    try {
      const data = await searchAnime(query);
      setResults(data);
    } catch (e) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleAdd(anime) {
    setAdding(anime.anilist_id);
    try {
      await addToTracking(anime.anilist_id, "planned");
      setAdded((prev) => new Set(prev).add(anime.anilist_id));
    } catch (e) {
      // "Уже в трекинге" — не ошибка, просто помечаем
      if (e.message.includes("трекинге")) {
        setAdded((prev) => new Set(prev).add(anime.anilist_id));
      } else {
        setError(e.message);
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
        <h1 className="text-xl font-bold">Поиск аниме</h1>
      </header>

      <main className="max-w-5xl mx-auto px-6 py-8">
        <form onSubmit={handleSearch} className="flex gap-3 mb-8">
          <Input
            placeholder="Введи название аниме..."
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

        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
          {results.map((anime) => (
            <div
              key={anime.anilist_id}
              className="rounded-xl overflow-hidden bg-white/5 border border-white/10 flex flex-col"
            >
              {anime.poster_url && (
                <img
                  src={anime.poster_url}
                  alt={anime.title_romaji}
                  className="w-full aspect-[2/3] object-cover"
                />
              )}
              <div className="p-3 flex flex-col gap-2 flex-1">
                <p className="text-sm font-medium truncate">
                  {anime.title_russian || anime.title_english || anime.title_romaji}
                </p>
                {anime.title_russian && (
                  <p className="text-xs text-white/40 truncate">
                    {anime.title_romaji}
                  </p>
                )}
                <p className="text-xs text-white/40">
                  {anime.episodes ? `${anime.episodes} эп.` : "?"}{" "}
                  {anime.status === "FINISHED" ? "· Завершён" : "· Онгоинг"}
                </p>
                <Button
                  size="sm"
                  disabled={adding === anime.anilist_id || added.has(anime.anilist_id)}
                  onClick={() => handleAdd(anime)}
                  className="mt-auto w-full rounded-full text-xs"
                  variant={added.has(anime.anilist_id) ? "outline" : "default"}
                >
                  {added.has(anime.anilist_id)
                    ? "✓ Добавлено"
                    : adding === anime.anilist_id
                    ? "..."
                    : "+ В список"}
                </Button>
              </div>
            </div>
          ))}
        </div>
      </main>
    </div>
  );
}