import { cn } from "@subboost/ui/lib/utils";

const artisticNavItemBaseClassName =
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-full border border-transparent font-medium transition-all duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring disabled:pointer-events-none disabled:opacity-50";

const artisticNavItemInactiveClassName = "text-fg-60 hover:bg-ink/5 hover:text-fg";

export const artisticNavContainerClassName =
  "inline-flex items-center gap-1 rounded-full border border-seg-track-border bg-seg-track p-1 shadow-(--seg-track-shadow) backdrop-blur-sm";

export const artisticTabsListClassName = cn(artisticNavContainerClassName, "h-auto");

export const artisticTabsTriggerClassName = cn(
  "group",
  artisticNavItemBaseClassName,
  "px-3.5 py-2 text-sm",
  artisticNavItemInactiveClassName,
  "data-[state=active]:border-seg-active-border data-[state=active]:bg-seg-active data-[state=active]:text-seg-active-fg data-[state=active]:shadow-(--seg-active-shadow)"
);

export const artisticTabsIconClassName =
  "h-3.5 w-3.5 text-fg-45 transition-colors group-data-[state=active]:text-indigo-300";
