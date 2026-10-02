import { useRef, useState } from "react";

/** Drag handle. `onDrag` receives the total pointer delta since the drag started. */
export function Splitter({ dir, onStart, onDrag }: { dir: "v" | "h"; onStart?: () => void; onDrag: (delta: number) => void }) {
  const start = useRef(0);
  const [dragging, setDragging] = useState(false);
  return (
    <div
      className={`splitter ${dir}${dragging ? " dragging" : ""}`}
      onPointerDown={(e) => {
        e.preventDefault();
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        start.current = dir === "v" ? e.clientX : e.clientY;
        onStart?.();
        setDragging(true);
        document.body.style.cursor = dir === "v" ? "col-resize" : "row-resize";
      }}
      onPointerMove={(e) => {
        if (!dragging) return;
        onDrag((dir === "v" ? e.clientX : e.clientY) - start.current);
      }}
      onPointerUp={() => {
        setDragging(false);
        document.body.style.cursor = "";
      }}
    />
  );
}
