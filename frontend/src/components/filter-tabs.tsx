"use client";

import { cn } from "@/lib/utils";
import { type MediaStatus } from "@/lib/media-types";

interface FilterTabsProps {
  activeFilter: MediaStatus | "all";
  onFilterChange: (filter: MediaStatus | "all") => void;
  counts: Record<MediaStatus | "all", number>;
}

// Убрали On Hold, перевели на русский
const filters: { value: MediaStatus | "all"; label: string }[] =[
  { value: "all", label: "Все" },
  { value: "watching", label: "Смотрю" },
  { value: "completed", label: "Просмотрено" },
  { value: "dropped", label: "Дропнул" },
  { value: "plan-to-watch", label: "Запланировано" },
];

export function FilterTabs({
  activeFilter,
  onFilterChange,
  counts,
}: FilterTabsProps) {
  return (
    <div className="flex gap-1 overflow-x-auto pb-1">
      {filters.map((filter) => {
        return (
          <button
            key={filter.value}
            onClick={() => onFilterChange(filter.value)}
            className={cn(
              "shrink-0 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
              activeFilter === filter.value
                ? "bg-secondary text-foreground"
                : "text-muted-foreground hover:bg-secondary/50 hover:text-foreground"
            )}
          >
            {filter.label}
            <span className="ml-1.5 text-xs opacity-60">
              {counts[filter.value] || 0}
            </span>
          </button>
        );
      })}
    </div>
  );
}