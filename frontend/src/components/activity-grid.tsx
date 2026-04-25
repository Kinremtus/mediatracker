"use client";

import React, { useMemo } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

interface ActivityGridProps {
  data?: Record<string, number>;
}

// Утилиты для дат
function getLocalDateString(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatDateRu(dateStr: string): string {
  const [year, month, day] = dateStr.split("-").map(Number);
  return `${day} ${MONTHS_SHORT[month - 1]} ${year}`;
}

// Генератор данных (генерим только до сегодняшнего дня, будущее будет пустым)
function generateActivityData(year: number): Record<string, number> {
  return {};
}

const MONTHS_SHORT =["Янв", "Фев", "Мар", "Апр", "Май", "Июн", "Июл", "Авг", "Сен", "Окт", "Ноя", "Дек"];

// ФИОЛЕТОВАЯ ПАЛИТРА
const getLevelClass = (level: number) => {
  switch (level) {
    case 0: return "bg-muted hover:bg-muted/80"; // Базовый серый цвет интерфейса для дней без активности
    case 1: return "bg-purple-900 hover:bg-purple-800";
    case 2: return "bg-purple-700 hover:bg-purple-600";
    case 3: return "bg-purple-500 hover:bg-purple-400";
    default: return "bg-purple-400 hover:bg-purple-300"; // Самая высокая активность
  }
};

export function ActivityGrid({ data }: ActivityGridProps) {
  // Фиксируем текущий год
  const currentYear = new Date().getFullYear();
  const activityData = useMemo(() => data || generateActivityData(currentYear), [data, currentYear]);
  
  // Строим ровную матрицу от 1 Января до 31 Декабря
  const weeks = useMemo(() => {
    const startOfYear = new Date(currentYear, 0, 1);
    const endOfYear = new Date(currentYear, 11, 31);
    
    // Сдвигаем начало на понедельник
    const gridStart = new Date(startOfYear);
    let startOffset = gridStart.getDay() - 1;
    if (startOffset < 0) startOffset = 6;
    gridStart.setDate(gridStart.getDate() - startOffset);
    
    // Сдвигаем конец на воскресенье
    const gridEnd = new Date(endOfYear);
    let endOffset = gridEnd.getDay() - 1;
    if (endOffset < 0) endOffset = 6;
    gridEnd.setDate(gridEnd.getDate() + (6 - endOffset));
    
    const generatedWeeks: ({ date: string; level: number } | null)[][] = [];
    let currentWeek: ({ date: string; level: number } | null)[] =[];
    
    for (let d = new Date(gridStart); d <= gridEnd; d.setDate(d.getDate() + 1)) {
      // Если день выпадает за пределы года (например, 30 декабря прошлого года)
      if (d.getFullYear() !== currentYear) {
        currentWeek.push(null);
      } else {
        const dateStr = getLocalDateString(d);
        const count = activityData[dateStr] || 0;
        currentWeek.push({ date: dateStr, level: Math.min(count, 4) });
      }
      
      if (currentWeek.length === 7) {
        generatedWeeks.push(currentWeek);
        currentWeek =[];
      }
    }
    return generatedWeeks;
  }, [activityData, currentYear]);

  // Подписи месяцев (ставим метку над неделей, где есть 1-е число)
  const monthLabels = useMemo(() => {
    return weeks.map((week) => {
      const firstOfMonth = week.find(d => d && d.date.endsWith("-01"));
      if (firstOfMonth) {
        const monthIdx = parseInt(firstOfMonth.date.split("-")[1], 10) - 1;
        return MONTHS_SHORT[monthIdx];
      }
      return null;
    });
  }, [weeks]);

  const totalContributions = useMemo(() => {
    return Object.values(activityData).reduce((sum, count) => sum + count, 0);
  }, [activityData]);

  return (
    <Card className="border-border bg-card">
      <CardHeader className="pb-2">
        <CardTitle className="text-base font-medium text-foreground">
          История активности
        </CardTitle>
        <p className="text-sm text-muted-foreground">
          {totalContributions} действий за {currentYear} год
        </p>
      </CardHeader>
      <CardContent className="px-4 pb-4 sm:px-6">
        <TooltipProvider delayDuration={100}>
          <div className="w-full">
            <div 
              className="grid w-full gap-[2px] sm:gap-[3px]" 
              style={{ gridTemplateColumns: `auto repeat(${weeks.length}, minmax(0, 1fr))` }}
            >
              {/* СТРОКА 1: Подписи месяцев */}
              <div className="h-4" /> {/* Пустой левый верхний угол */}
              {monthLabels.map((label, i) => (
                <div key={`month-${i}`} className="relative h-4 w-full">
                  {label && (
                    <span className="absolute bottom-1 left-0 z-10 whitespace-nowrap text-[9px] text-muted-foreground sm:text-[10px]">
                      {label}
                    </span>
                  )}
                </div>
              ))}

              {/* СТРОКИ 2-8: Дни недели и ячейки */}
              {[0, 1, 2, 3, 4, 5, 6].map(dayIdx => (
                <React.Fragment key={`day-row-${dayIdx}`}>
                  {/* Подписи дней слева */}
                  <div className="flex items-center justify-end pr-2 text-[9px] text-muted-foreground sm:text-[10px]">
                    {dayIdx === 0 ? "Пн" : dayIdx === 2 ? "Ср" : dayIdx === 4 ? "Пт" : dayIdx === 6 ? "Вс" : ""}
                  </div>

                  {/* Квадратики графика */}
                  {weeks.map((week, weekIdx) => {
                    const dayData = week[dayIdx];
                    
                    // Если это день из другого года (для выравнивания сетки)
                    if (!dayData) {
                      return <div key={`empty-${weekIdx}-${dayIdx}`} className="aspect-square w-full rounded-[2px]" />;
                    }

                    return (
                      <Tooltip key={`day-${weekIdx}-${dayIdx}`}>
                        <TooltipTrigger asChild>
                          <div
                            className={cn(
                              "aspect-square w-full rounded-[2px] transition-all duration-200",
                              getLevelClass(dayData.level)
                            )}
                          />
                        </TooltipTrigger>
                        <TooltipContent 
                          side="top" 
                          className="border-border bg-popover px-3 py-1.5 text-popover-foreground shadow-xl"
                        >
                          <p className="text-xs">
                            <span className="font-semibold text-foreground">
                              {activityData[dayData.date] || 0} действий
                            </span>
                            <br />
                            <span className="text-muted-foreground">
                              {formatDateRu(dayData.date)}
                            </span>
                          </p>
                        </TooltipContent>
                      </Tooltip>
                    );
                  })}
                </React.Fragment>
              ))}
            </div>

            {/* ЛЕГЕНДА С НОВЫМИ ЦВЕТАМИ */}
            <div className="mt-4 flex items-center justify-end gap-2 text-[10px] text-muted-foreground">
              <span>Меньше</span>
              <div className="flex gap-[3px]">
                <div className="h-2.5 w-2.5 rounded-[2px] bg-muted" />
                <div className="h-2.5 w-2.5 rounded-[2px] bg-purple-900" />
                <div className="h-2.5 w-2.5 rounded-[2px] bg-purple-700" />
                <div className="h-2.5 w-2.5 rounded-[2px] bg-purple-500" />
                <div className="h-2.5 w-2.5 rounded-[2px] bg-purple-400" />
              </div>
              <span>Больше</span>
            </div>
          </div>
        </TooltipProvider>
      </CardContent>
    </Card>
  );
}