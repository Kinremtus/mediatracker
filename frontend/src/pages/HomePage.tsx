import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import {
  getTracking,
  updateTracking,
  deleteTracking,
  TrackingEntry,
} from "@/api/tracking";
import SearchPage from "./SearchPage";

const STATUS_OPTIONS = [
  { value: "planned", label: "Запланировано" },
  { value: "in_progress", label: "Смотрю" },
  { value: "completed", label: "Просмотрено" },
  { value: "dropped", label: "Дропнул" },
];

const STATUS_COLORS: Record<string, string> = {
  planned: "text-blue-400",
  in_progress: "text-yellow-400",
  completed: "text-green-400",
  dropped: "text-red-400",
};

const FILTER_OPTIONS = [
  { value: null, label: "Все" },
  { value: "in_progress", label: "Смотрю" },
  { value: "planned", label: "Запланировано" },
  { value: "completed", label: "Просмотрено" },
  { value: "dropped", label: "Дропнул" },
];

function TrackingCard({
  entry,
  onUpdate,
  onDelete,
}: {
  entry: TrackingEntry;
  onUpdate: (id: number, status: string) => void;
  onDelete: (id: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  async function handleStatusChange(status: string) {
    setLoading(true);
    setOpen(false);
    try {
      await updateTracking(entry.id, { status });
      onUpdate(entry.id, status);
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete() {
    setDeleting(true);
    try {
      await deleteTracking(entry.id);
      onDelete(entry.id);
    } finally {
      setDeleting(false);
    }
  }

  const currentLabel =
    STATUS_OPTIONS.find((s) => s.value === entry.status)?.label || entry.status;

  return (
    <div className="rounded-xl overflow-hidden bg-white/5 border border-white/10 hover:border-white/30 transition-colors flex flex-col group relative">
      <button
        onClick={handleDelete}
        disabled={deleting}
        className="absolute top-2 right-2 z-10 w-6 h-6 rounded-full bg-black/60 text-white/70 hover:text-red-400 hover:bg-black/80 transition-all opacity-0 group-hover:opacity-100 text-xs flex items-center justify-center"
      >
        ✕
      </button>

      {entry.media.poster_url && (
        <img
          src={entry.media.poster_url}
          alt={entry.media.title}
          className="w-full aspect-[2/3] object-cover"
        />
      )}
      <div className="p-3 flex flex-col gap-2">
        <p className="text-sm font-medium truncate">
          {entry.media.title_russian || entry.media.title}
        </p>
        <div className="relative">
          <button
            onClick={() => setOpen(!open)}
            disabled={loading}
            className={`text-xs ${STATUS_COLORS[entry.status] || "text-white/50"} hover:text-white transition-colors`}
          >
            {loading ? "..." : currentLabel + " ▾"}
          </button>
          {open && (
            <div className="absolute bottom-full left-0 mb-1 bg-gray-800 border border-white/20 rounded-lg overflow-hidden z-10 w-36">
              {STATUS_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  onClick={() => handleStatusChange(option.value)}
                  className={`w-full text-left px-3 py-2 text-xs hover:bg-white/10 transition-colors ${
                    option.value === entry.status
                      ? "text-white font-semibold"
                      : "text-white/70"
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function HomePage({ onLogout }: { onLogout: () => void }) {
  const [tracking, setTracking] = useState<TrackingEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [showSearch, setShowSearch] = useState(false);
  const [activeFilter, setActiveFilter] = useState<string | null>(null);

  function loadTracking(status?: string | null) {
    setLoading(true);
    getTracking(status ?? undefined)
      .then(setTracking)
      .catch((e: Error) => setError(e.message))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    loadTracking();
  }, []);

  function handleFilterChange(status: string | null) {
    setActiveFilter(status);
    loadTracking(status);
  }

  function handleStatusUpdate(id: number, status: string) {
    setTracking((prev) =>
      prev.map((e) => (e.id === id ? { ...e, status } : e))
    );
  }

  function handleDelete(id: number) {
    setTracking((prev) => prev.filter((e) => e.id !== id));
  }

  if (showSearch) {
    return (
      <SearchPage
        onBack={() => {
          setShowSearch(false);
          loadTracking(activeFilter);
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
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-2xl font-bold">Мой список</h2>
          {/* Счётчик */}
          <span className="text-white/40 text-sm">{tracking.length} записей</span>
        </div>

        {/* Фильтры */}
        <div className="flex gap-2 flex-wrap mb-6">
          {FILTER_OPTIONS.map((option) => (
            <button
              key={option.value ?? "all"}
              onClick={() => handleFilterChange(option.value)}
              className={`px-4 py-1.5 rounded-full text-sm transition-colors ${
                activeFilter === option.value
                  ? "bg-white text-gray-950 font-semibold"
                  : "bg-white/10 text-white/70 hover:bg-white/20"
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>

        {loading && <p className="text-white/50">Загрузка...</p>}
        {error && <p className="text-red-400">{error}</p>}

        {!loading && tracking.length === 0 && (
          <div className="text-center py-20">
            <p className="text-white/30 text-lg mb-4">
              {activeFilter ? "Ничего не найдено" : "Список пуст"}
            </p>
            {!activeFilter && (
              <Button
                onClick={() => setShowSearch(true)}
                className="rounded-full px-6"
              >
                Найти аниме
              </Button>
            )}
          </div>
        )}

        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
          {tracking.map((entry) => (
            <TrackingCard
              key={entry.id}
              entry={entry}
              onUpdate={handleStatusUpdate}
              onDelete={handleDelete}
            />
          ))}
        </div>
      </main>
    </div>
  );
}