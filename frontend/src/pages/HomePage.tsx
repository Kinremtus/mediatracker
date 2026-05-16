import { useState, useEffect, useMemo } from "react";
import { DashboardHeader } from "@/components/dashboard-header";
import { StatsCards } from "@/components/stats-cards";
import { FilterTabs } from "@/components/filter-tabs";
import { AppSidebar, type SidebarView } from "@/components/app-sidebar";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { mediaTypeConfig, type MediaStatus, type MediaType } from "@/lib/media-types";
import { StatisticsSection } from "@/components/statistics-section";
import { TrackingCard, mapStatusToUI } from "@/components/tracking-card";
import { type SearchType } from "@/api/tracking";
import { MediaDetailSheet } from "@/components/media-detail-sheet";
import { getTracking, type TrackingEntry } from "@/api/tracking";
import SearchPage from "./SearchPage";

// ─── Единый шаблон для одной категории ──────────────────────────────────────
interface CategoryViewProps {
  entries: TrackingEntry[];
  loading: boolean;
  activeFilter: MediaStatus | "all";
  onFilterChange: (f: MediaStatus | "all") => void;
  statusCounts: Record<MediaStatus | "all", number>;
  onAddClick: () => void;
  categoryLabel: string;
  onUpdate: (id: number, status: string, progress?: number) => void;
  onDelete: (id: number) => void;
  onPosterClick: (entry: TrackingEntry) => void;
}

