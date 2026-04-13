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

// Импорты твоего API
import { getTracking, type TrackingEntry } from "@/api/tracking";
import SearchPage from "./SearchPage";

// === АДАПТЕР: Переводчик с языка Бэкенда на язык UI ===
// Бэкенд шлет "in_progress", UI ожидает "watching" (для статистики и табов)

// === ГЛАВНЫЙ КОНТРОЛЛЕР СТРАНИЦЫ ===
export default function HomePage({ onLogout }: { onLogout: () => void }) {
  // Состояния из v0 (UI)
  const [searchQuery, setSearchQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<MediaStatus | "all">("all");
  const [activeCategory, setActiveCategory] = useState<MediaType | "all">("all");
  const[activeView, setActiveView] = useState<SidebarView>("media");

  // Состояния твоего API
  const [tracking, setTracking] = useState<TrackingEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [showSearch, setShowSearch] = useState(false);
  const [searchInitialType, setSearchInitialType] = useState<SearchType>("anime");
  
  // Загрузка данных
  const loadTracking = (status?: string | null) => {
    setLoading(true);
    getTracking(status ?? undefined)
      .then(setTracking)
      .catch(console.error)
      .finally(() => setLoading(false));
  };  

  useEffect(() => {
    loadTracking();
  },[]);

  // Обновление статуса без перезагрузки
  const handleStatusUpdate = (id: number, status: string) => {
    setTracking((prev) =>
      prev.map((e) => (e.id === id ? { ...e, status } : e))
    );
  };

  // Удаление из стейта без перезагрузки
  const handleDelete = (id: number) => {
    setTracking((prev) => prev.filter((e) => e.id !== id));
  };

  const statusCounts = useMemo(() => {
    const counts: Record<MediaStatus | "all", number> = {
      all: tracking.length,
      watching: 0,
      completed: 0,
      dropped: 0,
      "plan-to-watch": 0,
    };

    tracking.forEach((entry) => {
      const uiStatus = mapStatusToUI(entry.status);
      if (counts.hasOwnProperty(uiStatus)) {
         counts[uiStatus]++;
      }
    });

    return counts;
  },[tracking]);

  // Заглушка для категорий (пока бэк не умеет отдавать тип медиа)
  const categoryCounts = useMemo(() => {
  const counts: Record<string, number> = {
    all: tracking.length,
    anime: 0,
    movies: 0,
    "tv-shows": 0,
    books: 0,
    manga: 0,
    manhwa: 0,
    manhua: 0,
    games: 0,
    dramas: 0,
    cartoons: 0,
    "animated-movies": 0,
    novels: 0,
  };
  tracking.forEach((entry) => {
    const type = entry.media.media_type;
    if (type in counts) counts[type]++;
  });
  return counts;
}, [tracking]);

  // Группируем то, что смотрим прямо сейчас, по категориям
  const inProgressByCategory = useMemo(() => {
    const grouped: Partial<Record<MediaType, TrackingEntry[]>> = {};

    tracking.forEach((entry) => {
      // Берем только активные (Смотрю/Читаю)
      if (entry.status === "in_progress") {
        // Если твой бэк пока не умеет отдавать тип медиа, фолбечим в anime
        const type = (entry.media.media_type as MediaType) || "anime";
        
        if (!grouped[type]) {
          grouped[type] = [];
        }
        grouped[type]!.push(entry);
      }
    });

    return grouped;
  }, [tracking]);

  // Фильтрация списка на фронте
  const filteredMedia = useMemo(() => {
    return tracking
      .filter((entry) => {
        const matchesSearch = (entry.media.title_russian || entry.media.title)
          .toLowerCase()
          .includes(searchQuery.toLowerCase());
        const uiStatus = mapStatusToUI(entry.status);
        const matchesFilter = activeFilter === "all" || uiStatus === activeFilter;
        const matchesCategory =
          !activeCategory ||
          activeCategory === "all" ||
          entry.media.media_type === activeCategory;
        return matchesSearch && matchesFilter && matchesCategory;
      })
      .sort((a, b) => {
        const titleA = (a.media.title_russian || a.media.title).toLowerCase();
        const titleB = (b.media.title_russian || b.media.title).toLowerCase();
        return titleA.localeCompare(titleB, "ru");
      });
  }, [tracking, searchQuery, activeFilter, activeCategory]);

  const CATEGORY_LABELS: Record<string, string> = {
  anime: "аниме",
  manga: "мангу",
  manhwa: "манхву",
  manhua: "маньхуа",
  novels: "новеллу",
  movies: "фильм",
  tv: "сериал",
  dramas: "дораму",
  cartoons: "мультсериал",
  "animated-movies": "мультфильм",
  games: "игру",
  books: "книгу",
};
  // Роутинг: Если открыт поиск - рендерим его
  if (showSearch) {
    return (
      <SearchPage
        initialType={searchInitialType}
        onBack={() => {
          setShowSearch(false);
          loadTracking(activeFilter);
        }}
      />
    );
  }

  return (
    <SidebarProvider defaultOpen={true}>
      <AppSidebar
        activeCategory={activeCategory}
        onCategoryChange={setActiveCategory}
        counts={categoryCounts}
        activeView={activeView}
        onViewChange={setActiveView}
      />
      <SidebarInset className="bg-background">
        
        {/* Хедер */}
        <DashboardHeader 
          searchQuery={searchQuery} 
          onSearchChange={setSearchQuery} 
          onAddClick={() => setShowSearch(true)}
          onLogout={onLogout}
        />

        <main className="mx-auto max-w-7xl px-4 py-6">
          
          {/* Раздел "Главная" (Активные процессы) */}
          {activeView === "overview" && (
            <section className="animate-in fade-in slide-in-from-bottom-4 duration-500">
              <h1 className="mb-6 text-xl font-semibold text-foreground">Сейчас в процессе</h1>
              
              {/* Если активных процессов нет */}
              {Object.keys(inProgressByCategory).length === 0 ? (
                <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border py-12 text-center">
                  <p className="text-muted-foreground">Вы сейчас ничего не смотрите.</p>
                  <button onClick={() => setShowSearch(true)} className="mt-2 text-sm text-primary hover:underline">
                    Найти и добавить
                  </button>
                </div>
              ) : (
                /* Рендерим категории */
                <div className="space-y-8">
                  {Object.entries(inProgressByCategory).map(([type, entries]) => {
                    const config = mediaTypeConfig[type as MediaType];
                    const Icon = config?.icon;
                    
                    return (
                      <div key={type}>
                        {/* Заголовок категории с иконкой */}
                        <h2 className="mb-4 flex items-center gap-2 text-lg font-medium text-foreground">
                          {Icon && <Icon className="size-5 text-muted-foreground" />}
                          {config?.labelRu || type}
                          <span className="text-sm text-muted-foreground">({entries.length})</span>
                        </h2>
                        
                        {/* Сетка карточек для этой категории */}
                        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
                          {entries.map((entry) => (
                            <TrackingCard
                              key={entry.id}
                              entry={entry}
                              onUpdate={handleStatusUpdate}
                              onDelete={handleDelete}
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

          {/* Раздел "Статистика" */}
          {activeView === "statistics" && (
            <section className="animate-in fade-in slide-in-from-bottom-4 duration-500">
              <h1 className="mb-6 text-xl font-semibold text-foreground">Статистика</h1>
              {/* ВОТ ЗДЕСЬ ОНО ДОЛЖНО БЫТЬ: */}
              <StatisticsSection tracking={tracking} />
            </section>
          )}

          {activeView === "media" && (
            <>
              {/* Статистика */}
              <section>
                <h2 className="mb-3 text-sm font-medium text-muted-foreground">
                  Моя статистика
                </h2>
                <StatsCards
                  watching={statusCounts.watching}
                  completed={statusCounts.completed}
                  dropped={statusCounts.dropped}
                  planToWatch={statusCounts["plan-to-watch"]}
                />
              </section>

              {/* Список тайтлов */}
              <section className="mt-8">
                <div className="mb-4 flex items-center justify-between">
                  <h2 className="text-sm font-medium text-muted-foreground">
                    Мой список
                  </h2>
                  <span className="text-xs text-muted-foreground">
                    {filteredMedia.length} тайтлов
                  </span>
                </div>

                {/* Табы фильтрации (Все, Смотрю, Завершено и тд) */}
                <FilterTabs
                  activeFilter={activeFilter}
                  onFilterChange={setActiveFilter}
                  counts={statusCounts}
                />

                {/* Сетка карточек */}
                {loading ? (
                  <p className="py-10 text-center text-muted-foreground">Загрузка...</p>
                ) : (
                  <div className="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
                    {filteredMedia.map((entry) => (
                      <TrackingCard
                        key={entry.id}
                        entry={entry}
                        onUpdate={handleStatusUpdate}
                        onDelete={handleDelete}
                      />
                    ))}
                  </div>
                )}

                {/* Если список пуст */}
                {!loading && filteredMedia.length === 0 && (
                  <div className="text-center py-20">
                    <p className="text-white/30 text-lg mb-4">
                      {activeCategory && activeCategory !== "all"
                        ? "Ничего не найдено"
                        : "Список пуст"}
                    </p>
                    {(!activeCategory || activeCategory === "all") && (
                      <button
                        onClick={() => setShowSearch(true)}
                        className="rounded-full px-6"
                      >
                        Найти и добавить что-нибудь
                      </button>
                    )}
                    {activeCategory !== "all" && (
                      <button
                        onClick={() => {
                          const searchType: SearchType =
                            activeCategory === "tv-shows" ? "tv" : (activeCategory as SearchType);
                          setSearchInitialType(searchType);
                          setShowSearch(true);
                        }}
                        className="rounded-full px-6"
                      >
                        Найти и добавить {CATEGORY_LABELS[activeCategory] ?? "что-нибудь"}
                      </button>
                    )}
                  </div>
                )}
              </section>
            </>
          )}
        </main>
      </SidebarInset>
    </SidebarProvider>
  );
}