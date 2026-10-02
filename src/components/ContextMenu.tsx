import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { create } from "zustand";
import { Icon, type IconName } from "./Icon";

export type MenuItem = { label: string; icon?: IconName; onClick: () => void; danger?: boolean } | "separator";

interface MenuState {
  menu: { x: number; y: number; items: MenuItem[] } | null;
  show(e: { clientX: number; clientY: number }, items: MenuItem[]): void;
  hide(): void;
}

export const useMenu = create<MenuState>((set) => ({
  menu: null,
  show: (e, items) => set({ menu: { x: e.clientX, y: e.clientY, items } }),
  hide: () => set({ menu: null }),
}));

export function ContextMenuHost() {
  const menu = useMenu((s) => s.menu);
  const hide = useMenu((s) => s.hide);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: 0, y: 0 });

  useLayoutEffect(() => {
    if (!menu || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    setPos({ x: Math.min(menu.x, window.innerWidth - r.width - 6), y: Math.min(menu.y, window.innerHeight - r.height - 6) });
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const close = () => hide();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && hide();
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu, hide]);

  if (!menu) return null;
  return (
    <div className="context-menu" ref={ref} style={{ left: pos.x, top: pos.y }} onMouseDown={(e) => e.stopPropagation()}>
      {menu.items.map((it, i) =>
        it === "separator" ? (
          <hr key={i} />
        ) : (
          <button
            key={i}
            className={it.danger ? "danger" : undefined}
            onClick={() => {
              hide();
              it.onClick();
            }}
          >
            {it.icon && <Icon name={it.icon} size={14} />}
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}
