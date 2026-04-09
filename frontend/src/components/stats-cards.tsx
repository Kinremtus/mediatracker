import { Play, CheckCircle2, XCircle, Clock } from "lucide-react";
import { cn } from "@/lib/utils";

interface StatCardProps {
  label: string;
  value: number;
  icon: typeof Play;
  colorClass: string;
}

function StatCard({ label, value, icon: Icon, colorClass }: StatCardProps) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-card p-3 sm:p-4">
      <div className={cn("rounded-md p-2", colorClass)}>
        <Icon className="size-4" />
      </div>
      <div>
        <p className="text-xl font-semibold tabular-nums text-foreground sm:text-2xl">
          {value}
        </p>
        <p className="text-xs text-muted-foreground">{label}</p>
      </div>
    </div>
  );
}

interface StatsCardsProps {
  watching: number;
  completed: number;
  dropped: number;
  planToWatch: number;
}

export function StatsCards({
  watching,
  completed,
  dropped,
  planToWatch,
}: StatsCardsProps) {
  // ВОТ ОНИ, НАШИ ПРАВИЛЬНЫЕ ЦВЕТА И НАЗВАНИЯ
  const stats =[
    {
      label: "Смотрю",
      value: watching,
      icon: Play,
      colorClass: "bg-watching/10 text-watching",
    },
    {
      label: "Просмотрено",
      value: completed,
      icon: CheckCircle2,
      colorClass: "bg-completed/10 text-completed",
    },
    {
      label: "Дропнул",
      value: dropped,
      icon: XCircle,
      colorClass: "bg-dropped/10 text-dropped",
    },
    {
      label: "Запланировано",
      value: planToWatch,
      icon: Clock,
      colorClass: "bg-planned/10 text-planned", // Теперь Tailwind поймет этот класс!
    },
  ];

  return (
    // Сделали grid-cols-4 вместо 5, так как категорий теперь 4
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 sm:gap-3">
      {stats.map((stat) => (
        <StatCard key={stat.label} {...stat} />
      ))}
    </div>
  );
}