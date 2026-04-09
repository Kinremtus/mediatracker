import { Search, Plus, User, LogOut } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SidebarTrigger } from "@/components/ui/sidebar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface DashboardHeaderProps {
  searchQuery: string;
  onSearchChange: (value: string) => void;
  onAddClick: () => void;
  onLogout: () => void;
}

export function DashboardHeader({
  searchQuery,
  onSearchChange,
  onAddClick,
  onLogout,
}: DashboardHeaderProps) {
  return (
    <header className="sticky top-0 z-40 border-b border-border bg-background/80 backdrop-blur-md">
      <div className="flex h-14 items-center justify-between px-4">
        {/* Sidebar Toggle */}
        <div className="flex items-center gap-3">
          <SidebarTrigger className="text-muted-foreground hover:text-foreground" />
        </div>

        {/* Search */}
        <div className="relative hidden max-w-sm flex-1 px-8 sm:block">
          <Search className="absolute left-10 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="text"
            placeholder="Поиск..."
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            className="h-9 border-0 bg-secondary pl-9 text-sm placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-accent"
          />
        </div>

        {/* Actions (Здесь больше нет дублей) */}
        <div className="flex items-center gap-2">
          {/* Кнопка добавить */}
          <Button 
            size="sm" 
            variant="ghost" 
            className="text-muted-foreground hover:text-foreground"
            onClick={onAddClick}
          >
            <Plus className="size-4" />
            <span className="hidden sm:inline">Добавить</span>
          </Button>
          
          {/* Аккаунт с выпадающим списком */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                size="icon"
                variant="ghost"
                className="size-8 rounded-full text-muted-foreground hover:text-foreground"
              >
                <User className="size-4" />
              </Button>
            </DropdownMenuTrigger>
            
            <DropdownMenuContent align="end" className="w-40 border-border bg-popover">
              <DropdownMenuItem 
                onClick={onLogout} 
                className="cursor-pointer text-destructive focus:bg-destructive/10 focus:text-destructive"
              >
                <LogOut className="mr-2 size-4" />
                <span>Выйти</span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </header>
  );
}