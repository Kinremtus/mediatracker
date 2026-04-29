"use client";

import { cn } from "@/lib/utils";
import { mediaTypes, mediaTypeConfig, type MediaType } from "@/lib/media-types";
import { LayoutGrid, Layers, Activity, ChevronDown, Home, Trash2 } from "lucide-react";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubItem,
  SidebarMenuSubButton,
  SidebarSeparator,
  useSidebar,
} from "@/components/ui/sidebar";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { useState } from "react";
import { deleteAccount } from "@/api/auth";
import { toast } from "sonner";

export type SidebarView = "overview" | "media" | "statistics";

interface AppSidebarProps {
  activeCategory: MediaType | "all";
  onCategoryChange: (category: MediaType | "all") => void;
  counts: Record<MediaType | "all", number>;
  activeView: SidebarView;
  onViewChange: (view: SidebarView) => void;
}

export function AppSidebar({
  activeCategory,
  onCategoryChange,
  counts,
  activeView,
  onViewChange,
}: AppSidebarProps) {
  const { state } = useSidebar();
  const isCollapsed = state === "collapsed";

  return (
    <Sidebar collapsible="icon" className="border-r border-border">
      
      {/* ИСПРАВЛЕНИЕ 1: Выравнивание полоски */}
      {/* Убрали py-4, поставили p-0 на обертку, а внутрь добавили h-14. 
          Теперь структура 1-в-1 совпадает с DashboardHeader (56px + 1px border) */}
      <SidebarHeader className="border-b border-border p-0">
        <div className="flex h-14 items-center gap-3 px-4">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-accent">
            <Layers className="size-4 text-accent-foreground" />
          </div>
          {!isCollapsed && (
            <span className="font-semibold text-foreground">MediaTrack</span>
          )}
        </div>
      </SidebarHeader>

      {/* ИСПРАВЛЕНИЕ 2: Убираем ползунок */}
      {/* Добавили overflow-x-hidden. Вертикальный скролл останется, а горизонтальный убит */}
      <SidebarContent className="overflow-x-hidden">
        
        {/* Overview Section */}
        <SidebarGroup>
          <SidebarGroupLabel>Навигация</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  onClick={() => onViewChange("overview")}
                  isActive={activeView === "overview"}
                  tooltip="Главная"
                  className={cn(
                    "transition-colors",
                    activeView === "overview" && "bg-accent/10 text-accent hover:bg-accent/20"
                  )}
                >
                  <Home className="size-4" />
                  <span>Главная</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarSeparator />

        {/* Categories Section */}
        <SidebarGroup>
          <SidebarGroupLabel>Категории</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {/* All Categories */}
              <SidebarMenuItem>
                <SidebarMenuButton
                  onClick={() => {
                    onCategoryChange("all");
                    onViewChange("media");
                  }}
                  isActive={activeView === "media" && activeCategory === "all"}
                  tooltip="Все категории"
                  className={cn(
                    "transition-colors",
                    activeView === "media" && activeCategory === "all" && "bg-accent/10 text-accent hover:bg-accent/20"
                  )}
                >
                  <LayoutGrid className="size-4" />
                  <span>Все категории</span>
                </SidebarMenuButton>
                <SidebarMenuBadge
                  className={cn(
                    activeView === "media" && activeCategory === "all" && "text-accent"
                  )}
                >
                  {counts.all}
                </SidebarMenuBadge>
              </SidebarMenuItem>

              {/* Collapsible Media Types */}
              <Collapsible defaultOpen className="group/collapsible">
                <SidebarMenuItem>
                  <CollapsibleTrigger asChild>
                    <SidebarMenuButton tooltip="Медиа">
                      <Layers className="size-4" />
                      <span>Медиа</span>
                      <ChevronDown className="ml-auto size-4 transition-transform group-data-[state=open]/collapsible:rotate-180" />
                    </SidebarMenuButton>
                  </CollapsibleTrigger>
                  <CollapsibleContent>
                    <SidebarMenuSub>
                      {mediaTypes.map((type) => {
                        const config = mediaTypeConfig[type];
                        const Icon = config.icon;
                        const count = counts[type];
                        const isActive = activeView === "media" && activeCategory === type;

                        return (
                          <SidebarMenuSubItem key={type}>
                            <SidebarMenuSubButton
                              onClick={() => {
                                onCategoryChange(type);
                                onViewChange("media");
                              }}
                              isActive={isActive}
                              className={cn(
                                "transition-colors",
                                isActive && "bg-accent/10 text-accent hover:bg-accent/20"
                              )}
                            >
                              <Icon className="size-3.5" />
                              <span className="truncate">{config.labelRu}</span>
                              <span className={cn(
                                "ml-auto text-xs tabular-nums",
                                isActive ? "text-accent" : "text-muted-foreground"
                              )}>
                                {count}
                              </span>
                            </SidebarMenuSubButton>
                          </SidebarMenuSubItem>
                        );
                      })}
                    </SidebarMenuSub>
                  </CollapsibleContent>
                </SidebarMenuItem>
              </Collapsible>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarSeparator />

        {/* Statistics Section */}
        <SidebarGroup>
          <SidebarGroupLabel>Аналитика</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  onClick={() => onViewChange("statistics")}
                  isActive={activeView === "statistics"}
                  tooltip="Статистика"
                  className={cn(
                    "transition-colors",
                    activeView === "statistics" && "bg-accent/10 text-accent hover:bg-accent/20"
                  )}
                >
                  <Activity className="size-4" />
                  <span>Статистика</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <SidebarMenuButton
                      tooltip="Удалить аккаунт"
                      className="text-destructive hover:text-destructive hover:bg-destructive/10"
                    >
                      <Trash2 className="size-4" />
                      <span>Удалить аккаунт</span>
                    </SidebarMenuButton>
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>Вы уверены?</AlertDialogTitle>
                      <AlertDialogDescription>
                        Это действие необратимо. Все ваши данные о просмотренном контенте будут безвозвратно удалены.
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>Отмена</AlertDialogCancel>
                      <AlertDialogAction
                        className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                        onClick={async (e) => {
                          e.preventDefault();
                          try {
                            await deleteAccount();
                            toast.success("Аккаунт успешно удален");
                            localStorage.removeItem("token");
                            sessionStorage.removeItem("token");
                            window.location.href = "/login";
                          } catch (error) {
                            toast.error("Ошибка при удалении аккаунта");
                          }
                        }}
                      >
                        Удалить
                      </AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
    </Sidebar>
  );
}