function CategoryView({
  entries,
  loading,
  activeFilter,
  onFilterChange,
  statusCounts,
  onAddClick,
  categoryLabel,
  onUpdate,
  onDelete,
  onPosterClick,
}: CategoryViewProps) {
  return (
    <section className="mt-8">
      {/* Заголовок — всегда на месте */}
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-sm font-medium text-muted-foreground">Мой список</h2>
        <span className="text-xs text-muted-foreground">{entries.length} тайтлов</span>
      </div>

      {/* Табы — всегда на месте */}
      <FilterTabs
        activeFilter={activeFilter}
        onFilterChange={onFilterChange}
        counts={statusCounts}
      />

      {/* Контентная зона — фиксированная минимальная высота */}
      <div className="mt-4 min-h-[420px]">
        {loading ? (
          // Скелетон — выглядит как грид карточек
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="flex flex-col overflow-hidden rounded-xl border border-border bg-card animate-pulse">
                <div className="aspect-[2/3] w-full bg-muted" />
                <div className="p-3 space-y-2">
                  <div className="h-3 rounded bg-muted w-3/4" />
                  <div className="h-3 rounded bg-muted w-1/2" />
                </div>
              </div>
            ))}
          </div>
        ) : entries.length === 0 ? (
          // Пустое состояние
          <div className="flex h-[420px] flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border text-center">
            <p className="text-muted-foreground">
              {activeFilter !== "all"
                ? "Нет тайтлов с таким статусом"
                : `В категории «${categoryLabel}» пока ничего нет`}
            </p>
            <button
              onClick={onAddClick}
              className="text-sm text-primary hover:underline"
            >
              + Найти и добавить
            </button>
          </div>
        ) : (
          // Грид карточек
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
            {entries.map((entry) => (
              <TrackingCard
                key={entry.id}
                entry={entry}
                onUpdate={onUpdate}
                onDelete={onDelete}
                onPosterClick={() => onPosterClick(entry)}
              />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

// ─── Главная страница ─────────────────────────────────────────────────────────
export default function HomePage({ onLogout }: { onLogout: () => void }) {
  const [searchQuery, setSearchQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<MediaStatus | "all">("all");
  const [activeCategory, setActiveCategory] = useState<MediaType | "all">("all");
  const [activeView, setActiveView] = useState<SidebarView>("media");
  const [tracking, setTracking] = useState<TrackingEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [showSearch, setShowSearch] = useState(false);
  const [searchInitialType, setSearchInitialType] = useState<SearchType>("anime");
  const [selectedEntry, setSelectedEntry] = useState<TrackingEntry | null>(null);

  const loadTracking = (status?: string | null) => {
    setLoading(true);
    getTracking(status ?? undefined)
      .then(setTracking)
      .catch(console.error)
      .finally(() => setLoading(false));
  };

  useEffect(() => { loadTracking(); }, []);

  const handleStatusUpdate = (id: number, status: string, progress?: number) => {
    setTracking((prev) =>
      prev.map((e) => e.id === id ? { ...e, status, ...(progress !== undefined ? { progress } : {}) } : e)
    );
    setSelectedEntry((prev) =>
      prev && prev.id === id ? { ...prev, status, ...(progress !== undefined ? { progress } : {}) } : prev
    );
  };

  const handleDelete = (id: number) => {
    setTracking((prev) => prev.filter((e) => e.id !== id));
  };

  // Подсчёт статусов для табов (с учётом текущей категории)
  const statusCounts = useMemo(() => {
    const base = activeCategory === "all"
      ? tracking
      : tracking.filter((e) => e.media.media_type === activeCategory);

    const counts: Record<MediaStatus | "all", number> = {
      all: base.length, watching: 0, completed: 0, dropped: 0, "plan-to-watch": 0,
    };
    base.forEach((entry) => {
      const s = mapStatusToUI(entry.status);
      if (s in counts) counts[s]++;
    });
    return counts;
  }, [tracking, activeCategory]);

  const globalStatusCounts = useMemo(() => {
    const counts: Record<MediaStatus | "all", number> = {
      all: tracking.length, watching: 0, completed: 0, dropped: 0, "plan-to-watch": 0,
    };
    tracking.forEach((e) => { const s = mapStatusToUI(e.status); if (s in counts) counts[s]++; });
    return counts;
  }, [tracking]);

  const categoryCounts = useMemo(() => {
    const counts: Record<string, number> = {
      all: tracking.length,
      anime: 0, movies: 0, tv: 0, books: 0, manga: 0, manhwa: 0,
      manhua: 0, "other-comics": 0, games: 0, dramas: 0,
      cartoons: 0, "animated-movies": 0, novels: 0,
    };
    tracking.forEach((e) => { if (e.media.media_type in counts) counts[e.media.media_type]++; });
    return counts;
  }, [tracking]);

  const inProgressByCategory = useMemo(() => {
    const grouped: Partial<Record<MediaType, TrackingEntry[]>> = {};
    tracking.forEach((entry) => {
      if (entry.status === "in_progress") {
        const type = (entry.media.media_type as MediaType) || "anime";
        if (!grouped[type]) grouped[type] = [];
        grouped[type]!.push(entry);
      }
    });
    return grouped;
  }, [tracking]);

  // Фильтрация для CategoryView
  const filteredMedia = useMemo(() => {
    return tracking
      .filter((entry) => {
        const matchesSearch = (entry.media.title_russian || entry.media.title)
          .toLowerCase().includes(searchQuery.toLowerCase());
        const matchesFilter = activeFilter === "all" || mapStatusToUI(entry.status) === activeFilter;
        const matchesCategory = activeCategory === "all" || entry.media.media_type === activeCategory;
        return matchesSearch && matchesFilter && matchesCategory;
      })
      .sort((a, b) => {
        const tA = (a.media.title_russian || a.media.title).toLowerCase();
        const tB = (b.media.title_russian || b.media.title).toLowerCase();
        return tA.localeCompare(tB, "ru");
      });
  }, [tracking, searchQuery, activeFilter, activeCategory]);

  const CATEGORY_LABELS: Record<string, string> = {
    anime: "Аниме", manga: "Манга", manhwa: "Манхва", manhua: "Маньхуа",
    "other-comics": "Другие комиксы", novels: "Новеллы", movies: "Фильмы",
    tv: "Сериалы", dramas: "Дорамы", cartoons: "Мультсериалы",
    "animated-movies": "Мультфильмы", games: "Игры", books: "Книги",
  };

  if (showSearch) {
    return (
      <SearchPage
        initialType={searchInitialType}
        onBack={() => { setShowSearch(false); loadTracking(); }}
      />
    );
  }

  return (
    <SidebarProvider defaultOpen={true}>
      <AppSidebar
        onCategoryChange={(cat) => {
          setActiveCategory(cat);
          setActiveFilter("all"); // сбрасываем фильтр при смене категории
        }}
        counts={categoryCounts}
        activeView={activeView}
        onViewChange={setActiveView}
      />
      <SidebarInset className="bg-background">
        <MediaDetailSheet
          entry={selectedEntry}
          open={selectedEntry !== null}
          onClose={() => setSelectedEntry(null)}
          onUpdate={handleStatusUpdate}
        />

        <DashboardHeader
          searchQuery={searchQuery}
          onSearchChange={setSearchQuery}
          onAddClick={() => setShowSearch(true)}
          onLogout={onLogout}
        />

        <main className="mx-auto max-w-7xl px-4 py-6">

          {/* Главная — в процессе */}
          {activeView === "overview" && (
            <section className="animate-in fade-in slide-in-from-bottom-4 duration-500">
              <h1 className="mb-6 text-xl font-semibold text-foreground">Сейчас в процессе</h1>
              {Object.keys(inProgressByCategory).length === 0 ? (
                <div className="flex min-h-[300px] flex-col items-center justify-center rounded-xl border border-dashed border-border text-center">
                  <p className="text-muted-foreground">Вы сейчас ничего не смотрите.</p>
                  <button onClick={() => setShowSearch(true)} className="mt-2 text-sm text-primary hover:underline">
                    Найти и добавить
                  </button>
                </div>
              ) : (
                <div className="space-y-8">
                  {Object.entries(inProgressByCategory).map(([type, entries]) => {
                    const config = mediaTypeConfig[type as MediaType];
                    const Icon = config?.icon;
                    return (
                      <div key={type}>
                        <h2 className="mb-4 flex items-center gap-2 text-lg font-medium text-foreground">
                          {Icon && <Icon className="size-5 text-muted-foreground" />}
                          {config?.labelRu || type}
                          <span className="text-sm text-muted-foreground">({entries.length})</span>
                        </h2>
                        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
                          {entries.map((entry) => (
                            <TrackingCard
                              key={entry.id}
                              entry={entry}
                              onUpdate={handleStatusUpdate}
                              onDelete={handleDelete}
                              onPosterClick={() => setSelectedEntry(entry)}
                            />
                          ))}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          )}

          {/* Статистика */}
          {activeView === "statistics" && (
            <section className="animate-in fade-in slide-in-from-bottom-4 duration-500">
              <h1 className="mb-6 text-xl font-semibold text-foreground">Статистика</h1>
              <StatsCards
                watching={globalStatusCounts.watching}
                completed={globalStatusCounts.completed}
                dropped={globalStatusCounts.dropped}
                planToWatch={globalStatusCounts["plan-to-watch"]}
              />
              <StatisticsSection tracking={tracking} />
            </section>
          )}

          {/* Медиа — единый шаблон CategoryView */}
          {activeView === "media" && (
            <CategoryView
              entries={filteredMedia}
              loading={loading}
              activeFilter={activeFilter}
              onFilterChange={setActiveFilter}
              statusCounts={statusCounts}
              onAddClick={() => {
                if (activeCategory !== "all") {
                  setSearchInitialType(activeCategory as SearchType);
                }
                setShowSearch(true);
              }}
              categoryLabel={
                activeCategory === "all"
                  ? "Все категории"
                  : CATEGORY_LABELS[activeCategory] ?? activeCategory
              }
              onUpdate={handleStatusUpdate}
              onDelete={handleDelete}
              onPosterClick={setSelectedEntry}
            />
          )}
        </main>
      </SidebarInset>
    </SidebarProvider>
  );
}