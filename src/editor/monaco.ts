// Monaco setup. This module is loaded lazily (dynamic import) the first time a file is
// opened, so the ~4 MB editor bundle never slows down app startup.
import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import JsonWorker from "monaco-editor/language/json/json.worker?worker";
import CssWorker from "monaco-editor/language/css/css.worker?worker";
import HtmlWorker from "monaco-editor/language/html/html.worker?worker";
import TsWorker from "monaco-editor/language/typescript/ts.worker?worker";

self.MonacoEnvironment = {
  getWorker(_id: string, label: string) {
    switch (label) {
      case "json":
        return new JsonWorker();
      case "css":
      case "scss":
      case "less":
        return new CssWorker();
      case "html":
      case "handlebars":
      case "razor":
        return new HtmlWorker();
      case "typescript":
      case "javascript":
        return new TsWorker();
      default:
        return new EditorWorker();
    }
  },
};

// Files are edited in isolation (no project-wide type info), so semantic errors such as
// "cannot find module" would only be noise. Keep syntax checking.
for (const defaults of [monaco.typescript.typescriptDefaults, monaco.typescript.javascriptDefaults]) {
  defaults.setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: false });
  defaults.setCompilerOptions({
    target: monaco.typescript.ScriptTarget.ESNext,
    module: monaco.typescript.ModuleKind.ESNext,
    moduleResolution: monaco.typescript.ModuleResolutionKind.NodeJs,
    jsx: monaco.typescript.JsxEmit.ReactJSX,
    allowJs: true,
    allowNonTsExtensions: true,
    esModuleInterop: true,
  });
}

monaco.editor.defineTheme("pilunch-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#14161b",
    "editor.lineHighlightBackground": "#1b1e25",
    "editorLineNumber.foreground": "#4a5060",
    "editorLineNumber.activeForeground": "#a9b1c3",
    "editorGutter.background": "#14161b",
    "editorWidget.background": "#1a1d24",
    "editorIndentGuide.background1": "#23262f",
    "editor.selectionBackground": "#3b3f6b",
    "minimap.background": "#14161b",
    "scrollbarSlider.background": "#ffffff14",
    "scrollbarSlider.hoverBackground": "#ffffff22",
  },
});

monaco.editor.defineTheme("pilunch-light", {
  base: "vs",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#ffffff",
    "editor.lineHighlightBackground": "#f4f5f8",
    "editorLineNumber.foreground": "#a6abb8",
    "editorGutter.background": "#ffffff",
    "minimap.background": "#ffffff",
  },
});

const extToLang = new Map<string, string>();
const nameToLang = new Map<string, string>();
for (const lang of monaco.languages.getLanguages()) {
  for (const ext of lang.extensions ?? []) extToLang.set(ext.toLowerCase(), lang.id);
  for (const name of lang.filenames ?? []) nameToLang.set(name.toLowerCase(), lang.id);
}
// Common extras Monaco doesn't map by default.
for (const [ext, id] of [
  [".rs", "rust"],
  [".toml", "ini"],
  [".mjs", "javascript"],
  [".cjs", "javascript"],
  [".mts", "typescript"],
  [".cts", "typescript"],
  [".svelte", "html"],
  [".vue", "html"],
  [".zsh", "shell"],
  [".bash", "shell"],
  [".lock", "plaintext"],
] as const) {
  if (!extToLang.has(ext)) extToLang.set(ext, id);
}

export function languageFor(path: string): string {
  const name = path.split("/").pop()!.toLowerCase();
  const byName = nameToLang.get(name);
  if (byName) return byName;
  if (name === "dockerfile" || name.startsWith("dockerfile.")) return "dockerfile";
  if (name === "makefile") return "shell";
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? (extToLang.get(name.slice(dot)) ?? "plaintext") : "plaintext";
}

export function monacoTheme(): string {
  return document.documentElement.dataset.theme === "light" ? "pilunch-light" : "pilunch-dark";
}

export { monaco };
