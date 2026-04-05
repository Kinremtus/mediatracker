import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { getTracking } from "@/api/tracking";
import SearchPage from "./SearchPage";

export default function HomePage({ onLogout }) {
  const [tracking, setTracking] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [showSearch, setShowSearch] = useState(false);

  function loadTracking() {
    setLoading(true);
    getTracking()
      .then(setTracking)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    loadTracking();
  }, []);

  if (showSearch) {
    return (
      <SearchPage
        onBack={() => {
          setShowSearch(false);
          loadTracking(); // обновляем список после возврата
        }}
      />
    );
  }

  return (
    <div className="min-h-screen bg-gray-950 text-white">
      <header className="border-b border-white/10 px-6 py-4 flex items-center justify-between">
        <h1 className="text-xl font-bold">MediaTracker</h1>
        <div className="flex items-center gap-4">
          <Button
            onClick={() => setShowSearch(true)}
            className="rounded-full px-5 text-sm"
          >
            + Добавить
          </Button>
          <button
            onClick={onLogout}
            className="text-white/50 hover:text-white text-sm transition-colors"
          >
            Выйти
          </button>
        </div>
      </header>

      <main className="max-w-5xl mx-auto px-6 py-8">
        <h2 className="text-2xl font-bold mb-6">Мой список</h2>

        {loading && <p className="text-white/50">Загрузка...</p>}
        {error && <p className="text-red-400">{error}</p>}

        {!loading && tracking.length === 0 && (
          <div className="text-center py-20">
            <p className="text-white/30 text-lg mb-4">Список пуст</p>
            <Button
              onClick={() => setShowSearch(true)}
              className="rounded-full px-6"
            >
              Найти аниме
            </Button>
          </div>
        )}

        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
          {tracking.map((entry) => (
            <div
              key={entry.id}
              className="rounded-xl overflow-hidden bg-white/5 border border-white/10 hover:border-white/30 transition-colors"
            >
              {entry.media.poster_url && (
                <img
                  src={entry.media.poster_url}
                  alt={entry.media.title}
                  className="w-full aspect-[2/3] object-cover"
                />
              )}
              <div className="p-3">
                <p className="text-sm font-medium truncate">
                  {entry.media.title_russian || entry.media.title}
                </p>
                <p className="text-xs text-white/50 mt-1">{entry.status}</p>
              </div>
            </div>
          ))}
        </div>
      </main>
    </div>
  );
}