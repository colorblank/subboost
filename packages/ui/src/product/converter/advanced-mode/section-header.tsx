"use client";

import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

export function SectionHeader({
  icon: Icon,
  title,
  badge,
  isExpanded,
  onToggle,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  badge?: React.ReactNode;
  isExpanded: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-expanded={isExpanded}
      onClick={onToggle}
      className="flex min-h-11 w-full items-center gap-2.5 rounded-xl border border-transparent px-3 py-2.5 text-left transition-colors hover:border-ink/10 hover:bg-ink/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring"
    >
      {isExpanded ? (
        <ChevronDown className="h-4 w-4 text-fg-50" />
      ) : (
        <ChevronRight className="h-4 w-4 text-fg-50" />
      )}
      <Icon className="h-4 w-4 text-indigo-400" />
      <span className="text-sm font-medium text-fg">{title}</span>
      {badge}
    </button>
  );
}

